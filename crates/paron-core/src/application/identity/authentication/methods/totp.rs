use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use derive_more::{Deref, From};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use zeroize::Zeroizing;

use super::AuthenticationMethod;
use super::ports::{TotpCredentialRepository, TotpRegistrationRepository};
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::event::AuditKind;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::domain::user::User;

#[derive(Debug, thiserror::Error)]
pub enum VerifyTotpError {
    #[error("the code is not correct")]
    IncorrectCode,

    #[error("this code was used before")]
    UsedCode,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyTotpSecretError {
    #[error("the code is not correct")]
    IncorrectCode,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum StartTotpRegistrationError {
    #[error("the account already has an authenticator app")]
    AlreadyExists,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum FinishTotpRegistrationError {
    #[error("the account already has an authenticator app")]
    AlreadyExists,

    #[error("no registration of an authenticator app is in progress")]
    NotStarted,

    #[error("the code is not correct")]
    IncorrectCode,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
#[error("the code is {length} digits", length = TotpCode::LENGTH)]
pub struct InvalidTotpCode;

#[derive(Debug, Clone, PartialEq, Eq, Deref)]
#[deref(forward)]
pub struct TotpCode(String);

impl TotpCode {
    pub const LENGTH: u8 = 6;

    pub fn parse(raw: &str) -> Result<Self, InvalidTotpCode> {
        if raw.len() != usize::from(Self::LENGTH) || !raw.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(InvalidTotpCode);
        }

        Ok(Self(raw.to_owned()))
    }
}

/// The number of steps since the Unix epoch (RFC 6238 §4.2, `T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TotpStep(pub u64);

impl TotpStep {
    pub const SECONDS: u64 = 30;

    /// The first second of the step.
    pub fn start(self) -> jiff::Timestamp {
        i64::try_from(self.0.saturating_mul(Self::SECONDS))
            .ok()
            .and_then(|second| jiff::Timestamp::from_second(second).ok())
            .unwrap_or(jiff::Timestamp::MAX)
    }
}

/// A secret for RFC 6238 codes
#[derive(Clone, Debug, Deref, From)]
pub struct TotpSecret(SecretSlice<u8>);

impl TotpSecret {
    const LENGTH: usize = 20;
    const ALGORITHM: totp_rs::Algorithm = totp_rs::Algorithm::SHA1;
    const SKEW_STEPS: u16 = 1;

    pub fn generate() -> Self {
        use rand::Rng as _;

        let mut secret = vec![0u8; Self::LENGTH];
        rand::rng().fill_bytes(&mut secret);

        Self(SecretSlice::from(secret))
    }

    pub fn as_base32(&self) -> SecretString {
        let encoded =
            Zeroizing::new(totp_rs::Secret::from(self.0.expose_secret().to_vec()).to_base32());

        SecretString::from(encoded.as_str())
    }

    pub fn key_uri(&self, issuer: &str, account: &str) -> SecretString {
        SecretString::from(format!(
            "otpauth://totp/{label}?secret={secret}&issuer={issuer}&algorithm={algorithm}\
             &digits={digits}&period={period}",
            label = utf8_percent_encode(&format!("{issuer}:{account}"), NON_ALPHANUMERIC),
            secret = self.as_base32().expose_secret(),
            issuer = utf8_percent_encode(issuer, NON_ALPHANUMERIC),
            algorithm = Self::ALGORITHM,
            digits = TotpCode::LENGTH,
            period = TotpStep::SECONDS,
        ))
    }

    pub fn verify(
        &self,
        code: &TotpCode,
        now: jiff::Timestamp,
    ) -> Result<TotpStep, VerifyTotpSecretError> {
        let seconds =
            u64::try_from(now.as_second()).context("the time is before the Unix epoch")?;

        totp_rs::Builder::new()
            .with_secret(self.0.expose_secret().to_vec())
            .with_algorithm(Self::ALGORITHM)
            .with_digits(TotpCode::LENGTH)
            .with_step_duration(TotpStep::SECONDS)
            .with_skew(Self::SKEW_STEPS)
            .build()
            .context("could not build totp")?
            .check(code, seconds)
            .map(TotpStep)
            .ok_or(VerifyTotpSecretError::IncorrectCode)
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub struct TotpCredential {
    pub user_id: uuid::Uuid,
    pub secret: TotpSecret,
    pub last_used_step: TotpStep,
    pub created_at: jiff::Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpSummary {
    pub created_at: jiff::Timestamp,
    pub last_used_at: jiff::Timestamp,
}

impl From<TotpCredential> for TotpSummary {
    fn from(credential: TotpCredential) -> Self {
        Self {
            created_at: credential.created_at,
            last_used_at: credential.last_used_step.start(),
        }
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub struct TotpRegistration {
    pub user_id: uuid::Uuid,
    pub secret: TotpSecret,
    pub expires_at: jiff::Timestamp,
}

#[derive(Debug)]
pub struct TotpSetup {
    pub secret: SecretString,
    pub key_uri: SecretString,
}

pub struct TotpService<Tx> {
    repository: Arc<dyn TotpCredentialRepository<Tx>>,
    registrations: Arc<dyn TotpRegistrationRepository<Tx>>,
    audit: Arc<AuditService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    clock: Arc<dyn Clock>,
    registration_lifetime: Duration,
    label_host: url::Host,
}

impl<Tx: Send + Sync + 'static> TotpService<Tx> {
    const TOTP_ISSUER: &str = "Paron";

    pub fn new(
        repository: Arc<dyn TotpCredentialRepository<Tx>>,
        registrations: Arc<dyn TotpRegistrationRepository<Tx>>,
        audit: Arc<AuditService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        clock: Arc<dyn Clock>,
        registration_lifetime: Duration,
        label_host: url::Host,
    ) -> Self {
        Self {
            repository,
            registrations,
            audit,
            unit_of_work,
            clock,
            registration_lifetime,
            label_host,
        }
    }

    pub async fn summary(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Option<TotpSummary>, anyhow::Error> {
        match self.repository.get(ctx, user_id).await {
            Ok(credential) => Ok(Some(credential.into())),
            Err(RepositoryError::NotFound) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error).context("could not get totp")),
        }
    }

    pub async fn verify(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        code: &TotpCode,
    ) -> Result<(), VerifyTotpError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let credential = match self.repository.get(ctx, user_id).await {
                    Ok(credential) => credential,
                    Err(RepositoryError::NotFound) => return Err(VerifyTotpError::IncorrectCode),
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get totp")
                            .into());
                    }
                };
                let step = match credential.secret.verify(code, self.clock.now()) {
                    Ok(step) => step,
                    Err(VerifyTotpSecretError::IncorrectCode) => {
                        return Err(VerifyTotpError::IncorrectCode);
                    }
                    Err(VerifyTotpSecretError::Unexpected(cause)) => return Err(cause.into()),
                };
                if step <= credential.last_used_step {
                    return Err(VerifyTotpError::UsedCode);
                }

                self.repository
                    .update(
                        ctx,
                        &TotpCredential {
                            last_used_step: step,
                            ..credential
                        },
                    )
                    .await
                    .context("could not update totp credential")?;

                Ok(())
            })
            .await
    }

    pub async fn start_registration(
        &self,
        ctx: &mut TxContext<Tx>,
        user: &User,
    ) -> Result<TotpSetup, StartTotpRegistrationError> {
        if self.summary(ctx, user.id).await?.is_some() {
            return Err(StartTotpRegistrationError::AlreadyExists);
        }

        let secret = TotpSecret::generate();
        let setup = TotpSetup {
            secret: secret.as_base32(),
            key_uri: secret.key_uri(
                Self::TOTP_ISSUER,
                &format!("{}@{}", &*user.username, self.label_host),
            ),
        };
        self.registrations
            .upsert(
                ctx,
                &TotpRegistration {
                    user_id: user.id,
                    secret,
                    expires_at: self.clock.now() + self.registration_lifetime,
                },
            )
            .await
            .context("could not upsert totp registration")?;

        Ok(setup)
    }

    pub async fn finish_registration(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
        code: &TotpCode,
    ) -> Result<(), FinishTotpRegistrationError> {
        let now = self.clock.now();

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let registration = match self.registrations.get(ctx, user.id).await {
                    Ok(registration) if now < registration.expires_at => registration,
                    Ok(_) | Err(RepositoryError::NotFound) => {
                        return Err(FinishTotpRegistrationError::NotStarted);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get totp registration")
                            .into());
                    }
                };
                let step = match registration.secret.verify(code, now) {
                    Ok(step) => step,
                    Err(VerifyTotpSecretError::IncorrectCode) => {
                        return Err(FinishTotpRegistrationError::IncorrectCode);
                    }
                    Err(VerifyTotpSecretError::Unexpected(cause)) => return Err(cause.into()),
                };

                let created = self
                    .repository
                    .create(
                        ctx,
                        &TotpCredential {
                            user_id: user.id,
                            secret: registration.secret,
                            last_used_step: step,
                            created_at: now,
                        },
                    )
                    .await;
                match created {
                    Ok(_) => {}
                    Err(RepositoryError::AlreadyExists) => {
                        return Err(FinishTotpRegistrationError::AlreadyExists);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not create totp credential")
                            .into());
                    }
                }
                self.registrations
                    .delete(ctx, user.id)
                    .await
                    .context("could not delete totp registration")?;
                self.audit
                    .append(
                        ctx,
                        actor,
                        user,
                        AuditKind::CredentialAdded {
                            kind: AuthenticationMethod::Totp,
                        },
                    )
                    .await?;

                Ok(())
            })
            .await
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
                                    kind: AuthenticationMethod::Totp,
                                },
                            )
                            .await
                    }
                    Err(RepositoryError::NotFound) => Ok(()),
                    Err(error) => Err(anyhow::Error::new(error).context("could not delete totp")),
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn parse_totp_code_of_wrong_length_or_letters_returns_invalid_totp_code() {
        for raw in ["12345", "1234567", "12345a", "123 456", ""] {
            assert_matches!(TotpCode::parse(raw), Err(InvalidTotpCode), "{raw}");
        }
    }

    // The SHA-1 secret of RFC 6238 appendix B. At 59 seconds, its code is 94287082 with eight
    // digits, and 287082 with six.
    fn rfc_secret() -> TotpSecret {
        TotpSecret::from(SecretSlice::from(b"12345678901234567890".to_vec()))
    }

    #[test]
    fn verify_code_of_the_rfc_returns_its_step() {
        let code = TotpCode::parse("287082").unwrap();

        let step = rfc_secret().verify(&code, jiff::Timestamp::from_second(59).unwrap());

        assert_matches!(step, Ok(TotpStep(1)));
    }

    #[test]
    fn verify_code_two_steps_later_returns_incorrect_code() {
        let code = TotpCode::parse("287082").unwrap();

        let step = rfc_secret().verify(&code, jiff::Timestamp::from_second(120).unwrap());

        assert_matches!(step, Err(VerifyTotpSecretError::IncorrectCode));
    }

    #[test]
    fn start_of_step_one_returns_second_thirty() {
        let start = TotpStep(1).start();

        assert_eq!(start, jiff::Timestamp::from_second(30).unwrap());
    }

    #[test]
    fn key_uri_of_the_rfc_secret_returns_the_escaped_otpauth_uri() {
        let uri = rfc_secret().key_uri("Paron", "alice rossi");

        assert_eq!(
            uri.expose_secret(),
            "otpauth://totp/Paron%3Aalice%20rossi?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ\
             &issuer=Paron&algorithm=SHA1&digits=6&period=30"
        );
    }
}
