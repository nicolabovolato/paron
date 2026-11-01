use std::num::NonZeroU16;

use derive_more::Deref;
use secrecy::ExposeSecret;
use ssh_key::Algorithm;

use crate::domain::text::LimitedText;

#[derive(Debug, thiserror::Error)]
pub enum InvalidPublicKey {
    #[error("expected an SSH public key in binary wire format")]
    Malformed,
    #[error("the key algorithm {0} is not supported")]
    UnsupportedAlgorithm(String),
}

/// An SSH public key in **wire format** (RFC 4253 6.6)
#[derive(Clone, Deref, PartialEq, Eq)]
#[deref(forward)]
pub struct PublicKey(Vec<u8>);

impl PublicKey {
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Self, InvalidPublicKey> {
        let bytes = bytes.into();
        let key =
            ssh_key::PublicKey::from_bytes(&bytes).map_err(|_| InvalidPublicKey::Malformed)?;

        match key.algorithm() {
            Algorithm::Ed25519
            | Algorithm::Ecdsa { .. }
            | Algorithm::Rsa { .. }
            | Algorithm::SkEd25519
            | Algorithm::SkEcdsaSha2NistP256 => Ok(Self(bytes)),
            other => Err(InvalidPublicKey::UnsupportedAlgorithm(
                other.as_str().to_owned(),
            )),
        }
    }

    /// The SHA-256 fingerprint in the form that OpenSSH prints: `SHA256:` and base64.
    pub fn fingerprint(&self) -> String {
        use sha2::Digest as _;

        ssh_key::Fingerprint::Sha256(sha2::Sha256::digest(&self.0).into()).to_string()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidPrivateKey {
    #[error("expected an OpenSSH private key in binary wire format")]
    Malformed,
    #[error("the key algorithm {0} is not supported")]
    UnsupportedAlgorithm(String),
    #[error("the private key is protected by a passphrase")]
    PassphraseProtected,
}

/// Private key in the **OpenSSH binary format**
#[derive(Clone, Deref)]
pub struct PrivateKey(secrecy::SecretSlice<u8>);

impl PrivateKey {
    pub fn parse(bytes: secrecy::SecretSlice<u8>) -> Result<Self, InvalidPrivateKey> {
        let key = ssh_key::PrivateKey::from_bytes(bytes.expose_secret())
            .map_err(|_| InvalidPrivateKey::Malformed)?;

        match key.algorithm() {
            Algorithm::Ed25519 | Algorithm::Ecdsa { .. } | Algorithm::Rsa { .. } => {}
            other => {
                return Err(InvalidPrivateKey::UnsupportedAlgorithm(
                    other.as_str().to_owned(),
                ));
            }
        }

        if key.is_encrypted() {
            return Err(InvalidPrivateKey::PassphraseProtected);
        }

        Ok(Self(bytes))
    }

    /// The OpenSSH format carries the public key in its own header
    #[allow(clippy::missing_panics_doc)]
    #[expect(clippy::expect_used)]
    pub fn public_key(&self) -> PublicKey {
        PublicKey(
            ssh_key::PrivateKey::from_bytes(self.expose_secret())
                .and_then(|key| key.public_key().to_bytes())
                .expect("these bytes decoded when the key was parsed, so they must decode again"),
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "expected a connect timeout between {} and {} seconds",
    ConnectTimeout::MIN.as_secs(),
    ConnectTimeout::MAX.as_secs()
)]
pub struct InvalidConnectTimeout;

#[derive(Clone, Copy, Debug, Deref, PartialEq, Eq)]
pub struct ConnectTimeout(std::time::Duration);

impl ConnectTimeout {
    pub const MIN: std::time::Duration = std::time::Duration::from_secs(1);
    pub const MAX: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn new(timeout: std::time::Duration) -> Result<Self, InvalidConnectTimeout> {
        (Self::MIN..=Self::MAX)
            .contains(&timeout)
            .then_some(Self(timeout))
            .ok_or(InvalidConnectTimeout)
    }
}

pub type User = LimitedText<1, 64>;

#[derive(Clone)]
pub struct Access {
    pub port: NonZeroU16,
    pub user: User,
    pub authentication: Authentication,
    pub connect_timeout: ConnectTimeout,
}

#[derive(Debug, thiserror::Error)]
#[error("changing the host key requires stating the password again")]
pub struct PasswordBoundToHostKey;

#[derive(Clone)]
pub struct Authentication {
    host_key: PublicKey,
    method: AuthMethod,
}

impl Authentication {
    pub fn new(host_key: PublicKey, method: AuthMethod) -> Self {
        Self { host_key, method }
    }

    pub fn host_key(&self) -> &PublicKey {
        &self.host_key
    }

    pub fn method(&self) -> &AuthMethod {
        &self.method
    }

    #[must_use]
    pub fn with_method(self, method: AuthMethod) -> Self {
        Self { method, ..self }
    }

    pub fn with_host_key(self, host_key: PublicKey) -> Result<Self, PasswordBoundToHostKey> {
        // SSH gives the password itself to the server. Without this check, an attacker can change
        // the host key to that of a server they control, and that server gets the stored password.
        if host_key != self.host_key && matches!(self.method, AuthMethod::Password(_)) {
            return Err(PasswordBoundToHostKey);
        }

        Ok(Self { host_key, ..self })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected between {min} and {max} characters", min = Password::MIN, max = Password::MAX)]
pub struct InvalidPassword;

#[derive(Clone, Deref)]
pub struct Password(secrecy::SecretString);

impl Password {
    pub const MIN: usize = 1;
    pub const MAX: usize = 1024;

    pub fn parse(raw: secrecy::SecretString) -> Result<Self, InvalidPassword> {
        if !(Self::MIN..=Self::MAX).contains(&raw.expose_secret().chars().count()) {
            return Err(InvalidPassword);
        }

        Ok(Self(raw))
    }
}

#[derive(Clone)]
pub enum AuthMethod {
    Password(Password),
    Keypair {
        private_key: PrivateKey,
        source: KeySource,
    },
}

impl AuthMethod {
    pub fn has_same_secret(&self, other: &Self) -> bool {
        use secrecy::ExposeSecret as _;

        match (self, other) {
            (Self::Password(password), Self::Password(other_password)) => {
                password.expose_secret() == other_password.expose_secret()
            }
            (
                Self::Keypair { private_key, .. },
                Self::Keypair {
                    private_key: other_private_key,
                    ..
                },
            ) => private_key.expose_secret() == other_private_key.expose_secret(),
            _ => false,
        }
    }
}

/// `Managed` keys are ours to rotate and delete; `Imported` ones belong to the user.
#[derive(Clone, Copy)]
pub enum KeySource {
    Managed,
    Imported,
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::time::Duration;

    use secrecy::SecretSlice;

    use super::*;
    use crate::testing::keys::{self, SSH_PRIVATE_KEY, SSH_PUBLIC_KEY};
    use crate::testing::machine::{SSH_PASSWORD, password_method};

    const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";
    const ED25519_ALGORITHM: &str = "ssh-ed25519";
    const SECURITY_KEY_ED25519_ALGORITHM: &str = "sk-ssh-ed25519@openssh.com";
    const DSA_ALGORITHM: &str = "ssh-dss";

    fn ssh_string(data: &[u8]) -> Vec<u8> {
        let length = u32::try_from(data.len()).unwrap().to_be_bytes();

        length.into_iter().chain(data.iter().copied()).collect()
    }

    fn take_ssh_string<'a>(bytes: &mut &'a [u8]) -> &'a [u8] {
        let (length_prefix, rest) = bytes.split_at(size_of::<u32>());
        let length = u32::from_be_bytes(length_prefix.try_into().unwrap()) as usize;
        let (string_bytes, rest) = rest.split_at(length);
        *bytes = rest;

        string_bytes
    }

    fn public_key_blob(algorithm: &str, key: &[u8]) -> Vec<u8> {
        [ssh_string(algorithm.as_bytes()), ssh_string(key)].concat()
    }

    // Only the header changes: it names a cipher and a KDF. The key body is not encrypted.
    fn passphrase_protected_key(key: &[u8]) -> Vec<u8> {
        let mut body = &key[OPENSSH_MAGIC.len()..];
        for _ in 0..3 {
            take_ssh_string(&mut body);
        }

        let bcrypt_options = [ssh_string(&[0x00; 16]), 16u32.to_be_bytes().to_vec()].concat();

        [
            OPENSSH_MAGIC.to_vec(),
            ssh_string(b"aes256-ctr"),
            ssh_string(b"bcrypt"),
            ssh_string(&bcrypt_options),
            body.to_vec(),
        ]
        .concat()
    }

    fn secret_bytes(bytes: impl Into<Vec<u8>>) -> SecretSlice<u8> {
        SecretSlice::from(bytes.into())
    }

    fn security_key_blob() -> Vec<u8> {
        [
            public_key_blob(SECURITY_KEY_ED25519_ALGORITHM, &[0xAA; 32]),
            ssh_string(b"ssh:"),
        ]
        .concat()
    }

    // `p`, `q`, `g` and `y` are one-byte integers. The parser reads the format and checks no number.
    fn dsa_blob() -> Vec<u8> {
        [
            ssh_string(DSA_ALGORITHM.as_bytes()),
            ssh_string(&[0x07]),
            ssh_string(&[0x03]),
            ssh_string(&[0x02]),
            ssh_string(&[0x04]),
        ]
        .concat()
    }

    fn private_key_file(public_blob: &[u8], private: &[u8]) -> Vec<u8> {
        const CHECK_INT: [u8; 4] = [0xC0, 0xFF, 0xEE, 0x00];
        const BLOCK_SIZE: usize = 8;

        let mut section = [
            &CHECK_INT[..],
            &CHECK_INT,
            public_blob,
            private,
            &ssh_string(b""),
        ]
        .concat();
        let padding = (BLOCK_SIZE - section.len() % BLOCK_SIZE) % BLOCK_SIZE;
        section.extend((1..).take(padding));

        [
            OPENSSH_MAGIC.to_vec(),
            ssh_string(b"none"),
            ssh_string(b"none"),
            ssh_string(b""),
            1u32.to_be_bytes().to_vec(),
            ssh_string(public_blob),
            ssh_string(&section),
        ]
        .concat()
    }

    fn password_authentication() -> Authentication {
        Authentication::new(keys::ssh_public_key(), password_method(SSH_PASSWORD))
    }

    #[test]
    fn new_connect_timeout_outside_bounds_returns_error() {
        for invalid_timeout in [
            Duration::ZERO,
            Duration::from_millis(999),
            Duration::from_secs(61),
        ] {
            assert_matches!(
                ConnectTimeout::new(invalid_timeout),
                Err(InvalidConnectTimeout),
                "{invalid_timeout:?}"
            );
        }
    }

    #[test]
    fn new_connect_timeout_at_bounds_succeeds() {
        for valid_timeout in [ConnectTimeout::MIN, ConnectTimeout::MAX] {
            assert_matches!(
                ConnectTimeout::new(valid_timeout),
                Ok(_),
                "{valid_timeout:?}"
            );
        }
    }

    #[test]
    fn parse_public_key_preserves_bytes() {
        let public_key = PublicKey::parse(SSH_PUBLIC_KEY).unwrap();

        assert_eq!(&*public_key, SSH_PUBLIC_KEY);
    }

    #[test]
    fn parse_public_key_not_in_wire_format_returns_malformed() {
        let authorized_keys_line = ssh_key::PublicKey::from_bytes(SSH_PUBLIC_KEY)
            .unwrap()
            .to_openssh()
            .unwrap();

        for (case, invalid_key_bytes) in [
            ("empty", Vec::new()),
            (
                "wrong ed25519 length",
                public_key_blob(ED25519_ALGORITHM, &[0xAA; 31]),
            ),
            ("trailing byte", [SSH_PUBLIC_KEY, &[0x00]].concat()),
            ("authorized_keys line", authorized_keys_line.into_bytes()),
        ] {
            assert_matches!(
                PublicKey::parse(invalid_key_bytes).err(),
                Some(InvalidPublicKey::Malformed),
                "{case}"
            );
        }
    }

    #[test]
    fn parse_public_key_with_dsa_returns_unsupported_algorithm() {
        let parsed = PublicKey::parse(dsa_blob());

        assert_matches!(
            parsed.err(),
            Some(InvalidPublicKey::UnsupportedAlgorithm(algorithm)) if algorithm == DSA_ALGORITHM
        );
    }

    #[test]
    fn parse_public_key_of_a_security_key_preserves_bytes() {
        let public_key = PublicKey::parse(security_key_blob()).unwrap();

        assert_eq!(&*public_key, security_key_blob());
    }

    #[test]
    fn parse_private_key_preserves_bytes() {
        let private_key = PrivateKey::parse(secret_bytes(SSH_PRIVATE_KEY)).unwrap();

        assert_eq!(private_key.expose_secret(), SSH_PRIVATE_KEY);
    }

    #[test]
    fn parse_private_key_with_passphrase_returns_passphrase_protected() {
        let parsed = PrivateKey::parse(secret_bytes(passphrase_protected_key(SSH_PRIVATE_KEY)));

        assert_matches!(parsed.err(), Some(InvalidPrivateKey::PassphraseProtected));
    }

    #[test]
    fn parse_private_key_of_a_security_key_returns_unsupported_algorithm() {
        let security_key_private_fields =
            [vec![0x01], ssh_string(b"handle"), ssh_string(b"")].concat();
        let key_file = private_key_file(&security_key_blob(), &security_key_private_fields);

        let parsed = PrivateKey::parse(secret_bytes(key_file));

        assert_matches!(
            parsed.err(),
            Some(InvalidPrivateKey::UnsupportedAlgorithm(algorithm))
                if algorithm == SECURITY_KEY_ED25519_ALGORITHM
        );
    }

    #[test]
    fn parse_private_key_with_dsa_returns_unsupported_algorithm() {
        let key_file = private_key_file(&dsa_blob(), &ssh_string(&[0x05]));

        let parsed = PrivateKey::parse(secret_bytes(key_file));

        assert_matches!(
            parsed.err(),
            Some(InvalidPrivateKey::UnsupportedAlgorithm(algorithm)) if algorithm == DSA_ALGORITHM
        );
    }

    #[test]
    fn parse_private_key_not_in_binary_format_returns_malformed() {
        let pem_armoured = ssh_key::PrivateKey::from_bytes(SSH_PRIVATE_KEY)
            .unwrap()
            .to_openssh(ssh_key::LineEnding::LF)
            .unwrap();

        for (case, invalid_key_bytes) in [
            ("PEM armour", pem_armoured.as_bytes().to_vec()),
            ("zeros after magic", [OPENSSH_MAGIC, &[0x00; 16]].concat()),
            ("trailing byte", [SSH_PRIVATE_KEY, &[0x00]].concat()),
        ] {
            assert_matches!(
                PrivateKey::parse(secret_bytes(invalid_key_bytes)).err(),
                Some(InvalidPrivateKey::Malformed),
                "{case}"
            );
        }
    }

    #[test]
    fn public_key_returns_key_from_private_key_header() {
        let private_key = PrivateKey::parse(secret_bytes(SSH_PRIVATE_KEY)).unwrap();

        let public_key = private_key.public_key();

        assert_eq!(&*public_key, SSH_PUBLIC_KEY);
    }

    #[test]
    fn with_host_key_on_password_and_new_key_returns_password_bound_to_host_key() {
        let new_key = keys::unauthorized_ssh_private_key().public_key();

        let authentication = password_authentication().with_host_key(new_key);

        assert_matches!(authentication.err(), Some(PasswordBoundToHostKey));
    }

    #[test]
    fn with_host_key_on_password_and_same_key_keeps_the_password() {
        let authentication = password_authentication()
            .with_host_key(keys::ssh_public_key())
            .unwrap();

        let AuthMethod::Password(password) = authentication.method() else {
            panic!("expected password authentication");
        };
        assert_eq!(password.expose_secret(), SSH_PASSWORD);
    }

    #[test]
    fn has_same_secret_with_the_same_password_returns_true() {
        let stored = password_method(SSH_PASSWORD);

        assert!(stored.has_same_secret(&password_method(SSH_PASSWORD)));
    }

    #[test]
    fn has_same_secret_with_another_password_returns_false() {
        let stored = password_method(SSH_PASSWORD);

        assert!(!stored.has_same_secret(&password_method("another password")));
    }

    #[test]
    fn has_same_secret_with_a_keypair_for_a_password_returns_false() {
        let stored = password_method(SSH_PASSWORD);
        let keypair = AuthMethod::Keypair {
            private_key: keys::ssh_private_key(),
            source: KeySource::Imported,
        };

        assert!(!stored.has_same_secret(&keypair));
    }

    #[test]
    fn has_same_secret_with_the_same_private_key_returns_true() {
        let keypair = || AuthMethod::Keypair {
            private_key: keys::ssh_private_key(),
            source: KeySource::Imported,
        };

        assert!(keypair().has_same_secret(&keypair()));
    }
}
