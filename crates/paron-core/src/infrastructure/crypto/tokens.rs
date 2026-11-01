use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::Zeroizing;

use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;

pub struct Hmac {
    key: SecretBox<[u8; 32]>,
}

impl Hmac {
    pub fn new(key: SecretBox<[u8; 32]>) -> Self {
        Self { key }
    }

    fn secret() -> SecretString {
        use rand::Rng as _;

        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::rng().fill_bytes(bytes.as_mut());

        let hex = Zeroizing::new(base16ct::lower::encode_string(bytes.as_ref()));

        SecretString::from(hex.as_str())
    }

    #[expect(clippy::expect_used)]
    fn digest(&self, message: &[&[u8]]) -> [u8; 32] {
        use hmac::{KeyInit as _, Mac as _};

        let mut hmac = hmac::Hmac::<sha2::Sha256>::new_from_slice(self.key.expose_secret())
            .expect("HMAC accepts a key of any length");
        for part in message {
            hmac.update(part);
        }

        hmac.finalize().into_bytes().into()
    }
}

impl Tokens for Hmac {
    fn issue(&self) -> (SecretString, TokenMac) {
        let token = Self::secret();
        let mac = TokenMac::from(self.digest(&[token.expose_secret().as_bytes()]).to_vec());

        (token, mac)
    }

    fn mac(&self, token: &SecretString) -> TokenMac {
        TokenMac::from(self.digest(&[token.expose_secret().as_bytes()]).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::keys::symmetric_key;

    const KEY: [u8; 32] = [3; 32];
    const OTHER_KEY: [u8; 32] = [4; 32];

    #[test]
    fn mac_of_the_issued_token_returns_the_issued_mac() {
        let tokens = Hmac::new(symmetric_key(KEY));
        let (token, issued_mac) = tokens.issue();

        let mac = tokens.mac(&token);

        assert_eq!(mac, issued_mac);
    }

    #[test]
    fn mac_under_another_key_returns_a_different_mac() {
        let (token, issued_mac) = Tokens::issue(&Hmac::new(symmetric_key(KEY)));

        let mac = Tokens::mac(&Hmac::new(symmetric_key(OTHER_KEY)), &token);

        assert_ne!(mac, issued_mac);
    }

    #[test]
    fn issue_twice_returns_different_secrets() {
        let tokens = Hmac::new(symmetric_key(KEY));

        let (first_secret, _) = tokens.issue();
        let (second_secret, _) = tokens.issue();

        assert_ne!(first_secret.expose_secret(), second_secret.expose_secret());
    }
}
