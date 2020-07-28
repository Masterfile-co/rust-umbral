//use k256::Secp256k1;
use k256::AffinePoint;
use k256::CompressedPoint;
pub use k256::ProjectivePoint as CurvePoint;
use k256::PublicKey;
pub use k256::Scalar as CurveScalar;
//use generic_array::{GenericArray, ArrayLength};

use rand_core::OsRng;

pub fn random_scalar() -> CurveScalar {
    CurveScalar::generate_vartime(&mut OsRng)
}

pub fn point_to_bytes(p: &CurvePoint) -> Vec<u8> {
    let cp = CompressedPoint::from(p.to_affine().unwrap());
    let res: Vec<u8> = cp.into_bytes().iter().cloned().collect();
    res
}

pub fn scalar_to_bytes(s: &CurveScalar) -> Vec<u8> {
    s.to_bytes().into()
}

pub fn bytes_to_point(bytes: &Vec<u8>) -> Option<CurvePoint> {
    // FIXME: Can we transform Option into CtOption directly?
    let pk = PublicKey::from_bytes(bytes).unwrap();
    let ap = AffinePoint::from_pubkey(&pk);
    // TODO: Can we transfrom CtOption into Option directly?
    if ap.is_some().into() {
        Some(CurvePoint::from(ap.unwrap()))
    } else {
        None
    }
}
