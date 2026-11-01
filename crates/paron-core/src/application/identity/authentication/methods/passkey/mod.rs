use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use webauthn_rs_proto::{
    PublicKeyCredential, PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions,
    RegisterPublicKeyCredential,
};

mod authenticator_data;
mod cbor;
pub mod ceremony;
mod cose;

use self::ceremony::{InvalidPasskey, PasskeyUserVerification};
use super::AuthenticationMethod;
use super::ports::{PasskeyCredentialRepository, PasskeyRegistrationRepository};
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::event::AuditKind;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::domain::text::LimitedText;
use crate::domain::user::User;

#[derive(Debug, thiserror::Error)]
pub enum StartPasskeyRegistrationError {
    #[error("a passkey does not work at this origin")]
    UnavailableAtOrigin,

    #[error("the account has the maximum number of passkeys")]
    LimitReached,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum FinishPasskeyRegistrationError {
    #[error("no registration of a passkey is in progress")]
    NotStarted,

    #[error("the response of the authenticator is not correct")]
    IncorrectResponse(#[source] InvalidPasskey),

    #[error("the authenticator did not verify the user")]
    NotVerified,

    #[error("passkey already exists")]
    AlreadyExists,

    #[error("the account has the maximum number of passkeys")]
    LimitReached,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RemovePasskeyError {
    #[error("passkey not found")]
    NotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyPasskeyError {
    #[error("no account has this passkey")]
    UnknownPasskey,

    #[error("the passkey belongs to a different user")]
    OtherUser,

    #[error("the authenticator did not name the owner of the passkey")]
    MissingUserHandle,

    #[error("the answer of the authenticator is not correct")]
    IncorrectResponse(#[source] InvalidPasskey),

    #[error("the authenticator did not verify the user")]
    NotVerified,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RenamePasskeyError {
    #[error("passkey not found")]
    NotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type PasskeyName = LimitedText<1, 64>;

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct PasskeyCredential {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub credential_id: CredentialId,
    pub public_key: Vec<u8>,
    pub name: PasskeyName,
    pub sign_count: u32,
    pub backup: PasskeyBackup,
    pub created_at: jiff::Timestamp,
    pub last_used_at: Option<jiff::Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeySummary {
    pub id: uuid::Uuid,
    pub name: PasskeyName,
    pub created_at: jiff::Timestamp,
    pub last_used_at: Option<jiff::Timestamp>,
    pub backed_up: bool,
}

impl From<PasskeyCredential> for PasskeySummary {
    fn from(passkey: PasskeyCredential) -> Self {
        Self {
            id: passkey.id,
            name: passkey.name,
            created_at: passkey.created_at,
            last_used_at: passkey.last_used_at,
            backed_up: passkey.backup.state,
        }
    }
}

/// The backup flags of a passkey (WebAuthn §6.1, BE and BS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PasskeyBackup {
    pub eligible: bool,
    pub state: bool,
}

/// The ID that the authenticator gives a passkey: opaque bytes (WebAuthn §4, Credential ID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialId(pub Vec<u8>);

/// The random value that the authenticator signs. WebAuthn §13.4.3 requires at least 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge(pub [u8; 32]);

impl Challenge {
    pub fn generate() -> Self {
        use rand::Rng as _;

        let mut challenge = [0; 32];
        rand::rng().fill_bytes(&mut challenge);

        Self(challenge)
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct PasskeyRegistration {
    pub user_id: uuid::Uuid,
    pub challenge: Challenge,
    pub expires_at: jiff::Timestamp,
}

/// The RP ID is the host of the origin, and a browser refuses an RP ID that is not a valid domain,
/// such as an IP address (WebAuthn §5.1.3).
pub fn available_at(origin: &url::Origin) -> bool {
    matches!(origin, url::Origin::Tuple(_, url::Host::Domain(_), _))
}

pub struct PasskeyService<Tx> {
    repository: Arc<dyn PasskeyCredentialRepository<Tx>>,
    registrations: Arc<dyn PasskeyRegistrationRepository<Tx>>,
    audit: Arc<AuditService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    clock: Arc<dyn Clock>,
    ceremony_lifetime: Duration,
}

impl<Tx: Send + Sync + 'static> PasskeyService<Tx> {
    const NEW_PASSKEY_NAME: &str = "Passkey";
    /// The most passkeys that one user can have. It keeps `excludeCredentials` and
    /// `allowCredentials` short enough for every authenticator.
    const MAX_PASSKEYS: usize = 10;

    pub fn new(
        repository: Arc<dyn PasskeyCredentialRepository<Tx>>,
        registrations: Arc<dyn PasskeyRegistrationRepository<Tx>>,
        audit: Arc<AuditService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        clock: Arc<dyn Clock>,
        ceremony_lifetime: Duration,
    ) -> Self {
        Self {
            repository,
            registrations,
            audit,
            unit_of_work,
            clock,
            ceremony_lifetime,
        }
    }

    pub async fn summaries(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Vec<PasskeySummary>, anyhow::Error> {
        let passkeys = self
            .repository
            .list_for_user(ctx, user_id)
            .await
            .context("could not list passkeys for user")?;

        Ok(passkeys.into_iter().map(PasskeySummary::from).collect())
    }

    pub async fn start_registration(
        &self,
        ctx: &mut TxContext<Tx>,
        user: &User,
        origin: &url::Origin,
    ) -> Result<PublicKeyCredentialCreationOptions, StartPasskeyRegistrationError> {
        if !available_at(origin) {
            return Err(StartPasskeyRegistrationError::UnavailableAtOrigin);
        }
        let exclude_credentials: Vec<CredentialId> = self
            .repository
            .list_for_user(ctx, user.id)
            .await
            .context("could not list passkeys for user")?
            .into_iter()
            .map(|passkey| passkey.credential_id)
            .collect();
        if exclude_credentials.len() >= Self::MAX_PASSKEYS {
            return Err(StartPasskeyRegistrationError::LimitReached);
        }
        let started = ceremony::start_registration(
            origin,
            user,
            &exclude_credentials,
            self.ceremony_lifetime,
        );

        self.registrations
            .upsert(
                ctx,
                &PasskeyRegistration {
                    user_id: user.id,
                    challenge: started.challenge,
                    expires_at: self.clock.now() + self.ceremony_lifetime,
                },
            )
            .await
            .context("could not upsert passkey registration")?;

        Ok(started.options)
    }

    pub async fn finish_registration(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
        origin: &url::Origin,
        response: &RegisterPublicKeyCredential,
    ) -> Result<PasskeyCredential, FinishPasskeyRegistrationError> {
        let now = self.clock.now();

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let registration = match self.registrations.delete(ctx, user.id).await {
                    Ok(registration) if now < registration.expires_at => registration,
                    Ok(_) | Err(RepositoryError::NotFound) => {
                        return Err(FinishPasskeyRegistrationError::NotStarted);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not delete passkey registration")
                            .into());
                    }
                };
                let new_passkey = match ceremony::finish_registration(
                    origin,
                    &registration.challenge,
                    response,
                ) {
                    Ok(new_passkey) => new_passkey,
                    Err(invalid) => {
                        return Err(FinishPasskeyRegistrationError::IncorrectResponse(invalid));
                    }
                };
                if new_passkey.user_verification != PasskeyUserVerification::Verified {
                    return Err(FinishPasskeyRegistrationError::NotVerified);
                }
                if self.summaries(ctx, user.id).await?.len() >= Self::MAX_PASSKEYS {
                    return Err(FinishPasskeyRegistrationError::LimitReached);
                }

                let passkey = PasskeyCredential {
                    id: uuid::Uuid::now_v7(),
                    user_id: user.id,
                    credential_id: new_passkey.credential_id,
                    public_key: new_passkey.public_key,
                    name: PasskeyName::parse(Self::NEW_PASSKEY_NAME)
                        .context("the name of a new passkey is not valid")?,
                    sign_count: new_passkey.sign_count,
                    backup: new_passkey.backup,
                    created_at: now,
                    last_used_at: None,
                };
                let created = match self.repository.create(ctx, &passkey).await {
                    Ok(created) => created,
                    Err(RepositoryError::AlreadyExists) => {
                        return Err(FinishPasskeyRegistrationError::AlreadyExists);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not create passkey")
                            .into());
                    }
                };
                self.audit
                    .append(
                        ctx,
                        actor,
                        user,
                        AuditKind::CredentialAdded {
                            kind: AuthenticationMethod::Passkey,
                        },
                    )
                    .await?;

                Ok(created)
            })
            .await
    }

    pub async fn remove(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
        passkey_id: uuid::Uuid,
    ) -> Result<PasskeyCredential, RemovePasskeyError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                match self.repository.delete(ctx, user.id, passkey_id).await {
                    Ok(passkey) => {
                        self.audit
                            .append(
                                ctx,
                                actor,
                                user,
                                AuditKind::CredentialRemoved {
                                    kind: AuthenticationMethod::Passkey,
                                },
                            )
                            .await?;

                        Ok(passkey)
                    }
                    Err(RepositoryError::NotFound) => Err(RemovePasskeyError::NotFound),
                    Err(error) => Err(anyhow::Error::new(error)
                        .context("could not delete passkey")
                        .into()),
                }
            })
            .await
    }

    pub async fn rename(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        passkey_id: uuid::Uuid,
        name: PasskeyName,
    ) -> Result<PasskeyCredential, RenamePasskeyError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let passkey = self
                    .repository
                    .list_for_user(ctx, user_id)
                    .await
                    .context("could not list passkeys for user")?
                    .into_iter()
                    .find(|passkey| passkey.id == passkey_id)
                    .ok_or(RenamePasskeyError::NotFound)?;

                Ok(self
                    .repository
                    .update(ctx, &PasskeyCredential { name, ..passkey })
                    .await
                    .context("could not update passkey")?)
            })
            .await
    }

    pub async fn start_authentication(
        &self,
        ctx: &mut TxContext<Tx>,
        origin: &url::Origin,
        user_id: Option<uuid::Uuid>,
    ) -> Result<(PublicKeyCredentialRequestOptions, Challenge), anyhow::Error> {
        let allow_credentials: Vec<CredentialId> = match user_id {
            Some(user_id) => self
                .repository
                .list_for_user(ctx, user_id)
                .await
                .context("could not list passkeys for user")?
                .into_iter()
                .map(|passkey| passkey.credential_id)
                .collect(),
            None => Vec::new(),
        };
        let started =
            ceremony::start_authentication(origin, &allow_credentials, self.ceremony_lifetime);

        Ok((started.options, started.challenge))
    }

    pub async fn verify(
        &self,
        ctx: &mut TxContext<Tx>,
        origin: &url::Origin,
        challenge: &Challenge,
        response: &PublicKeyCredential,
        identified_user: Option<uuid::Uuid>,
    ) -> Result<uuid::Uuid, VerifyPasskeyError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let passkey = match self
                    .repository
                    .get_by_credential_id(ctx, &CredentialId(response.raw_id.clone()))
                    .await
                {
                    Ok(passkey) => passkey,
                    Err(RepositoryError::NotFound) => {
                        return Err(VerifyPasskeyError::UnknownPasskey);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get passkey by credential id")
                            .into());
                    }
                };
                // WebAuthn §7.2 step 6: the passkey belongs to the user that the sign-in knows,
                // and a user handle names its owner. Without a known user, the user handle is
                // required.
                if identified_user.is_some_and(|user_id| user_id != passkey.user_id) {
                    return Err(VerifyPasskeyError::OtherUser);
                }
                match response.response.user_handle.as_deref() {
                    Some(handle) if handle != passkey.user_id.as_bytes() => {
                        return Err(VerifyPasskeyError::OtherUser);
                    }
                    None if identified_user.is_none() => {
                        return Err(VerifyPasskeyError::MissingUserHandle);
                    }
                    _ => {}
                }
                let assertion =
                    match ceremony::finish_authentication(origin, challenge, response, &passkey) {
                        Ok(assertion) => assertion,
                        Err(invalid) => return Err(VerifyPasskeyError::IncorrectResponse(invalid)),
                    };
                if assertion.user_verification != PasskeyUserVerification::Verified {
                    return Err(VerifyPasskeyError::NotVerified);
                }

                self.repository
                    .update(
                        ctx,
                        &PasskeyCredential {
                            sign_count: assertion.sign_count,
                            backup: assertion.backup,
                            last_used_at: Some(self.clock.now()),
                            ..passkey
                        },
                    )
                    .await
                    .context("could not update passkey")?;

                Ok(passkey.user_id)
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
                let deleted = self
                    .repository
                    .delete_for_user(ctx, user.id)
                    .await
                    .context("could not delete passkeys for user")?;
                if deleted.is_empty() {
                    return Ok(());
                }

                self.audit
                    .append(
                        ctx,
                        actor,
                        user,
                        AuditKind::CredentialRemoved {
                            kind: AuthenticationMethod::Passkey,
                        },
                    )
                    .await
            })
            .await
    }
}
