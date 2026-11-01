use std::sync::Arc;

use anyhow::Context as _;
use derive_more::Deref;
use secrecy::SecretString;
use zeroize::Zeroizing;

use super::AuthenticationMethod;
use super::ports::RecoveryCodeRepository;
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::event::AuditKind;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;
use crate::domain::user::User;

#[derive(Debug, thiserror::Error)]
pub enum VerifyRecoveryCodeError {
    #[error("the code is not correct, or it was used before")]
    IncorrectCode,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
#[error("invalid recovery code")]
pub struct InvalidRecoveryCode;

#[derive(Deref)]
pub struct RecoveryCode(SecretString);

impl RecoveryCode {
    /// Crockford Base32
    pub const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    pub const LENGTH: usize = 10;

    pub fn generate() -> Self {
        use rand::Rng as _;

        let mut random = Zeroizing::new([0u8; Self::LENGTH]);
        rand::rng().fill_bytes(random.as_mut());
        let characters = Zeroizing::new(
            random
                .iter()
                .map(|byte| char::from(Self::ALPHABET[usize::from(*byte) % Self::ALPHABET.len()]))
                .collect::<String>(),
        );

        Self(SecretString::from(characters.as_str()))
    }

    pub fn parse(raw: &str) -> Result<Self, InvalidRecoveryCode> {
        if raw.len() != Self::LENGTH || !raw.bytes().all(|byte| Self::ALPHABET.contains(&byte)) {
            return Err(InvalidRecoveryCode);
        }

        Ok(Self(SecretString::from(raw)))
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct RecoveryCodeCredential {
    pub user_id: uuid::Uuid,
    pub code_macs: Vec<TokenMac>,
    pub generated_at: jiff::Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCodesSummary {
    pub generated_at: jiff::Timestamp,
    pub remaining: usize,
}

impl From<RecoveryCodeCredential> for RecoveryCodesSummary {
    fn from(credential: RecoveryCodeCredential) -> Self {
        Self {
            generated_at: credential.generated_at,
            remaining: credential.code_macs.len(),
        }
    }
}

pub struct RecoveryCodeService<Tx> {
    repository: Arc<dyn RecoveryCodeRepository<Tx>>,
    tokens: Arc<dyn Tokens>,
    audit: Arc<AuditService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    clock: Arc<dyn Clock>,
    code_count: usize,
}

impl<Tx: Send + Sync + 'static> RecoveryCodeService<Tx> {
    pub fn new(
        repository: Arc<dyn RecoveryCodeRepository<Tx>>,
        tokens: Arc<dyn Tokens>,
        audit: Arc<AuditService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        clock: Arc<dyn Clock>,
        code_count: usize,
    ) -> Self {
        Self {
            repository,
            tokens,
            audit,
            unit_of_work,
            clock,
            code_count,
        }
    }

    pub async fn summary(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Option<RecoveryCodesSummary>, anyhow::Error> {
        match self.repository.get(ctx, user_id).await {
            Ok(credential) => Ok(Some(credential.into())),
            Err(RepositoryError::NotFound) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error).context("could not get recovery codes")),
        }
    }

    pub async fn replace(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
    ) -> Result<Vec<SecretString>, anyhow::Error> {
        let codes: Vec<RecoveryCode> = (0..self.code_count)
            .map(|_| RecoveryCode::generate())
            .collect();
        let code_macs = codes.iter().map(|code| self.tokens.mac(code)).collect();

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                self.repository
                    .upsert(
                        ctx,
                        &RecoveryCodeCredential {
                            user_id: user.id,
                            code_macs,
                            generated_at: self.clock.now(),
                        },
                    )
                    .await
                    .context("could not upsert recovery codes")?;
                self.audit
                    .append(
                        ctx,
                        actor,
                        user,
                        AuditKind::CredentialAdded {
                            kind: AuthenticationMethod::RecoveryCode,
                        },
                    )
                    .await
            })
            .await?;

        Ok(codes.into_iter().map(|code| code.0).collect())
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
                                    kind: AuthenticationMethod::RecoveryCode,
                                },
                            )
                            .await
                    }
                    Err(RepositoryError::NotFound) => Ok(()),
                    Err(error) => {
                        Err(anyhow::Error::new(error).context("could not delete recovery codes"))
                    }
                }
            })
            .await
    }

    pub async fn verify(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        code: &RecoveryCode,
    ) -> Result<(), VerifyRecoveryCodeError> {
        let mac = self.tokens.mac(code);

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let mut credential = match self.repository.get(ctx, user_id).await {
                    Ok(credential) => credential,
                    Err(RepositoryError::NotFound) => {
                        return Err(VerifyRecoveryCodeError::IncorrectCode);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get recovery codes")
                            .into());
                    }
                };
                let Some(position) = credential
                    .code_macs
                    .iter()
                    .position(|stored| *stored == mac)
                else {
                    return Err(VerifyRecoveryCodeError::IncorrectCode);
                };

                credential.code_macs.remove(position);
                self.repository
                    .upsert(ctx, &credential)
                    .await
                    .context("could not upsert recovery codes")?;

                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn parse_of_a_generated_code_succeeds() {
        let code = RecoveryCode::generate();

        assert!(RecoveryCode::parse(code.expose_secret()).is_ok());
    }

    #[test]
    fn parse_with_capitals_dashes_or_spaces_returns_invalid_recovery_code() {
        for invalid_code in ["ABCDEFGHJK", "abcde-fghjk", " abcdefghjk"] {
            assert_matches!(
                RecoveryCode::parse(invalid_code).err(),
                Some(InvalidRecoveryCode),
                "{invalid_code:?}"
            );
        }
    }

    #[test]
    fn parse_with_a_letter_that_looks_like_another_returns_invalid_recovery_code() {
        assert_matches!(
            RecoveryCode::parse("abcdefghij").err(),
            Some(InvalidRecoveryCode)
        );
    }
}
