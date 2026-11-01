use std::sync::Arc;

use argon2::{Argon2, PasswordHasher as _, PasswordVerifier as _};
use async_trait::async_trait;
use secrecy::ExposeSecret as _;

use crate::application::identity::authentication::methods::password::{Password, PasswordPhc};
use crate::application::identity::authentication::methods::ports::PasswordHasher;

pub struct BoundedArgon2PasswordHasher {
    argon2: Argon2<'static>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl BoundedArgon2PasswordHasher {
    pub fn new(concurrency: usize) -> Self {
        Self {
            argon2: Argon2::default(),
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
        }
    }
}

#[async_trait]
impl PasswordHasher for BoundedArgon2PasswordHasher {
    #[expect(
        clippy::expect_used,
        reason = "the semaphore is never closed, Argon2 with its default parameters hashes any password, and the operating system always gives random bytes"
    )]
    async fn hash(&self, password: &Password) -> PasswordPhc {
        let (argon2, password) = (self.argon2.clone(), password.clone());
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .expect("could not acquire a password hashing permit");

        tokio::task::spawn_blocking(move || {
            let _permit = permit;

            let hash = argon2
                .hash_password(password.expose_secret().as_bytes())
                .expect("could not hash the password");

            PasswordPhc::parse(hash.to_string()).expect("could not parse the hash")
        })
        .await
        .unwrap_or_else(|error| match error.try_into_panic() {
            Ok(payload) => std::panic::resume_unwind(payload),
            Err(reason) => panic!("the password hashing task was cancelled: {reason}"),
        })
    }

    #[expect(clippy::expect_used, reason = "the semaphore is never closed")]
    async fn verify(&self, password: &Password, hash: &PasswordPhc) -> bool {
        let (argon2, password, hash) = (self.argon2.clone(), password.clone(), hash.clone());
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .expect("could not acquire a password hashing permit");

        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            argon2
                .verify_password(password.expose_secret().as_bytes(), &*hash)
                .is_ok()
        })
        .await
        .unwrap_or_else(|error| match error.try_into_panic() {
            Ok(payload) => std::panic::resume_unwind(payload),
            Err(reason) => panic!("the password verification task was cancelled: {reason}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::testing::identity;

    const CONCURRENCY: usize = 1;

    #[tokio::test]
    async fn verify_the_hashed_password_returns_true() {
        let hasher = BoundedArgon2PasswordHasher::new(CONCURRENCY);
        let password = identity::password();
        let phc = hasher.hash(&password).await;

        let verified = hasher.verify(&password, &phc).await;

        assert!(verified);
    }

    #[tokio::test]
    async fn verify_another_password_returns_false() {
        let hasher = BoundedArgon2PasswordHasher::new(CONCURRENCY);
        let phc = hasher.hash(&identity::password()).await;
        let other_password =
            Password::parse(SecretString::from("another password entirely")).unwrap();

        let verified = hasher.verify(&other_password, &phc).await;

        assert!(!verified);
    }

    #[tokio::test]
    async fn verify_phc_of_another_algorithm_returns_false() {
        let hasher = BoundedArgon2PasswordHasher::new(CONCURRENCY);
        let scrypt_phc = PasswordPhc::parse(
            "$scrypt$ln=17,r=8,p=1$c29tZXNhbHR2YWx1ZQ$3Vh0YkFDR0hJSktMTU5PUFFSU1RVVldY".to_owned(),
        )
        .unwrap();

        let verified = hasher.verify(&identity::password(), &scrypt_phc).await;

        assert!(!verified);
    }

    #[tokio::test]
    async fn hash_twice_returns_different_phc_strings() {
        let hasher = BoundedArgon2PasswordHasher::new(CONCURRENCY);
        let password = identity::password();

        let first_phc = hasher.hash(&password).await;
        let second_phc = hasher.hash(&password).await;

        assert_ne!(*first_phc, *second_phc);
    }

    #[tokio::test]
    async fn hash_returns_argon2id_phc() {
        let hasher = BoundedArgon2PasswordHasher::new(CONCURRENCY);

        let phc = hasher.hash(&identity::password()).await;

        assert!(phc.starts_with("$argon2id$"));
    }
}
