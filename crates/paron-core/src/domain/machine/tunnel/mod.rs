pub mod token;

use derive_more::Deref;

use crate::domain::token_mac::TokenMac;

#[derive(Clone)]
pub enum Enrollment {
    NotEnrolled,
    Invited {
        mac: TokenMac,
        expires_at: jiff::Timestamp,
    },
    Enrolled {
        client_public_key: PublicKey,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("expected a DER-encoded SubjectPublicKeyInfo")]
pub struct InvalidPublicKey;

/// The public half of a machine's tunnel identity, in the tunnel protocol's canonical encoding
/// (a DER `SubjectPublicKeyInfo`, tunnel RFC §2.2).
#[derive(Debug, Clone, Deref, PartialEq, Eq)]
#[deref(forward)]
pub struct PublicKey(Vec<u8>);

impl PublicKey {
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Self, InvalidPublicKey> {
        let bytes = bytes.into();

        paron_tunnel_protocol::PublicKey::parse(bytes.as_slice()).map_err(|_| InvalidPublicKey)?;

        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::keys::TUNNEL_PUBLIC_KEY;

    const SPKI_LENGTH: usize = TUNNEL_PUBLIC_KEY.len();

    #[test]
    fn parse_spki_preserves_bytes() {
        let public_key = PublicKey::parse(TUNNEL_PUBLIC_KEY).unwrap();

        assert_eq!(&*public_key, TUNNEL_PUBLIC_KEY);
    }

    #[test]
    fn parse_bytes_that_are_not_spki_returns_error() {
        for (case, invalid_key_bytes) in [
            ("empty", Vec::new()),
            ("text", b"hello".to_vec()),
            ("unstructured bytes of SPKI length", vec![0xA5; SPKI_LENGTH]),
            ("truncated", TUNNEL_PUBLIC_KEY[..SPKI_LENGTH - 1].to_vec()),
            ("trailing byte", [TUNNEL_PUBLIC_KEY, &[0x00]].concat()),
        ] {
            assert_matches!(
                PublicKey::parse(invalid_key_bytes),
                Err(InvalidPublicKey),
                "{case}"
            );
        }
    }
}
