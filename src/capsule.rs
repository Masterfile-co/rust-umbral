use crate::capsule_frag::CapsuleFrag;
use crate::constants::{NON_INTERACTIVE, X_COORDINATE};
use crate::curve::{
    bytes_to_compressed_point, bytes_to_scalar, point_to_bytes, random_nonzero_scalar,
    scalar_to_bytes, CurveCompressedPointSize, CurvePoint, CurveScalar, CurveScalarSize,
};
use crate::curve::{Serializable, UmbralPublicKey, UmbralSecretKey};
use crate::hashing::ScalarDigest;
use crate::key_frag::KeyFrag;
use crate::params::UmbralParameters;

#[cfg(feature = "std")]
use std::vec::Vec;

use generic_array::sequence::Concat;
use generic_array::typenum::{op, Unsigned};
use generic_array::{sequence::Split, ArrayLength, GenericArray};

#[derive(Clone, Copy, Debug)]
pub struct Capsule {
    pub(crate) params: UmbralParameters,
    pub(crate) point_e: CurvePoint,
    pub(crate) point_v: CurvePoint,
    pub(crate) signature: CurveScalar,
}

type UmbralParametersSize = <UmbralParameters as Serializable>::Size;
type CapsuleSize = op!(UmbralParametersSize
    + CurveCompressedPointSize
    + CurveCompressedPointSize
    + CurveScalarSize);

impl Serializable for Capsule {
    type Size = CapsuleSize;

    fn to_bytes(&self) -> GenericArray<u8, <Self as Serializable>::Size> {
        self.params
            .to_bytes()
            .concat(point_to_bytes(&self.point_e))
            .concat(point_to_bytes(&self.point_v))
            .concat(scalar_to_bytes(&self.signature))
    }

    fn from_bytes(bytes: impl AsRef<[u8]>) -> Option<Self> {
        // TODO: can fail here; return None in this case
        let sized_bytes = GenericArray::<u8, CapsuleSize>::from_slice(bytes.as_ref());

        let (params_bytes, rest): (
            &GenericArray<u8, UmbralParametersSize>,
            &GenericArray<u8, _>,
        ) = sized_bytes.split();
        let (e_bytes, rest): (
            &GenericArray<u8, CurveCompressedPointSize>,
            &GenericArray<u8, _>,
        ) = rest.split();
        let (v_bytes, signature): (
            &GenericArray<u8, CurveCompressedPointSize>,
            &GenericArray<u8, _>,
        ) = rest.split();

        // TODO: propagate error properly
        let params = UmbralParameters::from_bytes(&params_bytes).unwrap();
        let e = bytes_to_compressed_point(&e_bytes).unwrap();
        let v = bytes_to_compressed_point(&v_bytes).unwrap();
        let signature = bytes_to_scalar(&signature).unwrap();

        Some(Capsule::new(&params, &e, &v, &signature))
    }
}

impl Capsule {
    fn new(
        params: &UmbralParameters,
        e: &CurvePoint,
        v: &CurvePoint,
        signature: &CurveScalar,
    ) -> Self {
        Self {
            params: *params,
            point_e: *e,
            point_v: *v,
            signature: *signature,
        }
    }

    pub fn with_correctness_keys(
        &self,
        delegating: &UmbralPublicKey,
        receiving: &UmbralPublicKey,
        verifying: &UmbralPublicKey,
    ) -> PreparedCapsule {
        PreparedCapsule {
            capsule: *self,
            delegating_key: *delegating,
            receiving_key: *receiving,
            verifying_key: *verifying,
        }
    }

    pub fn verify(&self) -> bool {
        let g = CurvePoint::generator();
        let h = ScalarDigest::new()
            .chain_point(&self.point_e)
            .chain_point(&self.point_v)
            .finalize();
        &g * &self.signature == &self.point_v + &(&self.point_e * &h)
    }

    /// Generates a symmetric key and its associated KEM ciphertext
    pub fn from_pubkey(
        params: &UmbralParameters,
        alice_pubkey: &UmbralPublicKey,
    ) -> (Capsule, GenericArray<u8, CurveCompressedPointSize>) {
        let g = CurvePoint::generator();

        let priv_r = random_nonzero_scalar();
        let pub_r = &g * &priv_r;

        let priv_u = random_nonzero_scalar();
        let pub_u = &g * &priv_u;

        let h = ScalarDigest::new().chain_points(&[pub_r, pub_u]).finalize();

        let s = &priv_u + (&priv_r * &h);

        let shared_key = &alice_pubkey.to_point() * &(&priv_r + &priv_u);

        let capsule = Self {
            params: *params,
            point_e: pub_r,
            point_v: pub_u,
            signature: s,
        };

        (capsule, point_to_bytes(&shared_key))
    }

    /// Derive the same symmetric key
    pub fn open_original(
        &self,
        private_key: &UmbralSecretKey,
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        let shared_key = (&self.point_e + &self.point_v) * private_key.secret_scalar();
        point_to_bytes(&shared_key)
    }

    fn open_reencrypted_generic<LC: LambdaCoeff>(
        &self,
        receiving_privkey: &UmbralSecretKey,
        delegating_key: &UmbralPublicKey,
        cfrags: &[CapsuleFrag],
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        let pub_key = UmbralPublicKey::from_secret_key(receiving_privkey).to_point();

        let precursor = cfrags[0].precursor;
        let dh_point = &precursor * receiving_privkey.secret_scalar();

        // Combination of CFrags via Shamir's Secret Sharing reconstruction
        let lc = LC::new(cfrags, &[precursor, pub_key, dh_point]);

        let mut e_prime = CurvePoint::identity();
        let mut v_prime = CurvePoint::identity();
        for (i, cfrag) in (&cfrags).iter().enumerate() {
            assert!(precursor == cfrag.precursor);
            let lambda_i = lc.lambda_coeff(i);
            e_prime += &cfrag.point_e1 * &lambda_i;
            v_prime += &cfrag.point_v1 * &lambda_i;
        }

        // Secret value 'd' allows to make Umbral non-interactive
        let d = ScalarDigest::new()
            .chain_points(&[precursor, pub_key, dh_point])
            .chain_bytes(NON_INTERACTIVE)
            .finalize();

        let e = self.point_e;
        let v = self.point_v;
        let s = self.signature;
        let h = ScalarDigest::new().chain_points(&[e, v]).finalize();

        let orig_pub_key = delegating_key.to_point();

        assert!(&orig_pub_key * &(&s * &d.invert().unwrap()) == &(&e_prime * &h) + &v_prime);
        //    raise GenericUmbralError()

        let shared_key = (&e_prime + &v_prime) * &d;
        point_to_bytes(&shared_key)
    }

    /// Derive the same symmetric encapsulated_key
    #[cfg(feature = "std")]
    pub fn open_reencrypted(
        &self,
        receiving_privkey: &UmbralSecretKey,
        delegating_key: &UmbralPublicKey,
        cfrags: &[CapsuleFrag],
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        self.open_reencrypted_generic::<LambdaCoeffHeap>(receiving_privkey, delegating_key, cfrags)
    }

    /// Derive the same symmetric encapsulated_key
    pub fn open_reencrypted_heapless<Threshold: ArrayLength<CurveScalar> + Unsigned>(
        &self,
        receiving_privkey: &UmbralSecretKey,
        delegating_key: &UmbralPublicKey,
        cfrags: &[CapsuleFrag],
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        self.open_reencrypted_generic::<LambdaCoeffHeapless<Threshold>>(
            receiving_privkey,
            delegating_key,
            cfrags,
        )
    }
}

fn lambda_coeff(xs: &[CurveScalar], i: usize) -> CurveScalar {
    let mut res = CurveScalar::one();
    for j in 0..xs.len() {
        if j != i {
            res = &res * &xs[j] * &(&xs[j] - &xs[i]).invert().unwrap();
        }
    }
    res
}

trait LambdaCoeff {
    fn new(cfrags: &[CapsuleFrag], points: &[CurvePoint]) -> Self;
    fn lambda_coeff(&self, i: usize) -> CurveScalar;
}

struct LambdaCoeffHeapless<Threshold: ArrayLength<CurveScalar> + Unsigned>(
    GenericArray<CurveScalar, Threshold>,
);

impl<Threshold: ArrayLength<CurveScalar> + Unsigned> LambdaCoeff
    for LambdaCoeffHeapless<Threshold>
{
    fn new(cfrags: &[CapsuleFrag], points: &[CurvePoint]) -> Self {
        let mut result = GenericArray::<CurveScalar, Threshold>::default();
        for i in 0..<Threshold as Unsigned>::to_usize() {
            result[i] = ScalarDigest::new()
                .chain_points(points)
                .chain_bytes(X_COORDINATE)
                .chain_scalar(&cfrags[i].kfrag_id)
                .finalize();
        }
        Self(result)
    }

    fn lambda_coeff(&self, i: usize) -> CurveScalar {
        lambda_coeff(&self.0, i)
    }
}

#[cfg(feature = "std")]
struct LambdaCoeffHeap(Vec<CurveScalar>);

#[cfg(feature = "std")]
impl LambdaCoeff for LambdaCoeffHeap {
    fn new(cfrags: &[CapsuleFrag], points: &[CurvePoint]) -> Self {
        let mut result = Vec::<CurveScalar>::with_capacity(cfrags.len());
        for cfrag in cfrags {
            let coeff = ScalarDigest::new()
                .chain_points(points)
                .chain_bytes(X_COORDINATE)
                .chain_scalar(&cfrag.kfrag_id)
                .finalize();
            result.push(coeff);
        }
        Self(result)
    }

    fn lambda_coeff(&self, i: usize) -> CurveScalar {
        lambda_coeff(&self.0, i)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedCapsule {
    pub(crate) capsule: Capsule,
    pub(crate) delegating_key: UmbralPublicKey,
    pub(crate) receiving_key: UmbralPublicKey,
    pub(crate) verifying_key: UmbralPublicKey,
}

impl PreparedCapsule {
    pub fn verify_cfrag(&self, cfrag: &CapsuleFrag) -> bool {
        cfrag.verify(
            &self.capsule,
            &self.delegating_key,
            &self.receiving_key,
            &self.verifying_key,
        )
    }

    pub fn verify_kfrag(&self, kfrag: &KeyFrag) -> bool {
        kfrag.verify(
            &self.verifying_key,
            Some(&self.delegating_key),
            Some(&self.receiving_key),
        )
    }

    pub fn reencrypt(
        &self,
        kfrag: &KeyFrag,
        metadata: Option<&[u8]>,
        verify_kfrag: bool,
    ) -> Option<CapsuleFrag> {
        if verify_kfrag && !self.verify_kfrag(&kfrag) {
            return None;
        }

        Some(CapsuleFrag::from_kfrag(&self.capsule, &kfrag, metadata))
    }

    #[cfg(feature = "std")]
    pub fn open_reencrypted(
        &self,
        cfrags: &[CapsuleFrag],
        receiving_privkey: &UmbralSecretKey,
        check_proof: bool,
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        if check_proof {
            // TODO: return Result with Error set to offending cfrag indices or something
            for cfrag in cfrags {
                assert!(self.verify_cfrag(cfrag));
            }
        }

        self.capsule
            .open_reencrypted(receiving_privkey, &self.delegating_key, cfrags)
    }

    /*
    Activates the Capsule from the attached CFrags,
    opens the Capsule and returns what is inside.

    This will often be a symmetric key.
    */
    pub fn open_reencrypted_heapless<Threshold: ArrayLength<CurveScalar> + Unsigned>(
        &self,
        cfrags: &[CapsuleFrag],
        receiving_privkey: &UmbralSecretKey,
        check_proof: bool,
    ) -> GenericArray<u8, CurveCompressedPointSize> {
        if check_proof {
            // TODO: return Result with Error set to offending cfrag indices or something
            for cfrag in cfrags {
                assert!(self.verify_cfrag(cfrag));
            }
        }

        self.capsule.open_reencrypted_heapless::<Threshold>(
            receiving_privkey,
            &self.delegating_key,
            cfrags,
        )
    }
}
