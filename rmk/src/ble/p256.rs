use embassy_crypto::Error;
use embassy_crypto::driver::{P256Ecdh, P256Point, P256Scalar};
use p256_cortex_m4::{PublicKey, SecretKey};
use zeroize::Zeroizing;

struct P256Driver;

impl P256Ecdh for P256Driver {
    fn public_key(k: &P256Scalar) -> Result<P256Point, Error> {
        let secret = Zeroizing::new(SecretKey::from_bytes(k.0.as_slice()).map_err(|_| Error::InvalidKey)?);
        let public = secret.public_key();
        Ok(P256Point {
            x: public.x(),
            y: public.y(),
        })
    }

    fn shared_secret(k: &P256Scalar, peer: &P256Point) -> Result<[u8; 32], Error> {
        let secret = Zeroizing::new(SecretKey::from_bytes(k.0.as_slice()).map_err(|_| Error::InvalidKey)?);
        let mut encoded = [0; 64];
        encoded[..32].copy_from_slice(&peer.x);
        encoded[32..].copy_from_slice(&peer.y);
        let peer = PublicKey::from_untagged_bytes(&encoded).map_err(|_| Error::InvalidKey)?;
        let shared = Zeroizing::new(secret.agree(&peer));
        Ok(*shared.as_bytes())
    }
}

embassy_crypto::p256_ecdh_impl!(P256Driver);
