use hmac::{Hmac, Mac, NewMac};
use num_bigint::BigUint;
use num_traits::One;
use sha2::Sha256;

use crate::utils::{hash160, prepend_padding, Chain};
use crate::{base58, Error, Result};

use super::curve::Point;
use super::field::FieldElement;
use super::signature::Signature;
use super::{G, N};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    pub(crate) ec_point: Point,
}

impl From<Point> for PublicKey {
    fn from(ec_point: Point) -> Self {
        Self { ec_point }
    }
}

impl PublicKey {
    pub fn new<U>(x: U, y: U) -> Result<Self>
    where
        U: Into<BigUint>,
    {
        let x = FieldElement::new(x);
        let y = FieldElement::new(y);
        let ec_point = Point::new(x, y)?;

        Ok(Self { ec_point })
    }

    pub fn from_bytes_be<B>(x: B, y: B) -> Result<Self>
    where
        B: AsRef<[u8]>,
    {
        let x = BigUint::from_bytes_be(x.as_ref());
        let y = BigUint::from_bytes_be(y.as_ref());
        Self::new(x, y)
    }

    pub fn from_bytes_le<B>(x: B, y: B) -> Result<Self>
    where
        B: AsRef<[u8]>,
    {
        let x = BigUint::from_bytes_le(x.as_ref());
        let y = BigUint::from_bytes_le(y.as_ref());
        Self::new(x, y)
    }

    pub fn valid_signature<B>(&self, digest: B, signature: &Signature) -> Result<bool>
    where
        B: AsRef<[u8]>,
    {
        signature.is_valid(digest, &self)
    }

    /// Serialize this public key using the SEC format
    pub fn serialize(&self, compressed: bool) -> Result<Vec<u8>> {
        self.ec_point.serialize(compressed)
    }

    /// Deserialize the given bytes using the SEC format
    pub fn deserialize<B>(bytes: B) -> Result<Self>
    where
        B: AsRef<[u8]>,
    {
        let ec_point = Point::deserialize(bytes)?;
        Ok(Self { ec_point })
    }

    /// Create the address
    pub fn create_address(&self, compressed: bool, testnet: bool) -> Result<String> {
        let serialized = self.serialize(compressed)?;
        let digest = hash160(serialized);
        let prefix = if testnet { 0x6f } else { 0x00 };
        let data: Vec<_> = std::iter::once(prefix).chain(digest).collect();
        Ok(base58::encode_checksum(data))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateKey {
    pub(crate) secret: BigUint,
    pub(crate) pub_key: PublicKey,
}

impl PrivateKey {
    pub fn new<U>(secret: U) -> Self
    where
        U: Into<BigUint>,
    {
        let secret = secret.into();
        let ec_point = &*G * secret.clone();
        let pub_key = PublicKey { ec_point };

        Self { secret, pub_key }
    }

    pub fn from_bytes_be<B>(secret: B) -> Self
    where
        B: AsRef<[u8]>,
    {
        let secret = BigUint::from_bytes_be(secret.as_ref());
        Self::new(secret)
    }

    pub fn from_bytes_le<B>(secret: B) -> Self
    where
        B: AsRef<[u8]>,
    {
        let secret = BigUint::from_bytes_le(secret.as_ref());
        Self::new(secret)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.pub_key
    }

    pub fn create_signature<B>(&self, digest: B) -> Result<Signature>
    where
        B: AsRef<[u8]>,
    {
        let digest = digest.as_ref();
        if digest.len() != 32 {
            return Err(Error::InvalidDigestLength(digest.len()));
        }

        let k = self.deterministic_k(digest)?;
        let r = (&*G * k.clone()).x().unwrap().0.clone();

        let k_inv = k.modpow(&(&*N - 2usize), &*N);
        let z = BigUint::from_bytes_be(digest);
        let mut s = (z + &r * &self.secret) * k_inv % &*N;
        if s > &*N / 2usize {
            s = &*N - s;
        }

        Ok(Signature::new(r, s))
    }

    fn deterministic_k<B>(&self, digest: B) -> Result<BigUint>
    where
        B: AsRef<[u8]>,
    {
        type HmacSha256 = Hmac<Sha256>;

        let digest = digest.as_ref();
        debug_assert!(digest.len() == 32);

        let mut z = BigUint::from_bytes_be(digest);
        let k = [0x00u8; 32];
        let v = [0x01u8; 32];

        if z > *N {
            z -= &*N;
        }

        let secret_bytes = prepend_padding(self.secret.to_bytes_be(), 32, 0)?;

        let hmac = HmacSha256::new_varkey(&k).unwrap();
        let k = hmac
            .chain(&v)
            .chain(&[0x00])
            .chain(&secret_bytes)
            .chain(digest)
            .finalize()
            .into_bytes();

        let hmac = HmacSha256::new_varkey(&k).unwrap();
        let v = hmac.chain(&v).finalize().into_bytes();

        let hmac = HmacSha256::new_varkey(&k).unwrap();
        let mut k = hmac
            .chain(&v)
            .chain(&[0x01])
            .chain(&secret_bytes)
            .chain(digest)
            .finalize()
            .into_bytes();

        let hmac = HmacSha256::new_varkey(&k).unwrap();
        let mut v = hmac.chain(&v).finalize().into_bytes();

        let one = BigUint::one();
        loop {
            let hmac = HmacSha256::new_varkey(&k).unwrap();
            v = hmac.chain(&v).finalize().into_bytes();

            let candidate = BigUint::from_bytes_be(&v);
            if candidate >= one && candidate < *N {
                return Ok(candidate);
            }

            let hmac = HmacSha256::new_varkey(&k).unwrap();
            k = hmac.chain(&v).chain(&[0x00]).finalize().into_bytes();
            let hmac = HmacSha256::new_varkey(&k).unwrap();
            v = hmac.chain(&v).finalize().into_bytes();
        }
    }

    pub fn create_wif(&self, compressed: bool, testnet: bool) -> Result<String> {
        let secret_bytes = prepend_padding(self.secret.to_bytes_be(), 32, 0)?;
        let prefix = if testnet { 0xef } else { 0x80 };
        let mut data: Vec<_> = std::iter::once(prefix).chain(secret_bytes).collect();
        if compressed {
            data.push(0x01)
        }

        Ok(base58::encode_checksum(data))
    }
}
