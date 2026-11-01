use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use secrecy::SecretString;
use webauthn_rs_proto::{
    PublicKeyCredential, PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions,
    RegisterPublicKeyCredential,
};

use super::methods::passkey::{
    Challenge, FinishPasskeyRegistrationError, PasskeyName, PasskeyService, PasskeySummary,
    RemovePasskeyError, RenamePasskeyError, StartPasskeyRegistrationError, VerifyPasskeyError,
};
use super::methods::password::{Password, PasswordService, PasswordSummary, VerifyPasswordError};
use super::methods::recovery_code::{
    RecoveryCode, RecoveryCodeService, RecoveryCodesSummary, VerifyRecoveryCodeError,
};
use super::methods::totp::{
    FinishTotpRegistrationError, StartTotpRegistrationError, TotpCode, TotpService, TotpSetup,
    TotpSummary, VerifyTotpError,
};
use super::methods::{AuthenticationMethod, MethodRole};
use super::sessions::SessionService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::{Actor, Denied};
use crate::application::identity::errors::{
    ExistingCredential, GetUserError, ListCredentialsError, UpdateCredentialsError,
};
use crate::application::identity::use_cases::CredentialManagement;
use crate::application::identity::users::UserService;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::domain::user::{Permission, User};

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct AccountCredentials {
    pub password: Option<PasswordSummary>,
    pub totp: Option<TotpSummary>,
    pub passkeys: Vec<PasskeySummary>,
    pub recovery_codes: Option<RecoveryCodesSummary>,
}

impl AccountCredentials {
    pub fn has(&self, method: AuthenticationMethod) -> bool {
        match method {
            AuthenticationMethod::Password => self.password.is_some(),
            AuthenticationMethod::Totp => self.totp.is_some(),
            AuthenticationMethod::Passkey => !self.passkeys.is_empty(),
            AuthenticationMethod::RecoveryCode => self
                .recovery_codes
                .as_ref()
                .is_some_and(|codes| codes.remaining > 0),
        }
    }

    pub fn two_factor_enabled(&self) -> bool {
        self.has_method_with_role(|role| role == MethodRole::SecondFactor)
    }

    pub fn needs_recovery_codes(&self) -> bool {
        self.two_factor_enabled()
            && self.has_method_with_role(|role| role != MethodRole::FirstFactor)
    }

    pub fn has_first_factor(&self) -> bool {
        self.has_method_with_role(|role| role != MethodRole::SecondFactor)
    }

    fn has_method_with_role(&self, role_matches: impl Fn(MethodRole) -> bool) -> bool {
        AuthenticationMethod::ALL
            .iter()
            .filter(|&&method| method != AuthenticationMethod::RecoveryCode)
            .any(|&method| role_matches(method.role()) && self.has(method))
    }
}

#[cfg_attr(test, derive(Debug))]
#[allow(clippy::large_enum_variant)]
pub enum AcceptedCredential {
    Password,
    Totp,
    RecoveryCode,
    Passkey {
        options: PublicKeyCredentialRequestOptions,
        challenge: Challenge,
    },
}

impl AcceptedCredential {
    pub fn method(&self) -> AuthenticationMethod {
        match self {
            Self::Password => AuthenticationMethod::Password,
            Self::Totp => AuthenticationMethod::Totp,
            Self::RecoveryCode => AuthenticationMethod::RecoveryCode,
            Self::Passkey { .. } => AuthenticationMethod::Passkey,
        }
    }

    pub fn challenge(&self) -> Option<&Challenge> {
        match self {
            Self::Passkey { challenge, .. } => Some(challenge),
            Self::Password | Self::Totp | Self::RecoveryCode => None,
        }
    }
}

#[cfg_attr(test, derive(Debug))]
pub struct AcceptedCredentials(Vec<AcceptedCredential>);

impl AcceptedCredentials {
    pub fn new(credentials: Vec<AcceptedCredential>) -> Option<Self> {
        (!credentials.is_empty()).then_some(Self(credentials))
    }

    pub fn as_slice(&self) -> &[AcceptedCredential] {
        &self.0
    }
}

impl IntoIterator for AcceptedCredentials {
    type Item = AcceptedCredential;
    type IntoIter = std::vec::IntoIter<AcceptedCredential>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub struct PasskeyChallenge {
    pub origin: url::Origin,
    pub challenge: Challenge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedUser {
    Known(uuid::Uuid),
    Unknown,
    Nonexistent,
}

pub enum CredentialToVerify {
    Password(Password),
    Totp(TotpCode),
    RecoveryCode(RecoveryCode),
    Passkey(PublicKeyCredential),
}

impl CredentialToVerify {
    pub fn method(&self) -> AuthenticationMethod {
        match self {
            Self::Password(_) => AuthenticationMethod::Password,
            Self::Totp(_) => AuthenticationMethod::Totp,
            Self::RecoveryCode(_) => AuthenticationMethod::RecoveryCode,
            Self::Passkey(_) => AuthenticationMethod::Passkey,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyCredentialError {
    #[error("the credential is not correct")]
    Rejected(#[source] anyhow::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub enum CredentialToAdd {
    Password(Password),
    Totp(TotpCode),
    Passkey {
        origin: url::Origin,
        response: RegisterPublicKeyCredential,
    },
}

pub enum CredentialToRemove {
    Password,
    Totp,
    Passkey(uuid::Uuid),
}

pub enum UpdateCredentialsCommand {
    Add(CredentialToAdd),
    StartTotpRegistration,
    StartPasskeyRegistration { origin: url::Origin },
    RenamePasskey { id: uuid::Uuid, name: PasskeyName },
    Remove(CredentialToRemove),
    GenerateRecoveryCodes,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CredentialUpdateResult {
    Done,
    TotpRegistrationStarted(TotpSetup),
    PasskeyCreationOptions(PublicKeyCredentialCreationOptions),
}

#[cfg_attr(test, derive(Debug))]
pub struct UpdatedCredentials {
    pub result: CredentialUpdateResult,
    pub recovery_codes: Option<Vec<SecretString>>,
}

pub struct CredentialService<Tx> {
    password: Arc<PasswordService<Tx>>,
    totp: Arc<TotpService<Tx>>,
    passkeys: Arc<PasskeyService<Tx>>,
    recovery_codes: Arc<RecoveryCodeService<Tx>>,
    users: Arc<UserService<Tx>>,
    sessions: Arc<SessionService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    clock: Arc<dyn Clock>,
}

impl<Tx: Send + Sync + 'static> CredentialService<Tx> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        password: Arc<PasswordService<Tx>>,
        totp: Arc<TotpService<Tx>>,
        passkeys: Arc<PasskeyService<Tx>>,
        recovery_codes: Arc<RecoveryCodeService<Tx>>,
        users: Arc<UserService<Tx>>,
        sessions: Arc<SessionService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            password,
            totp,
            passkeys,
            recovery_codes,
            users,
            sessions,
            unit_of_work,
            clock,
        }
    }

    pub async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountCredentials, ListCredentialsError> {
        if !actor.is_user(user_id) {
            actor.authorize(Permission::UserRead)?;
        }

        Ok(AccountCredentials {
            password: self.password.summary(ctx, user_id).await?,
            totp: self.totp.summary(ctx, user_id).await?,
            passkeys: self.passkeys.summaries(ctx, user_id).await?,
            recovery_codes: self.recovery_codes.summary(ctx, user_id).await?,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user_id: uuid::Uuid,
        cmd: UpdateCredentialsCommand,
    ) -> Result<UpdatedCredentials, UpdateCredentialsError> {
        let kept_session_id = match actor {
            Actor::User(authenticated) if authenticated.user.id == user_id => {
                actor.require_recent_authentication(self.clock.now())?;
                Some(authenticated.session_id)
            }
            Actor::User(_) => return Err(Denied::NotAllowed.into()),
            Actor::System => None,
        };
        let user = &match self.users.get_by_id(ctx, actor, user_id).await {
            Ok(user) => user,
            Err(GetUserError::Denied(denied)) => return Err(denied.into()),
            Err(GetUserError::NotFound) => return Err(UpdateCredentialsError::UserNotFound),
            Err(GetUserError::Unexpected(cause)) => return Err(cause.into()),
        };
        let (result, recovery_codes) = match cmd {
            UpdateCredentialsCommand::Add(credential) => {
                let ((), recovery_codes) = self
                    .with_credential_change(ctx, actor, kept_session_id, user, async |ctx, _| {
                        match credential {
                            CredentialToAdd::Password(password) => {
                                self.password.replace(ctx, actor, user, &password).await?;
                            }
                            CredentialToAdd::Totp(code) => {
                                match self.totp.finish_registration(ctx, actor, user, &code).await {
                                    Ok(()) => {}
                                    Err(FinishTotpRegistrationError::AlreadyExists) => {
                                        return Err(UpdateCredentialsError::AlreadyExists(
                                            ExistingCredential::Totp,
                                        ));
                                    }
                                    Err(FinishTotpRegistrationError::Unexpected(cause)) => {
                                        return Err(cause.into());
                                    }
                                    Err(rejection) => {
                                        return Err(UpdateCredentialsError::InvalidCredentials(
                                            rejection.into(),
                                        ));
                                    }
                                }
                            }
                            CredentialToAdd::Passkey { origin, response } => {
                                match self
                                    .passkeys
                                    .finish_registration(ctx, actor, user, &origin, &response)
                                    .await
                                {
                                    Ok(_) => {}
                                    Err(FinishPasskeyRegistrationError::AlreadyExists) => {
                                        return Err(UpdateCredentialsError::AlreadyExists(
                                            ExistingCredential::Passkey,
                                        ));
                                    }
                                    Err(FinishPasskeyRegistrationError::LimitReached) => {
                                        return Err(UpdateCredentialsError::LimitReached(
                                            AuthenticationMethod::Passkey,
                                        ));
                                    }
                                    Err(FinishPasskeyRegistrationError::Unexpected(cause)) => {
                                        return Err(cause.into());
                                    }
                                    Err(rejection) => {
                                        return Err(UpdateCredentialsError::InvalidCredentials(
                                            rejection.into(),
                                        ));
                                    }
                                }
                            }
                        }

                        Ok(())
                    })
                    .await?;

                (CredentialUpdateResult::Done, recovery_codes)
            }
            UpdateCredentialsCommand::StartTotpRegistration => {
                let setup = match self.totp.start_registration(ctx, user).await {
                    Ok(setup) => setup,
                    Err(StartTotpRegistrationError::AlreadyExists) => {
                        return Err(UpdateCredentialsError::AlreadyExists(
                            ExistingCredential::Totp,
                        ));
                    }
                    Err(StartTotpRegistrationError::Unexpected(cause)) => return Err(cause.into()),
                };

                (CredentialUpdateResult::TotpRegistrationStarted(setup), None)
            }
            UpdateCredentialsCommand::StartPasskeyRegistration { origin } => {
                let options = match self.passkeys.start_registration(ctx, user, &origin).await {
                    Ok(options) => options,
                    Err(error @ StartPasskeyRegistrationError::UnavailableAtOrigin) => {
                        return Err(UpdateCredentialsError::InvalidCredentials(error.into()));
                    }
                    Err(StartPasskeyRegistrationError::LimitReached) => {
                        return Err(UpdateCredentialsError::LimitReached(
                            AuthenticationMethod::Passkey,
                        ));
                    }
                    Err(StartPasskeyRegistrationError::Unexpected(cause)) => {
                        return Err(cause.into());
                    }
                };

                (
                    CredentialUpdateResult::PasskeyCreationOptions(options),
                    None,
                )
            }
            UpdateCredentialsCommand::RenamePasskey { id, name } => {
                match self.passkeys.rename(ctx, user.id, id, name).await {
                    Ok(_) => (CredentialUpdateResult::Done, None),
                    Err(RenamePasskeyError::NotFound) => {
                        return Err(UpdateCredentialsError::CredentialNotFound);
                    }
                    Err(RenamePasskeyError::Unexpected(cause)) => return Err(cause.into()),
                }
            }
            UpdateCredentialsCommand::Remove(credential) => {
                self.with_credential_change(
                    ctx,
                    actor,
                    kept_session_id,
                    user,
                    async |ctx, before| {
                        match credential {
                            CredentialToRemove::Password => {
                                if !before.has(AuthenticationMethod::Password) {
                                    return Err(UpdateCredentialsError::CredentialNotFound);
                                }
                                self.password.unenroll(ctx, actor, user).await?;
                            }
                            CredentialToRemove::Totp => {
                                if !before.has(AuthenticationMethod::Totp) {
                                    return Err(UpdateCredentialsError::CredentialNotFound);
                                }
                                self.totp.unenroll(ctx, actor, user).await?;
                            }
                            CredentialToRemove::Passkey(id) => {
                                match self.passkeys.remove(ctx, actor, user, id).await {
                                    Ok(_) => {}
                                    Err(RemovePasskeyError::NotFound) => {
                                        return Err(UpdateCredentialsError::CredentialNotFound);
                                    }
                                    Err(RemovePasskeyError::Unexpected(cause)) => {
                                        return Err(cause.into());
                                    }
                                }
                            }
                        }
                        if !self
                            .list(ctx, &Actor::System, user.id)
                            .await
                            .context("could not list the credentials of the user")?
                            .has_first_factor()
                        {
                            return Err(UpdateCredentialsError::LastSignInMethod);
                        }

                        Ok(())
                    },
                )
                .await?;

                (CredentialUpdateResult::Done, None)
            }
            UpdateCredentialsCommand::GenerateRecoveryCodes => {
                let (codes, _) = self
                    .with_credential_change(
                        ctx,
                        actor,
                        kept_session_id,
                        user,
                        async |ctx, before| {
                            if !before.needs_recovery_codes() {
                                return Err(UpdateCredentialsError::CredentialNotFound);
                            }

                            Ok(self.recovery_codes.replace(ctx, actor, user).await?)
                        },
                    )
                    .await?;

                (CredentialUpdateResult::Done, Some(codes))
            }
        };

        Ok(UpdatedCredentials {
            result,
            recovery_codes,
        })
    }

    pub fn first_factors(&self, origin: &url::Origin) -> Vec<AuthenticationMethod> {
        AuthenticationMethod::ALL
            .iter()
            .copied()
            .filter(|&method| {
                method.role() != MethodRole::SecondFactor && method.available_at(origin)
            })
            .collect()
    }

    pub async fn first_step(
        &self,
        ctx: &mut TxContext<Tx>,
        origin: &url::Origin,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Option<AcceptedCredentials>, anyhow::Error> {
        let credentials = match user_id {
            Some(user_id) => Some(self.list(ctx, &Actor::System, user_id).await?),
            None => None,
        };
        let mut accepted = Vec::new();
        for method in self.first_factors(origin) {
            if credentials
                .as_ref()
                .is_some_and(|credentials| !credentials.has(method))
            {
                continue;
            }
            accepted.push(
                self.accepted_credential(ctx, method, origin, user_id)
                    .await?,
            );
        }

        Ok(AcceptedCredentials::new(accepted))
    }

    pub async fn next_step(
        &self,
        ctx: &mut TxContext<Tx>,
        origin: &url::Origin,
        user_id: uuid::Uuid,
        verified_methods: &[AuthenticationMethod],
    ) -> Result<Option<AcceptedCredentials>, anyhow::Error> {
        if verified_methods
            .iter()
            .any(|method| method.role() != MethodRole::FirstFactor)
        {
            return Ok(None);
        }
        let credentials = self.list(ctx, &Actor::System, user_id).await?;
        if !credentials.two_factor_enabled() {
            return Ok(None);
        }

        let mut accepted = Vec::new();
        for &method in AuthenticationMethod::ALL {
            if method.role() != MethodRole::FirstFactor
                && method.available_at(origin)
                && credentials.has(method)
            {
                accepted.push(
                    self.accepted_credential(ctx, method, origin, Some(user_id))
                        .await?,
                );
            }
        }

        AcceptedCredentials::new(accepted)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("user {user_id} has no second factor method"))
    }

    pub async fn verify(
        &self,
        ctx: &mut TxContext<Tx>,
        passkey_challenge: Option<&PasskeyChallenge>,
        user: ClaimedUser,
        credential: &CredentialToVerify,
    ) -> Result<uuid::Uuid, VerifyCredentialError> {
        match (credential, user) {
            (CredentialToVerify::Password(password), ClaimedUser::Known(user_id)) => {
                match self.password.verify(ctx, Some(user_id), password).await {
                    Ok(()) => Ok(user_id),
                    Err(VerifyPasswordError::Unexpected(cause)) => Err(cause.into()),
                    Err(rejection) => Err(VerifyCredentialError::Rejected(rejection.into())),
                }
            }
            (CredentialToVerify::Password(password), ClaimedUser::Nonexistent) => {
                match self.password.verify(ctx, None, password).await {
                    Err(VerifyPasswordError::Unexpected(cause)) => Err(cause.into()),
                    Ok(()) | Err(_) => Err(VerifyCredentialError::Rejected(anyhow::anyhow!(
                        "no user has this username"
                    ))),
                }
            }
            (CredentialToVerify::Totp(code), ClaimedUser::Known(user_id)) => {
                match self.totp.verify(ctx, user_id, code).await {
                    Ok(()) => Ok(user_id),
                    Err(VerifyTotpError::Unexpected(cause)) => Err(cause.into()),
                    Err(rejection) => Err(VerifyCredentialError::Rejected(rejection.into())),
                }
            }
            (CredentialToVerify::RecoveryCode(code), ClaimedUser::Known(user_id)) => {
                match self.recovery_codes.verify(ctx, user_id, code).await {
                    Ok(()) => Ok(user_id),
                    Err(VerifyRecoveryCodeError::Unexpected(cause)) => Err(cause.into()),
                    Err(rejection) => Err(VerifyCredentialError::Rejected(rejection.into())),
                }
            }
            (
                CredentialToVerify::Passkey(response),
                ClaimedUser::Known(_) | ClaimedUser::Unknown,
            ) => {
                let Some(passkey_challenge) = passkey_challenge else {
                    return Err(VerifyCredentialError::Rejected(anyhow::anyhow!(
                        "this step of the sign-in offered no passkey"
                    )));
                };
                let known_user_id = match user {
                    ClaimedUser::Known(user_id) => Some(user_id),
                    ClaimedUser::Nonexistent | ClaimedUser::Unknown => None,
                };

                match self
                    .passkeys
                    .verify(
                        ctx,
                        &passkey_challenge.origin,
                        &passkey_challenge.challenge,
                        response,
                        known_user_id,
                    )
                    .await
                {
                    Ok(user_id) => Ok(user_id),
                    Err(VerifyPasskeyError::Unexpected(cause)) => Err(cause.into()),
                    Err(rejection) => Err(VerifyCredentialError::Rejected(rejection.into())),
                }
            }
            (_, ClaimedUser::Nonexistent) => Err(VerifyCredentialError::Rejected(anyhow::anyhow!(
                "no user has this username"
            ))),
            (_, ClaimedUser::Unknown) => Err(VerifyCredentialError::Rejected(anyhow::anyhow!(
                "the credential does not identify its user"
            ))),
        }
    }

    pub async fn unenroll(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user: &User,
    ) -> Result<(), anyhow::Error> {
        self.with_credential_change(ctx, actor, None, user, async |ctx, _| {
            for &method in AuthenticationMethod::ALL {
                match method {
                    AuthenticationMethod::Password => {
                        self.password.unenroll(ctx, actor, user).await?;
                    }
                    AuthenticationMethod::Totp => self.totp.unenroll(ctx, actor, user).await?,
                    AuthenticationMethod::Passkey => {
                        self.passkeys.unenroll(ctx, actor, user).await?;
                    }
                    AuthenticationMethod::RecoveryCode => {
                        self.recovery_codes.unenroll(ctx, actor, user).await?;
                    }
                }
            }

            Ok(())
        })
        .await
        .map(|((), _)| ())
    }

    async fn accepted_credential(
        &self,
        ctx: &mut TxContext<Tx>,
        method: AuthenticationMethod,
        origin: &url::Origin,
        user_id: Option<uuid::Uuid>,
    ) -> Result<AcceptedCredential, anyhow::Error> {
        Ok(match method {
            AuthenticationMethod::Password => AcceptedCredential::Password,
            AuthenticationMethod::Totp => AcceptedCredential::Totp,
            AuthenticationMethod::RecoveryCode => AcceptedCredential::RecoveryCode,
            AuthenticationMethod::Passkey => {
                let (options, challenge) = self
                    .passkeys
                    .start_authentication(ctx, origin, user_id)
                    .await?;

                AcceptedCredential::Passkey { options, challenge }
            }
        })
    }

    async fn with_credential_change<T, E: From<anyhow::Error>>(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        kept_session_id: Option<uuid::Uuid>,
        user: &User,
        change: impl AsyncFnOnce(&mut TxContext<Tx>, &AccountCredentials) -> Result<T, E>,
    ) -> Result<(T, Option<Vec<SecretString>>), E> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let before = self
                    .list(ctx, &Actor::System, user.id)
                    .await
                    .context("could not list the credentials of the user")?;
                let outcome = change(ctx, &before).await?;
                let after = self
                    .list(ctx, &Actor::System, user.id)
                    .await
                    .context("could not list the credentials of the user")?;

                let recovery_codes =
                    match (before.needs_recovery_codes(), after.needs_recovery_codes()) {
                        (false, true) => Some(self.recovery_codes.replace(ctx, actor, user).await?),
                        (true, false) => {
                            self.recovery_codes.unenroll(ctx, actor, user).await?;

                            None
                        }
                        _ => None,
                    };
                self.sessions
                    .delete_for_user(ctx, user.id, kept_session_id)
                    .await?;

                Ok((outcome, recovery_codes))
            })
            .await
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> CredentialManagement for CredentialService<Tx> {
    async fn list(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountCredentials, ListCredentialsError> {
        Self::list(self, &mut TxContext::none(), actor, user_id).await
    }

    async fn update(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
        cmd: UpdateCredentialsCommand,
    ) -> Result<UpdatedCredentials, UpdateCredentialsError> {
        Self::update(self, &mut TxContext::none(), actor, user_id, cmd).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use secrecy::{ExposeSecret as _, SecretSlice};

    use super::*;
    use crate::application::identity::authentication::methods::passkey::{
        CredentialId, PasskeyBackup, PasskeyCredential,
    };
    use crate::application::identity::authentication::methods::ports::{
        PasskeyCredentialRepository as _, PasswordCredentialRepository as _,
        RecoveryCodeRepository as _, TotpCredentialRepository as _,
        TotpRegistrationRepository as _,
    };
    use crate::application::identity::authentication::methods::recovery_code::RecoveryCodeCredential;
    use crate::application::identity::authentication::methods::totp::{
        TotpRegistration, TotpSecret,
    };
    use crate::application::identity::authentication::ports::SessionRepository as _;
    use crate::application::identity::authentication::sessions::Session;
    use crate::application::identity::authorization::actor::AuthenticatedUser;
    use crate::application::repository::RepositoryError;
    use crate::domain::token_mac::TokenMac;
    use crate::infrastructure::sqlite::SqliteTx;
    use crate::testing::authentication::{
        Installation, NOW, TOTP_SECRET, code_at, origin, passkey_registration,
    };
    use crate::testing::clock::timestamp;
    use crate::testing::identity;

    fn credential_service(installation: &Installation) -> Arc<CredentialService<SqliteTx>> {
        installation.services(NOW).credentials
    }

    #[tokio::test]
    async fn list_of_another_user_as_an_administrator_returns_the_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let administrator = installation.user_with_password("bruno").await;

        let account = credential_service(&installation)
            .list(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap();

        assert_matches!(account.password, Some(_));
        assert_matches!(account.totp, Some(_));
    }

    #[tokio::test]
    async fn list_of_another_user_without_permission_returns_denied() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let listed = credential_service(&installation)
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                user.id,
            )
            .await;

        assert_matches!(
            listed,
            Err(ListCredentialsError::Denied(Denied::MissingPermission(_)))
        );
    }

    #[tokio::test]
    async fn update_finish_totp_registration_with_wrong_code_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let credentials = credential_service(&installation);
        let actor = identity::actor_for(user.clone());
        credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::StartTotpRegistration,
            )
            .await
            .unwrap();

        let confirmed = credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Totp(
                    TotpCode::parse("000000").unwrap(),
                )),
            )
            .await;

        assert_matches!(
            confirmed,
            Err(UpdateCredentialsError::InvalidCredentials(_))
        );
        let totp = installation
            .registry
            .totps
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(totp, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn update_finish_totp_registration_without_a_start_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let confirmed = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Totp(code_at(NOW))),
            )
            .await;

        assert_matches!(
            confirmed,
            Err(UpdateCredentialsError::InvalidCredentials(_))
        );
    }

    #[tokio::test]
    async fn update_finish_totp_registration_after_a_wrong_code_accepts_the_right_code() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation
            .totp_registrations
            .upsert(
                &mut TxContext::<SqliteTx>::none(),
                &TotpRegistration {
                    user_id: user.id,
                    secret: TotpSecret::from(SecretSlice::from(TOTP_SECRET.to_vec())),
                    expires_at: timestamp(NOW + 60),
                },
            )
            .await
            .unwrap();
        let credentials = credential_service(&installation);
        let actor = identity::actor_for(user.clone());
        credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Totp(
                    TotpCode::parse("000000").unwrap(),
                )),
            )
            .await
            .unwrap_err();

        credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Totp(code_at(NOW))),
            )
            .await
            .unwrap();

        let totp = installation
            .registry
            .totps
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(totp, Ok(_));
    }

    #[tokio::test]
    async fn update_finish_passkey_registration_returns_no_recovery_codes() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let credentials = credential_service(&installation);
        let actor = identity::actor_for(user.clone());
        let started = credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::StartPasskeyRegistration { origin: origin() },
            )
            .await
            .unwrap();
        let CredentialUpdateResult::PasskeyCreationOptions(options) = started.result else {
            panic!("a start of a passkey registration gives options");
        };
        let challenge = Challenge(options.challenge.clone().try_into().unwrap());

        let added = credentials
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Passkey {
                    origin: origin(),
                    response: passkey_registration(&origin(), &challenge),
                }),
            )
            .await
            .unwrap();

        assert_matches!(added.result, CredentialUpdateResult::Done);
        assert_matches!(added.recovery_codes, None);
    }

    #[tokio::test]
    async fn update_start_passkey_registration_at_an_ip_address_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let started = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::StartPasskeyRegistration {
                    origin: url::Url::parse("https://10.0.0.5:8443").unwrap().origin(),
                },
            )
            .await;

        assert_matches!(started, Err(UpdateCredentialsError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn update_start_passkey_registration_with_ten_passkeys_returns_limit_reached() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        for credential_byte in 0..10 {
            installation
                .registry
                .passkeys
                .create(
                    &mut TxContext::none(),
                    &PasskeyCredential {
                        id: uuid::Uuid::now_v7(),
                        user_id: user.id,
                        credential_id: CredentialId(vec![credential_byte; 16]),
                        public_key: vec![1; 33],
                        name: PasskeyName::parse("Laptop").unwrap(),
                        sign_count: 0,
                        backup: PasskeyBackup::default(),
                        created_at: jiff::Timestamp::UNIX_EPOCH,
                        last_used_at: None,
                    },
                )
                .await
                .unwrap();
        }

        let started = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::StartPasskeyRegistration { origin: origin() },
            )
            .await;

        assert_matches!(
            started,
            Err(UpdateCredentialsError::LimitReached(
                AuthenticationMethod::Passkey
            ))
        );
    }

    #[tokio::test]
    async fn update_finish_passkey_registration_without_a_start_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let added = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Passkey {
                    origin: origin(),
                    response: passkey_registration(&origin(), &Challenge::generate()),
                }),
            )
            .await;

        assert_matches!(added, Err(UpdateCredentialsError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn update_rename_passkey_gives_the_passkey_the_name() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let passkey = installation.add_passkey(&user).await;

        credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::RenamePasskey {
                    id: passkey.id,
                    name: PasskeyName::parse("Work laptop").unwrap(),
                },
            )
            .await
            .unwrap();

        let passkeys = installation
            .registry
            .passkeys
            .list_for_user(&mut TxContext::none(), user.id)
            .await
            .unwrap();
        assert_eq!(&*passkeys[0].name, "Work laptop");
    }

    #[tokio::test]
    async fn update_rename_passkey_of_another_user_returns_not_found() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let other_user = installation.user_with_password("bruno").await;
        let passkey = installation.add_passkey(&other_user).await;

        let renamed = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::RenamePasskey {
                    id: passkey.id,
                    name: PasskeyName::parse("Mine now").unwrap(),
                },
            )
            .await;

        assert_matches!(renamed, Err(UpdateCredentialsError::CredentialNotFound));
    }

    #[tokio::test]
    async fn update_remove_passkey_of_another_user_returns_not_found() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let other_user = installation.user_with_password("bruno").await;
        let passkey = installation.add_passkey(&other_user).await;

        let removed = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Passkey(passkey.id)),
            )
            .await;

        assert_matches!(removed, Err(UpdateCredentialsError::CredentialNotFound));
    }

    #[tokio::test]
    async fn update_remove_passkey_as_the_last_way_to_sign_in_returns_last_sign_in_method() {
        let installation = Installation::new().await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let passkey = installation.add_passkey(&user).await;

        let removed = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Passkey(passkey.id)),
            )
            .await;

        assert_matches!(removed, Err(UpdateCredentialsError::LastSignInMethod));
    }

    #[tokio::test]
    async fn update_remove_totp_as_the_last_second_factor_removes_the_recovery_codes() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_recovery_code(&user).await;

        credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Totp),
            )
            .await
            .unwrap();

        let recovery_codes = installation
            .registry
            .recovery_codes
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(recovery_codes, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn update_remove_totp_after_the_last_recovery_code_removes_the_empty_codes() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation
            .registry
            .recovery_codes
            .upsert(
                &mut TxContext::none(),
                &RecoveryCodeCredential {
                    user_id: user.id,
                    code_macs: Vec::new(),
                    generated_at: timestamp(0),
                },
            )
            .await
            .unwrap();

        credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Totp),
            )
            .await
            .unwrap();

        let recovery_codes = installation
            .registry
            .recovery_codes
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(recovery_codes, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn update_generate_recovery_codes_without_two_factor_returns_credential_not_found() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let generated = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::GenerateRecoveryCodes,
            )
            .await;

        assert_matches!(generated, Err(UpdateCredentialsError::CredentialNotFound));
    }

    #[tokio::test]
    async fn update_remove_password_with_a_passkey_removes_the_password() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;

        credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Password),
            )
            .await
            .unwrap();

        let password = installation
            .registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(password, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn update_remove_password_as_the_last_way_to_sign_in_returns_last_sign_in_method() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let removed = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Password),
            )
            .await;

        assert_matches!(removed, Err(UpdateCredentialsError::LastSignInMethod));
    }

    #[tokio::test]
    async fn update_change_password_ends_the_other_sessions_and_keeps_the_current_one() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let current = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let other = installation
            .registry
            .create_session(&Session {
                id: uuid::Uuid::now_v7(),
                mac: TokenMac::from(vec![2; 32]),
                ..identity::session_for(&user)
            })
            .await;

        credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &Actor::User(AuthenticatedUser {
                    session_id: current.id,
                    recently_authenticated_until: jiff::Timestamp::MAX,
                    user: user.clone(),
                }),
                user.id,
                UpdateCredentialsCommand::Add(CredentialToAdd::Password(identity::password())),
            )
            .await
            .unwrap();

        let other_lookup = installation
            .registry
            .sessions
            .get_by_id(&mut TxContext::none(), other.id)
            .await;
        let current_lookup = installation
            .registry
            .sessions
            .get_by_id(&mut TxContext::none(), current.id)
            .await;
        assert_matches!(other_lookup, Err(RepositoryError::NotFound));
        assert_eq!(current_lookup.unwrap(), current);
    }

    #[tokio::test]
    async fn update_of_another_user_returns_denied() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let administrator = installation.user_with_password("bruno").await;

        let updated = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
                UpdateCredentialsCommand::Remove(CredentialToRemove::Password),
            )
            .await;

        assert_matches!(
            updated,
            Err(UpdateCredentialsError::Denied(Denied::NotAllowed))
        );
    }

    #[tokio::test]
    async fn update_without_a_recent_proof_returns_recent_authentication_required() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let actor = Actor::User(AuthenticatedUser {
            session_id: uuid::Uuid::now_v7(),
            recently_authenticated_until: timestamp(NOW),
            user: user.clone(),
        });

        let updated = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &actor,
                user.id,
                UpdateCredentialsCommand::StartTotpRegistration,
            )
            .await;

        assert_matches!(
            updated,
            Err(UpdateCredentialsError::Denied(
                Denied::RecentAuthenticationRequired
            ))
        );
    }

    #[tokio::test]
    async fn update_start_totp_registration_names_the_user_in_the_app() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;

        let started = credential_service(&installation)
            .update(
                &mut TxContext::none(),
                &identity::actor_for(user.clone()),
                user.id,
                UpdateCredentialsCommand::StartTotpRegistration,
            )
            .await
            .unwrap();

        assert_matches!(
            started.result,
            CredentialUpdateResult::TotpRegistrationStarted(setup)
                if setup
                    .key_uri
                    .expose_secret()
                    .starts_with("otpauth://totp/Paron%3Aalice%40paron%2Eexample?")
        );
    }
}
