use crate::capsule::Capsule;
use crate::curve::{point_to_hash_seed, random_nonzero_scalar, CurvePoint, CurveScalar};
use crate::key_frag::KeyFrag;
use crate::keys::{UmbralPublicKey, UmbralSignature};
use crate::random_oracles::hash_to_scalar;

use generic_array::sequence::Concat;

pub struct CapsuleFragProof {
    point_e2: CurvePoint,
    point_v2: CurvePoint,
    kfrag_commitment: CurvePoint,
    kfrag_pok: CurvePoint,
    signature: CurveScalar,
    kfrag_signature: UmbralSignature,

    // TODO: (for @tux and @dnunez): originally it was a bytestring.
    // In heapless mode I'd have to make this struct, and all that depends on it
    // generic on the metadata size, and that's just too cumbersome.
    // Instead I'm hashing it to a scalar. Hope it's ok.
    metadata: CurveScalar,
}

impl CapsuleFragProof {
    fn from_kfrag_and_cfrag(
        capsule: &Capsule,
        kfrag: &KeyFrag,
        cfrag_e1: &CurvePoint,
        cfrag_v1: &CurvePoint,
        metadata: &CurveScalar,
    ) -> Self {
        let params = capsule.params;

        let rk = kfrag.key;
        let t = random_nonzero_scalar();

        // Here are the formulaic constituents shared with `verify_correctness`.

        let e = capsule.point_e;
        let v = capsule.point_v;

        let e1 = cfrag_e1;
        let v1 = cfrag_v1;

        let u = params.u;
        let u1 = kfrag.proof.commitment;

        let e2 = &e * &t;
        let v2 = &v * &t;
        let u2 = &u * &t;

        let hash_input = [e, *e1, e2, v, *v1, v2, u, u1, u2];

        // TODO: original uses ExtendedKeccak here
        let h = hash_to_scalar(&hash_input, Some(&metadata.to_bytes()));

        ////////

        let z3 = &t + &rk * &h;

        Self {
            point_e2: e2,
            point_v2: v2,
            kfrag_commitment: u1,
            kfrag_pok: u2,
            signature: z3,
            kfrag_signature: kfrag.proof.signature_for_bob(),
            metadata: *metadata,
        }
    }
}

pub struct CapsuleFrag {
    pub(crate) point_e1: CurvePoint,
    pub(crate) point_v1: CurvePoint,
    pub(crate) kfrag_id: CurveScalar,
    pub(crate) precursor: CurvePoint,
    pub(crate) proof: CapsuleFragProof,
}

impl CapsuleFrag {
    pub fn from_kfrag(capsule: &Capsule, kfrag: &KeyFrag, metadata: Option<&[u8]>) -> Self {
        let rk = kfrag.key;
        let e1 = &capsule.point_e * &rk;
        let v1 = &capsule.point_v * &rk;
        let metadata_scalar = match metadata {
            Some(s) => hash_to_scalar(&[], Some(s)),
            None => CurveScalar::default(),
        };
        let proof =
            CapsuleFragProof::from_kfrag_and_cfrag(&capsule, &kfrag, &e1, &v1, &metadata_scalar);

        Self {
            point_e1: e1,
            point_v1: v1,
            kfrag_id: kfrag.id,
            precursor: kfrag.precursor,
            proof,
        }
    }

    pub(crate) fn verify(
        &self,
        capsule: &Capsule,
        delegating_pubkey: &UmbralPublicKey,
        receiving_pubkey: &UmbralPublicKey,
        signing_pubkey: &UmbralPublicKey,
    ) -> bool {
        let params = capsule.params;

        // TODO: Here are the formulaic constituents shared with `prove_correctness`.

        let e = capsule.point_e;
        let v = capsule.point_v;

        let e1 = self.point_e1;
        let v1 = self.point_v1;

        let u = params.u;
        let u1 = self.proof.kfrag_commitment;

        let e2 = self.proof.point_e2;
        let v2 = self.proof.point_v2;
        let u2 = self.proof.kfrag_pok;

        let hash_input = [e, e1, e2, v, v1, v2, u, u1, u2];

        // TODO: original uses ExtendedKeccak here
        let h = hash_to_scalar(&hash_input, Some(&self.proof.metadata.to_bytes()));

        ///////

        let precursor = self.precursor;
        let kfrag_id = self.kfrag_id;

        let kfrag_validity_message = kfrag_id
            .to_bytes()
            .concat(delegating_pubkey.to_hash_seed())
            .concat(receiving_pubkey.to_hash_seed())
            .concat(point_to_hash_seed(&u1))
            .concat(point_to_hash_seed(&precursor));

        let valid_kfrag_signature =
            signing_pubkey.verify(&kfrag_validity_message, &self.proof.kfrag_signature);

        let z3 = self.proof.signature;
        let correct_reencryption_of_e = &e * &z3 == &e2 + &(&e1 * &h);
        let correct_reencryption_of_v = &v * &z3 == &v2 + &(&v1 * &h);
        let correct_rk_commitment = &u * &z3 == &u2 + &(&u1 * &h);

        valid_kfrag_signature
            & correct_reencryption_of_e
            & correct_reencryption_of_v
            & correct_rk_commitment
    }
}
