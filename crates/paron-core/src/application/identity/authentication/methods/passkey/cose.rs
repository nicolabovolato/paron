use ciborium::Value;
use ring::signature::{self, UnparsedPublicKey};

use super::cbor;

/// The COSE algorithms that a passkey may use, in the order of preference that the options give.
pub const ALGORITHMS: [i64; 3] = [EDDSA, ES256, RS256];

const ES256: i64 = -7;
const EDDSA: i64 = -8;
const RS256: i64 = -257;

const KEY_TYPE: i64 = 1;
const ALGORITHM: i64 = 3;
const KEY_TYPE_OKP: i64 = 1;
const KEY_TYPE_EC2: i64 = 2;
const KEY_TYPE_RSA: i64 = 3;
const CURVE_P256: i64 = 1;
const CURVE_ED25519: i64 = 6;
const MIN_RSA_MODULUS_BYTES: usize = 256;

/// A credential public key in COSE form (RFC 9052 and RFC 9053).
#[derive(Debug, PartialEq, Eq)]
pub enum CoseKey {
    Es256 { point: Vec<u8> },
    Ed25519 { x: Vec<u8> },
    Rs256 { n: Vec<u8>, e: Vec<u8> },
}

impl CoseKey {
    pub fn parse(encoded: &[u8]) -> Option<Self> {
        let Value::Map(entries) = cbor::read_all(encoded)? else {
            return None;
        };
        let parameter = |label: i64| {
            entries
                .iter()
                .find_map(|(key, value)| (key.as_integer() == Some(label.into())).then_some(value))
        };
        let integer =
            |label: i64| -> Option<i64> { i64::try_from(parameter(label)?.as_integer()?).ok() };
        let bytes = |label: i64| parameter(label)?.as_bytes().cloned();

        match (integer(KEY_TYPE)?, integer(ALGORITHM)?) {
            (KEY_TYPE_EC2, ES256) if integer(-1)? == CURVE_P256 => {
                let (x, y) = (bytes(-2)?, bytes(-3)?);
                (x.len() == 32 && y.len() == 32).then(|| Self::Es256 {
                    point: [&[0x04][..], &x, &y].concat(),
                })
            }
            (KEY_TYPE_OKP, EDDSA) if integer(-1)? == CURVE_ED25519 => {
                let x = bytes(-2)?;
                (x.len() == 32).then_some(Self::Ed25519 { x })
            }
            (KEY_TYPE_RSA, RS256) => {
                let (n, e) = (bytes(-1)?, bytes(-2)?);
                (n.len() >= MIN_RSA_MODULUS_BYTES).then_some(Self::Rs256 { n, e })
            }
            _ => None,
        }
    }

    pub fn algorithm(&self) -> i64 {
        match self {
            Self::Es256 { .. } => ES256,
            Self::Ed25519 { .. } => EDDSA,
            Self::Rs256 { .. } => RS256,
        }
    }

    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        match self {
            Self::Es256 { point } => {
                UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, point)
                    .verify(message, signature)
                    .is_ok()
            }
            Self::Ed25519 { x } => UnparsedPublicKey::new(&signature::ED25519, x)
                .verify(message, signature)
                .is_ok(),
            Self::Rs256 { n, e } => signature::RsaPublicKeyComponents { n, e }
                .verify(&signature::RSA_PKCS1_2048_8192_SHA256, message, signature)
                .is_ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(entries: Vec<(i64, Value)>) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(
            &Value::Map(
                entries
                    .into_iter()
                    .map(|(label, value)| (Value::from(label), value))
                    .collect(),
            ),
            &mut bytes,
        )
        .unwrap();

        bytes
    }

    #[test]
    fn parse_p256_key_returns_its_uncompressed_point() {
        let key = encoded(vec![
            (KEY_TYPE, Value::from(KEY_TYPE_EC2)),
            (ALGORITHM, Value::from(ES256)),
            (-1, Value::from(CURVE_P256)),
            (-2, Value::Bytes(vec![1; 32])),
            (-3, Value::Bytes(vec![2; 32])),
        ]);

        let parsed = CoseKey::parse(&key);

        assert_eq!(
            parsed,
            Some(CoseKey::Es256 {
                point: [vec![0x04], vec![1; 32], vec![2; 32]].concat()
            })
        );
    }

    #[test]
    fn parse_key_of_another_curve_or_algorithm_returns_not_found() {
        let p384 = encoded(vec![
            (KEY_TYPE, Value::from(KEY_TYPE_EC2)),
            (ALGORITHM, Value::from(-35)),
            (-1, Value::from(2)),
            (-2, Value::Bytes(vec![1; 48])),
            (-3, Value::Bytes(vec![2; 48])),
        ]);
        let short_rsa = encoded(vec![
            (KEY_TYPE, Value::from(KEY_TYPE_RSA)),
            (ALGORITHM, Value::from(RS256)),
            (-1, Value::Bytes(vec![1; 128])),
            (-2, Value::Bytes(vec![1, 0, 1])),
        ]);

        assert_eq!(CoseKey::parse(&p384), None);
        assert_eq!(CoseKey::parse(&short_rsa), None);
    }
}
