use chacha20poly1305::aead::{Aead, Generate, Nonce, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use secrecy::{ExposeSecret, SecretBox, SecretSlice};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
#[error("the value could not be decrypted")]
pub struct DecryptError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

/// Authenticated encryption with associated data.
pub trait AeadCipher: Send + Sync {
    fn encrypt(&self, associated_data: &[u8], plaintext: &[u8]) -> Vec<u8>;
    fn decrypt(
        &self,
        associated_data: &[u8],
        stored: &[u8],
    ) -> Result<SecretSlice<u8>, DecryptError>;
}

const FORMAT_VERSION: u8 = 1;

pub struct XChaCha20Poly1305Cipher {
    aead: XChaCha20Poly1305,
}

impl XChaCha20Poly1305Cipher {
    pub fn new(key: &SecretBox<[u8; 32]>) -> Self {
        Self {
            aead: XChaCha20Poly1305::new(key.expose_secret().into()),
        }
    }
}

impl AeadCipher for XChaCha20Poly1305Cipher {
    #[expect(
        clippy::expect_used,
        reason = "Paron encrypts passwords and keys, far below the limit of 2^38 bytes"
    )]
    fn encrypt(&self, associated_data: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let nonce = Nonce::<XChaCha20Poly1305>::generate();
        let ciphertext = self
            .aead
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &[&[FORMAT_VERSION], associated_data].concat(),
                },
            )
            .expect("plaintext exceeds ChaCha20-Poly1305's 2^38-byte limit");

        [&[FORMAT_VERSION], nonce.as_slice(), &ciphertext].concat()
    }

    fn decrypt(
        &self,
        associated_data: &[u8],
        stored: &[u8],
    ) -> Result<SecretSlice<u8>, DecryptError> {
        let nonce_len = Nonce::<XChaCha20Poly1305>::default().len();
        let (header, ciphertext) = stored.split_at_checked(1 + nonce_len).ok_or_else(|| {
            DecryptError(
                "the value is shorter than the version and the XChaCha20Poly1305 nonce".into(),
            )
        })?;
        let (version, nonce) = (header[0], &header[1..]);
        let nonce =
            Nonce::<XChaCha20Poly1305>::try_from(nonce).map_err(|e| DecryptError(Box::new(e)))?;

        let plaintext = self
            .aead
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: &[&[version], associated_data].concat(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|e| DecryptError(Box::new(e)))?;

        Ok(SecretSlice::from(plaintext.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::keys::symmetric_key;

    const KEY: [u8; 32] = [7; 32];
    const OTHER_KEY: [u8; 32] = [8; 32];
    const ASSOCIATED_DATA: &[u8] = b"machine-1";
    const OTHER_ASSOCIATED_DATA: &[u8] = b"machine-2";
    const PLAINTEXT: &[u8] = b"hunter2";

    fn cipher(key: [u8; 32]) -> XChaCha20Poly1305Cipher {
        XChaCha20Poly1305Cipher::new(&symmetric_key(key))
    }

    #[test]
    fn decrypt_encrypted_value_returns_plaintext() {
        let cipher = cipher(KEY);
        let ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);

        let plaintext = cipher.decrypt(ASSOCIATED_DATA, &ciphertext).unwrap();

        assert_eq!(plaintext.expose_secret(), PLAINTEXT);
    }

    #[test]
    fn encrypt_twice_returns_different_ciphertexts() {
        let cipher = cipher(KEY);

        let first_ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);
        let second_ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);

        assert_ne!(first_ciphertext, second_ciphertext);
    }

    #[test]
    fn decrypt_under_another_key_returns_error() {
        let ciphertext = cipher(KEY).encrypt(ASSOCIATED_DATA, PLAINTEXT);

        let decrypted = cipher(OTHER_KEY).decrypt(ASSOCIATED_DATA, &ciphertext);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn decrypt_under_another_associated_data_returns_error() {
        let cipher = cipher(KEY);
        let ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);

        let decrypted = cipher.decrypt(OTHER_ASSOCIATED_DATA, &ciphertext);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn decrypt_changed_value_returns_error() {
        let cipher = cipher(KEY);
        let mut ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);
        *ciphertext.last_mut().unwrap() ^= 1;

        let decrypted = cipher.decrypt(ASSOCIATED_DATA, &ciphertext);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn decrypt_value_shorter_than_nonce_returns_error() {
        let cipher = cipher(KEY);
        let ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);

        let decrypted = cipher.decrypt(ASSOCIATED_DATA, &ciphertext[..4]);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn decrypt_value_shorter_than_tag_returns_error() {
        let cipher = cipher(KEY);
        let ciphertext = cipher.encrypt(ASSOCIATED_DATA, b"");

        let decrypted = cipher.decrypt(ASSOCIATED_DATA, &ciphertext[..ciphertext.len() - 1]);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn encrypt_returns_value_that_starts_with_format_version() {
        let ciphertext = cipher(KEY).encrypt(ASSOCIATED_DATA, PLAINTEXT);

        assert_eq!(ciphertext[0], FORMAT_VERSION);
    }

    #[test]
    fn decrypt_value_with_unknown_format_version_returns_error() {
        let cipher = cipher(KEY);
        let mut ciphertext = cipher.encrypt(ASSOCIATED_DATA, PLAINTEXT);
        ciphertext[0] = FORMAT_VERSION + 1;

        let decrypted = cipher.decrypt(ASSOCIATED_DATA, &ciphertext);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }

    #[test]
    fn decrypt_empty_value_returns_error() {
        let decrypted = cipher(KEY).decrypt(ASSOCIATED_DATA, &[]);

        assert_matches!(decrypted, Err(DecryptError(_)));
    }
}
