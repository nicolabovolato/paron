use std::sync::Arc;

use anyhow::Context as _;
use derive_more::Deref;
use secrecy::{ExposeSecret as _, SecretString};

use super::AuthenticationMethod;
use super::ports::{PasswordCredentialRepository, PasswordHasher};
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::event::AuditKind;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::domain::user::User;

#[derive(Debug, thiserror::Error)]
pub enum VerifyPasswordError {
    #[error("the password is not correct")]
    IncorrectPassword,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidPassword {
    #[error("a password is at least {min} characters", min = Password::MIN)]
    TooShort,

    #[error("a password is at most {max} characters", max = Password::MAX)]
    TooLong,
}

#[derive(Clone, Deref)]
pub struct Password(SecretString);

impl Password {
    pub const MIN: usize = 12;
    pub const MAX: usize = 64;

    pub fn parse(raw: SecretString) -> Result<Self, InvalidPassword> {
        let length = raw.expose_secret().chars().count();

        if length < Self::MIN {
            return Err(InvalidPassword::TooShort);
        }

        if length > Self::MAX {
            return Err(InvalidPassword::TooLong);
        }

        Ok(Self(raw))
    }
}

/// A hashed password in PHC format.
#[derive(Debug, Clone, PartialEq, Eq, Deref)]
#[deref(forward)]
pub struct PasswordPhc(String);

impl PasswordPhc {
    pub fn parse(phc: String) -> Result<Self, InvalidPasswordPhc> {
        password_hash::phc::PasswordHash::new(&phc).map_err(|_| InvalidPasswordPhc)?;

        Ok(Self(phc))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the password hash is not in PHC format")]
pub struct InvalidPasswordPhc;

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct PasswordCredential {
    pub user_id: uuid::Uuid,
    pub phc: PasswordPhc,
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordSummary {
    pub updated_at: jiff::Timestamp,
}

impl From<PasswordCredential> for PasswordSummary {
    fn from(credential: PasswordCredential) -> Self {
        Self {
            updated_at: credential.updated_at,
        }
    }
}

pub struct PasswordService<Tx> {
    repository: Arc<dyn PasswordCredentialRepository<Tx>>,
    hasher: Arc<dyn PasswordHasher>,
    audit: Arc<AuditService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    clock: Arc<dyn Clock>,
}

impl<Tx: Send + Sync + 'static> PasswordService<Tx> {
    pub fn new(
        repository: Arc<dyn PasswordCredentialRepository<Tx>>,
        hasher: Arc<dyn PasswordHasher>,
        audit: Arc<AuditService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            hasher,
            audit,
            unit_of_work,
            clock,
        }
    }

    pub async fn summary(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Option<PasswordSummary>, anyhow::Error> {
        match self.repository.get(ctx, user_id).await {
            Ok(credential) => Ok(Some(credential.into())),
            Err(RepositoryError::NotFound) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error).context("could not get password")),
        }
    }

    pub async fn replace(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
        password: &Password,
    ) -> Result<(), anyhow::Error> {
        let phc = self.hasher.hash(password).await;

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                self.repository
                    .upsert(
                        ctx,
                        &PasswordCredential {
                            user_id: user.id,
                            phc,
                            updated_at: self.clock.now(),
                        },
                    )
                    .await
                    .context("could not upsert password")?;

                self.audit
                    .append(
                        ctx,
                        actor,
                        user,
                        AuditKind::CredentialAdded {
                            kind: AuthenticationMethod::Password,
                        },
                    )
                    .await
            })
            .await
    }

    pub async fn verify(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: Option<uuid::Uuid>,
        password: &Password,
    ) -> Result<(), VerifyPasswordError> {
        let credential = match user_id {
            Some(user_id) => match self.repository.get(ctx, user_id).await {
                Ok(credential) => Some(credential),
                Err(RepositoryError::NotFound) => None,
                Err(error) => {
                    return Err(anyhow::Error::new(error)
                        .context("could not get password")
                        .into());
                }
            },
            // Still hash the password to avoid timing attacks.
            None => None,
        };
        let Some(credential) = credential else {
            self.hasher.hash(password).await;

            return Err(VerifyPasswordError::IncorrectPassword);
        };

        if !self.hasher.verify(password, &credential.phc).await {
            return Err(VerifyPasswordError::IncorrectPassword);
        }

        Ok(())
    }

    pub async fn unenroll(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
    ) -> Result<(), anyhow::Error> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                match self.repository.delete(ctx, user.id).await {
                    Ok(_) => {
                        self.audit
                            .append(
                                ctx,
                                actor,
                                user,
                                AuditKind::CredentialRemoved {
                                    kind: AuthenticationMethod::Password,
                                },
                            )
                            .await
                    }
                    Err(RepositoryError::NotFound) => Ok(()),
                    Err(error) => {
                        Err(anyhow::Error::new(error).context("could not delete password"))
                    }
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::identity::PASSWORD_PHC;

    #[test]
    fn parse_password_at_the_minimum_and_the_maximum_succeeds() {
        for valid_password in ["a".repeat(Password::MIN), "a".repeat(Password::MAX)] {
            Password::parse(SecretString::from(valid_password)).unwrap();
        }
    }

    #[test]
    fn parse_short_password_returns_too_short() {
        let parsed = Password::parse(SecretString::from("a".repeat(Password::MIN - 1)));

        assert_matches!(parsed.err(), Some(InvalidPassword::TooShort));
    }

    #[test]
    fn parse_long_password_returns_too_long() {
        let parsed = Password::parse(SecretString::from("a".repeat(Password::MAX + 1)));

        assert_matches!(parsed.err(), Some(InvalidPassword::TooLong));
    }

    #[test]
    fn parse_password_of_multibyte_characters_at_the_maximum_succeeds() {
        Password::parse(SecretString::from("é".repeat(Password::MAX))).unwrap();
    }

    #[test]
    fn parse_password_phc_of_any_algorithm_succeeds() {
        for valid_phc in [
            PASSWORD_PHC,
            "$scrypt$ln=17,r=8,p=1$c29tZXNhbHR2YWx1ZQ$3Vh0YkFDR0hJSktMTU5PUFFSU1RVVldY",
        ] {
            assert_matches!(
                PasswordPhc::parse(valid_phc.to_owned()),
                Ok(_),
                "{valid_phc}"
            );
        }
    }

    #[test]
    fn parse_password_phc_of_other_text_returns_invalid_password_phc() {
        for invalid_phc in ["", "nonsense"] {
            assert_matches!(
                PasswordPhc::parse(invalid_phc.to_owned()),
                Err(InvalidPasswordPhc),
                "{invalid_phc:?}"
            );
        }
    }
}
