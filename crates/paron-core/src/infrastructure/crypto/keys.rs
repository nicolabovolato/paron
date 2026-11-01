use russh::keys::{Algorithm, PrivateKey};
use secrecy::SecretSlice;

use crate::application::machine::{SshKeyAlgorithm, SshKeypairs};
use crate::domain::machine::ssh;

pub struct RusshKeypairs;

impl SshKeypairs for RusshKeypairs {
    #[expect(clippy::expect_used)]
    fn generate(&self, algorithm: SshKeyAlgorithm) -> ssh::PrivateKey {
        let algorithm = match algorithm {
            SshKeyAlgorithm::Ed25519 => Algorithm::Ed25519,
        };

        let key = PrivateKey::random(&mut rand::rng(), algorithm)
            .and_then(|key| key.to_bytes())
            .expect("russh generates and encodes a key of every algorithm matched above");

        ssh::PrivateKey::parse(SecretSlice::from(key.to_vec()))
            .expect("a newly generated key is in the OpenSSH format and has no passphrase")
    }
}
