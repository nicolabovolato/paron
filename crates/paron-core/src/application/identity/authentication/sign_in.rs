use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use secrecy::SecretString;

use super::credentials::{
    AcceptedCredential, AcceptedCredentials, ClaimedUser, CredentialService, CredentialToVerify,
    PasskeyChallenge, VerifyCredentialError,
};
use super::device_token::DeviceToken;
use super::failed_sign_in_attempts::{
    AttemptKey, FailedSignInAttempts, FailureDelay, ThrottledMethod,
};
use super::methods::AuthenticationMethod;
use super::methods::passkey::Challenge;
use super::ports::{FailedSignInAttemptRepository, PendingSignInRepository};
use super::sessions::{SessionError, SessionService};
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::{Actor, AuthenticatedUser};
use crate::application::identity::errors::{
    AuthenticateError, ContinueSignInError, GetUserError, SignOutError, StartSignInError,
};
use crate::application::identity::event::AuditKind;
use crate::application::identity::use_cases::Authentication;
use crate::application::identity::users::UserService;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;
use crate::domain::user::Username;

pub enum StartSignInCommand {
    NewSession {
        origin: url::Origin,
    },
    Reauthentication {
        origin: url::Origin,
        user: AuthenticatedUser,
    },
}

#[cfg_attr(test, derive(Debug))]
pub struct SignInStep {
    pub token: SecretString,
    pub expires_at: jiff::Timestamp,
    pub accepted: AcceptedCredentials,
}

pub struct ContinueSignInCommand {
    pub token: SecretString,
    pub device_token: Option<SecretString>,
    /// Names the user of the credential. A credential that identifies its user alone has none.
    pub username: Option<Username>,
    pub credential: CredentialToVerify,
}

#[cfg_attr(test, derive(Debug))]
pub enum SignInOutcome {
    NextStep(SignInStep),
    SignedIn {
        token: SecretString,
        expires_at: jiff::Timestamp,
        device_token: SecretString,
    },
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct PendingSignIn {
    pub mac: TokenMac,
    pub origin: url::Origin,
    pub purpose: SignInPurpose,
    pub accepted_methods: Vec<AuthenticationMethod>,
    pub challenge: Option<Challenge>,
    pub expires_at: jiff::Timestamp,
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub enum SignInPurpose {
    NewSession(Option<SignInUser>),
    Reauthentication {
        session_id: uuid::Uuid,
        user: SignInUser,
    },
}

impl SignInPurpose {
    pub fn user(&self) -> Option<&SignInUser> {
        match self {
            Self::NewSession(user) => user.as_ref(),
            Self::Reauthentication { user, .. } => Some(user),
        }
    }
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct SignInUser {
    pub user_id: uuid::Uuid,
    pub verified_methods: Vec<AuthenticationMethod>,
}

#[derive(Debug, Clone, Copy)]
pub struct SignInPolicy {
    pub lifetime: Duration,
    pub password_failure_delay: FailureDelay,
    pub totp_failure_delay: FailureDelay,
    pub recovery_code_failure_delay: FailureDelay,
}

impl SignInPolicy {
    pub fn failure_delay_for(&self, method: ThrottledMethod) -> &FailureDelay {
        match method {
            ThrottledMethod::Password => &self.password_failure_delay,
            ThrottledMethod::Totp => &self.totp_failure_delay,
            ThrottledMethod::RecoveryCode => &self.recovery_code_failure_delay,
        }
    }
}

pub struct AuthenticationDeps<Tx> {
    pub users: Arc<UserService<Tx>>,
    pub sessions: Arc<SessionService<Tx>>,
    pub credentials: Arc<CredentialService<Tx>>,
    pub pending_sign_ins: Arc<dyn PendingSignInRepository<Tx>>,
    pub sign_in_tokens: Arc<dyn Tokens>,
    pub device_tokens: Arc<dyn Tokens>,
    pub failed_sign_in_attempts: Arc<dyn FailedSignInAttemptRepository<Tx>>,
    pub unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    pub audit: Arc<AuditService<Tx>>,
    pub clock: Arc<dyn Clock>,
    pub policy: SignInPolicy,
}

pub struct AuthenticationService<Tx> {
    deps: AuthenticationDeps<Tx>,
}

impl<Tx: Send + Sync + 'static> AuthenticationService<Tx> {
    pub fn new(deps: AuthenticationDeps<Tx>) -> Self {
        Self { deps }
    }

    pub async fn sign_out(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
    ) -> Result<(), SignOutError> {
        let Some(authenticated) = actor.user() else {
            return Ok(());
        };

        self.deps
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                match self
                    .deps
                    .sessions
                    .delete(ctx, authenticated.session_id)
                    .await
                {
                    Ok(_) => {}
                    Err(SessionError::Ended) => return Ok(()),
                    Err(SessionError::Unexpected(cause)) => return Err(cause.into()),
                }
                self.deps
                    .audit
                    .append(
                        ctx,
                        actor,
                        &authenticated.user,
                        AuditKind::SignedOut {
                            session_id: authenticated.session_id,
                        },
                    )
                    .await?;

                Ok(())
            })
            .await
    }

    pub async fn authenticate(
        &self,
        ctx: &mut TxContext<Tx>,
        session_token: &SecretString,
    ) -> Result<AuthenticatedUser, AuthenticateError> {
        let session = match self.deps.sessions.get_by_token(ctx, session_token).await {
            Ok(session) => session,
            Err(SessionError::Ended) => return Err(AuthenticateError::SessionEnded),
            Err(SessionError::Unexpected(cause)) => return Err(cause.into()),
        };

        let user = match self
            .deps
            .users
            .get_by_id(ctx, &Actor::System, session.user_id)
            .await
        {
            Ok(user) => user,
            Err(GetUserError::NotFound) => return Err(AuthenticateError::SessionEnded),
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("could not get user by id")
                    .into());
            }
        };

        if !user.is_active() {
            return Err(AuthenticateError::SessionEnded);
        }

        match self.deps.sessions.touch(ctx, session.id).await {
            Ok(()) => {}
            Err(SessionError::Ended) => return Err(AuthenticateError::SessionEnded),
            Err(SessionError::Unexpected(cause)) => return Err(cause.into()),
        }

        Ok(AuthenticatedUser {
            session_id: session.id,
            recently_authenticated_until: session.recently_authenticated_until,
            user,
        })
    }

    pub async fn delete_expired_sign_ins(
        &self,
        ctx: &mut TxContext<Tx>,
    ) -> Result<(), anyhow::Error> {
        self.deps
            .pending_sign_ins
            .delete_expired(ctx, self.deps.clock.now())
            .await
            .context("could not delete the expired sign-ins")?;

        Ok(())
    }

    pub async fn delete_expired_sign_in_attempts(
        &self,
        ctx: &mut TxContext<Tx>,
    ) -> Result<(), anyhow::Error> {
        self.deps
            .failed_sign_in_attempts
            .delete_expired(ctx, self.deps.clock.now())
            .await
            .context("could not delete the expired attempts")?;

        Ok(())
    }

    pub async fn start_sign_in(
        &self,
        ctx: &mut TxContext<Tx>,
        cmd: StartSignInCommand,
    ) -> Result<SignInStep, StartSignInError> {
        let (origin, purpose) = match cmd {
            StartSignInCommand::NewSession { origin } => (origin, SignInPurpose::NewSession(None)),
            StartSignInCommand::Reauthentication { origin, user } => (
                origin,
                SignInPurpose::Reauthentication {
                    session_id: user.session_id,
                    user: SignInUser {
                        user_id: user.user.id,
                        verified_methods: Vec::new(),
                    },
                },
            ),
        };
        let now = self.deps.clock.now();
        let user_id = purpose.user().map(|user| user.user_id);
        let accepted = self
            .deps
            .credentials
            .first_step(ctx, &origin, user_id)
            .await?
            .ok_or(StartSignInError::NoMethodAtOrigin)?;

        Ok(self
            .store_step(
                ctx,
                origin,
                purpose,
                accepted,
                now + self.deps.policy.lifetime,
            )
            .await?)
    }

    pub async fn continue_sign_in(
        &self,
        ctx: &mut TxContext<Tx>,
        cmd: ContinueSignInCommand,
    ) -> Result<SignInOutcome, ContinueSignInError> {
        let now = self.deps.clock.now();
        let mac = self.deps.sign_in_tokens.mac(&cmd.token);
        let method = cmd.credential.method();

        let pending_sign_in = match self.deps.pending_sign_ins.get_by_mac(ctx, &mac).await {
            Ok(pending_sign_in) if now < pending_sign_in.expires_at => pending_sign_in,
            Ok(_) | Err(RepositoryError::NotFound) => {
                return Err(ContinueSignInError::SignInEnded);
            }
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("could not get the pending sign-in")
                    .into());
            }
        };
        if !pending_sign_in.accepted_methods.contains(&method) {
            return Err(ContinueSignInError::InvalidCredentials(anyhow::anyhow!(
                "this step of the sign-in did not offer {method}"
            )));
        }

        let user_id = self
            .verify_credential(
                ctx,
                pending_sign_in
                    .challenge
                    .clone()
                    .map(|challenge| PasskeyChallenge {
                        origin: pending_sign_in.origin.clone(),
                        challenge,
                    })
                    .as_ref(),
                pending_sign_in.purpose.user().map(|user| user.user_id),
                &cmd,
            )
            .await?;

        let verified_methods: Vec<_> = pending_sign_in
            .purpose
            .user()
            .into_iter()
            .flat_map(|known_user| known_user.verified_methods.iter().copied())
            .chain([method])
            .collect();

        self.deps
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                match self.deps.pending_sign_ins.delete_by_mac(ctx, &mac).await {
                    Ok(_) => {}
                    Err(RepositoryError::NotFound) => {
                        return Err(ContinueSignInError::SignInEnded);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not delete the pending sign-in")
                            .into());
                    }
                }

                let next_step = self
                    .deps
                    .credentials
                    .next_step(ctx, &pending_sign_in.origin, user_id, &verified_methods)
                    .await?;

                match next_step {
                    Some(accepted) => {
                        let sign_in_user = SignInUser {
                            user_id,
                            verified_methods,
                        };

                        Ok(SignInOutcome::NextStep(
                            self.store_step(
                                ctx,
                                pending_sign_in.origin,
                                match pending_sign_in.purpose {
                                    SignInPurpose::NewSession(_) => {
                                        SignInPurpose::NewSession(Some(sign_in_user))
                                    }
                                    SignInPurpose::Reauthentication { session_id, .. } => {
                                        SignInPurpose::Reauthentication {
                                            session_id,
                                            user: sign_in_user,
                                        }
                                    }
                                },
                                accepted,
                                pending_sign_in.expires_at,
                            )
                            .await?,
                        ))
                    }
                    None => {
                        self.finish_sign_in(ctx, pending_sign_in.purpose, user_id, verified_methods)
                            .await
                    }
                }
            })
            .await
    }

    async fn verify_credential(
        &self,
        ctx: &mut TxContext<Tx>,
        passkey_challenge: Option<&PasskeyChallenge>,
        known_user_id: Option<uuid::Uuid>,
        cmd: &ContinueSignInCommand,
    ) -> Result<uuid::Uuid, ContinueSignInError> {
        let method = cmd.credential.method();
        let user = match (&cmd.username, known_user_id) {
            (None, None) => {
                if ThrottledMethod::try_from(method).is_ok() {
                    return Err(ContinueSignInError::InvalidCredentials(anyhow::anyhow!(
                        "the method {method} can be guessed, and this sign-in specifies no user"
                    )));
                }

                ClaimedUser::Unknown
            }
            (None, Some(user_id)) => ClaimedUser::Known(user_id),
            (Some(username), _) => {
                let user = match self
                    .deps
                    .users
                    .get_by_username(ctx, &Actor::System, username)
                    .await
                {
                    Ok(user) => ClaimedUser::Known(user.id),
                    Err(GetUserError::NotFound) => ClaimedUser::Nonexistent,
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get user by username")
                            .into());
                    }
                };
                if known_user_id.is_some_and(|user_id| user != ClaimedUser::Known(user_id)) {
                    return Err(ContinueSignInError::InvalidCredentials(anyhow::anyhow!(
                        "the username is not the user of this sign-in"
                    )));
                }

                user
            }
        };

        let verify = async |ctx: &mut TxContext<Tx>| match self
            .deps
            .credentials
            .verify(ctx, passkey_challenge, user, &cmd.credential)
            .await
        {
            Ok(user_id) => Ok(user_id),
            Err(VerifyCredentialError::Rejected(cause)) => {
                Err(ContinueSignInError::InvalidCredentials(cause))
            }
            Err(VerifyCredentialError::Unexpected(cause)) => {
                Err(ContinueSignInError::Unexpected(cause))
            }
        };

        match (ThrottledMethod::try_from(method), user) {
            (Ok(throttled_method), ClaimedUser::Known(user_id)) => {
                self.throttled(
                    ctx,
                    throttled_method,
                    user_id,
                    cmd.device_token.as_ref(),
                    verify,
                )
                .await
            }
            _ => verify(ctx).await,
        }
    }

    async fn store_step(
        &self,
        ctx: &mut TxContext<Tx>,
        origin: url::Origin,
        purpose: SignInPurpose,
        accepted: AcceptedCredentials,
        expires_at: jiff::Timestamp,
    ) -> Result<SignInStep, anyhow::Error> {
        let (token, mac) = self.deps.sign_in_tokens.issue();
        let pending_sign_in = self
            .deps
            .pending_sign_ins
            .create(
                ctx,
                &PendingSignIn {
                    mac,
                    origin,
                    purpose,
                    accepted_methods: accepted
                        .as_slice()
                        .iter()
                        .map(AcceptedCredential::method)
                        .collect(),
                    challenge: accepted
                        .as_slice()
                        .iter()
                        .find_map(AcceptedCredential::challenge)
                        .cloned(),
                    expires_at,
                },
            )
            .await
            .context("could not create the pending sign-in")?;

        Ok(SignInStep {
            token,
            expires_at: pending_sign_in.expires_at,
            accepted,
        })
    }

    async fn finish_sign_in(
        &self,
        ctx: &mut TxContext<Tx>,
        purpose: SignInPurpose,
        user_id: uuid::Uuid,
        verified_methods: Vec<AuthenticationMethod>,
    ) -> Result<SignInOutcome, ContinueSignInError> {
        let user = match self
            .deps
            .users
            .get_by_id(ctx, &Actor::System, user_id)
            .await
        {
            Ok(user) => user,
            Err(GetUserError::NotFound) => return Err(ContinueSignInError::SignInEnded),
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("could not get user by id")
                    .into());
            }
        };
        if !user.is_active() {
            return Err(ContinueSignInError::Suspended);
        }

        let device_token = DeviceToken::issue(self.deps.device_tokens.as_ref(), user.id).encode();
        let (token, session) = match purpose {
            SignInPurpose::NewSession(_) => {
                self.deps
                    .unit_of_work
                    .transaction(ctx, ReadModifyWrite, async |ctx| {
                        let (token, session) = self.deps.sessions.create(ctx, user.id).await?;
                        let authenticated = AuthenticatedUser {
                            session_id: session.id,
                            recently_authenticated_until: session.recently_authenticated_until,
                            user,
                        };
                        self.deps
                            .audit
                            .append(
                                ctx,
                                &Actor::User(authenticated.clone()),
                                &authenticated.user,
                                AuditKind::SignedIn {
                                    session_id: session.id,
                                    methods: verified_methods,
                                },
                            )
                            .await?;

                        Ok::<_, anyhow::Error>((token, session))
                    })
                    .await?
            }
            SignInPurpose::Reauthentication { session_id, .. } => {
                match self.deps.sessions.renew(ctx, session_id).await {
                    Ok(renewed) => renewed,
                    Err(SessionError::Ended) => return Err(ContinueSignInError::SignInEnded),
                    Err(SessionError::Unexpected(cause)) => return Err(cause.into()),
                }
            }
        };

        Ok(SignInOutcome::SignedIn {
            token,
            expires_at: session.expires_at,
            device_token,
        })
    }

    async fn throttled<T>(
        &self,
        ctx: &mut TxContext<Tx>,
        method: ThrottledMethod,
        user_id: uuid::Uuid,
        device_token: Option<&SecretString>,
        action: impl AsyncFnOnce(&mut TxContext<Tx>) -> Result<T, ContinueSignInError>,
    ) -> Result<T, ContinueSignInError> {
        let key = AttemptKey {
            method,
            user_id,
            device_id: device_token
                .and_then(|token| {
                    DeviceToken::verify(self.deps.device_tokens.as_ref(), token, user_id)
                })
                .map(|device_token| device_token.device_id),
        };
        let delay = self.deps.policy.failure_delay_for(key.method);
        let now = self.deps.clock.now();

        if ctx.transaction().is_some() {
            return Err(anyhow::anyhow!("throttled was called inside a transaction").into());
        }

        self.deps
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let previous = match self.deps.failed_sign_in_attempts.get(ctx, &key).await {
                    Ok(attempts) if now < attempts.expires_at => Some(attempts),
                    Ok(_) | Err(RepositoryError::NotFound) => None,
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get the failed attempts")
                            .into());
                    }
                };
                if let Some(attempts) = &previous
                    && now < attempts.next_attempt_at
                {
                    return Err(ContinueSignInError::TooManyAttempts {
                        retry_after: attempts.next_attempt_at.duration_since(now).unsigned_abs(),
                    });
                }

                let count = previous.map_or(0, |attempts| attempts.count) + 1;
                self.deps
                    .failed_sign_in_attempts
                    .upsert(
                        ctx,
                        &FailedSignInAttempts {
                            key,
                            count,
                            next_attempt_at: now + delay.delay(count),
                            expires_at: now + delay.forget_after,
                        },
                    )
                    .await
                    .context("could not count the attempt")?;

                Ok(())
            })
            .await?;

        let result = action(ctx).await;
        if result.is_ok() {
            match self.deps.failed_sign_in_attempts.delete(ctx, &key).await {
                Ok(_) | Err(RepositoryError::NotFound) => {}
                Err(error) => {
                    return Err(anyhow::Error::new(error)
                        .context("could not clear the failed attempts")
                        .into());
                }
            }
        }

        result
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> Authentication for AuthenticationService<Tx> {
    async fn start_sign_in(&self, cmd: StartSignInCommand) -> Result<SignInStep, StartSignInError> {
        Self::start_sign_in(self, &mut TxContext::none(), cmd).await
    }

    async fn continue_sign_in(
        &self,
        cmd: ContinueSignInCommand,
    ) -> Result<SignInOutcome, ContinueSignInError> {
        Self::continue_sign_in(self, &mut TxContext::none(), cmd).await
    }

    async fn sign_out(&self, actor: &Actor) -> Result<(), SignOutError> {
        Self::sign_out(self, &mut TxContext::none(), actor).await
    }

    async fn authenticate(
        &self,
        session_token: &SecretString,
    ) -> Result<AuthenticatedUser, AuthenticateError> {
        Self::authenticate(self, &mut TxContext::none(), session_token).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::audit;
    use crate::application::identity::authentication::methods::passkey::{
        Challenge, CredentialId, PasskeyBackup, PasskeyCredential, PasskeyName,
    };
    use crate::application::identity::authentication::methods::ports::PasskeyCredentialRepository as _;
    use crate::application::identity::authentication::methods::recovery_code::RecoveryCode;
    use crate::application::identity::authentication::methods::totp::TotpCode;
    use crate::application::identity::authentication::ports::SessionRepository as _;
    use crate::application::identity::authentication::sessions::Session;
    use crate::application::identity::ports::UserRepository as _;
    use crate::application::repository::RepositoryError;
    use crate::domain::user::{User, UserStatus};
    use crate::infrastructure::crypto::tokens::Hmac;
    use crate::infrastructure::sqlite::SqliteTx;
    use crate::testing::authentication::{
        CREDENTIAL_ID, Installation, NOW, RECOVERY_CODE, code_at, origin, passkey_assertion,
        wrong_password,
    };
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::keys::symmetric_key;

    const SIGN_IN_POLICY: SignInPolicy = SignInPolicy {
        lifetime: Duration::from_mins(10),
        password_failure_delay: FailureDelay {
            free_failures: 5,
            first_delay: Duration::from_secs(1),
            max_delay: Duration::from_mins(15),
            forget_after: Duration::from_hours(12),
        },
        totp_failure_delay: FailureDelay {
            free_failures: 3,
            first_delay: Duration::from_mins(1),
            max_delay: Duration::from_mins(1),
            forget_after: Duration::from_mins(1),
        },
        recovery_code_failure_delay: FailureDelay {
            free_failures: 5,
            first_delay: Duration::from_secs(1),
            max_delay: Duration::from_mins(15),
            forget_after: Duration::from_hours(12),
        },
    };

    fn sign_in_service(
        installation: &Installation,
        seconds: i64,
    ) -> AuthenticationService<SqliteTx> {
        let services = installation.services(seconds);

        AuthenticationService::new(AuthenticationDeps {
            users: services.users,
            sessions: services.sessions,
            credentials: services.credentials,
            pending_sign_ins: Arc::clone(&installation.registry.pending_sign_ins) as Arc<_>,
            sign_in_tokens: Arc::new(sign_in_tokens()),
            device_tokens: Arc::new(Hmac::new(symmetric_key([5; 32]))),
            failed_sign_in_attempts: Arc::clone(&installation.registry.failed_sign_in_attempts)
                as Arc<_>,
            unit_of_work: services.unit_of_work as Arc<_>,
            audit: services.audit,
            clock: services.clock,
            policy: SIGN_IN_POLICY,
        })
    }

    async fn new_sign_in_token(
        service: &AuthenticationService<SqliteTx>,
        origin: url::Origin,
    ) -> SecretString {
        service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::NewSession { origin },
            )
            .await
            .unwrap()
            .token
    }

    fn signed_in_methods(installation: &Installation) -> Vec<Vec<AuthenticationMethod>> {
        installation
            .audit
            .recorded_kinds()
            .into_iter()
            .filter_map(|kind| match kind {
                audit::AuditKind::User(AuditKind::SignedIn { methods, .. }) => Some(methods),
                _ => None,
            })
            .collect()
    }

    fn sign_in_tokens() -> Hmac {
        Hmac::new(symmetric_key([4; 32]))
    }

    /// The origin and the passkey challenge of the step of `sign_in_token`. A step that offered no
    /// passkey has no challenge, so this gives a new one.
    async fn step_challenge(
        installation: &Installation,
        sign_in_token: &SecretString,
    ) -> (url::Origin, Challenge) {
        let pending_sign_in = installation
            .registry
            .pending_sign_ins
            .get_by_mac(&mut TxContext::none(), &sign_in_tokens().mac(sign_in_token))
            .await
            .unwrap();
        let challenge = pending_sign_in
            .challenge
            .unwrap_or_else(Challenge::generate);

        (pending_sign_in.origin, challenge)
    }

    /// The test passkey of `user`, signed with user verification over the challenge of the step.
    async fn passkey_response(
        installation: &Installation,
        user: &User,
        sign_in_token: &SecretString,
    ) -> CredentialToVerify {
        let (origin, challenge) = step_challenge(installation, sign_in_token).await;

        CredentialToVerify::Passkey(passkey_assertion(user, &origin, &challenge, true))
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_method_that_the_step_did_not_offer_returns_invalid_credentials()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
        assert_eq!(
            signed_in_methods(&installation),
            Vec::<Vec<AuthenticationMethod>>::new()
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_password_without_a_username_returns_invalid_credentials() {
        let installation = Installation::new().await;
        installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_password_without_second_factor_returns_a_session_from_the_password()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();

        let SignInOutcome::SignedIn { token, .. } = outcome else {
            panic!("{outcome:?}");
        };
        let authenticated = service
            .authenticate(&mut TxContext::none(), &token)
            .await
            .unwrap();
        assert_eq!(authenticated.user.id, user.id);
        assert_eq!(
            signed_in_methods(&installation),
            [[AuthenticationMethod::Password]]
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_password_of_a_user_with_only_a_passkey_returns_a_session() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();

        assert_matches!(outcome, SignInOutcome::SignedIn { .. });
    }

    #[tokio::test]
    async fn continue_sign_in_with_wrong_password_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username),
                    credential: CredentialToVerify::Password(wrong_password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_code_before_the_password_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_the_token_of_a_finished_sign_in_returns_sign_in_ended() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token.clone(),
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::SignInEnded));
    }

    #[tokio::test]
    async fn continue_sign_in_with_password_of_a_user_with_totp_asks_for_the_second_factor() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();

        assert_matches!(
            outcome,
            SignInOutcome::NextStep(SignInStep { accepted, .. })
                if matches!(accepted.as_slice(), [AcceptedCredential::Totp])
        );
        assert_eq!(
            signed_in_methods(&installation),
            Vec::<Vec<AuthenticationMethod>>::new()
        );
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_suspended_user_returns_suspended_only_after_the_last_step() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation
            .registry
            .users
            .update(
                &mut TxContext::none(),
                &User {
                    status: UserStatus::Suspended,
                    ..user.clone()
                },
            )
            .await
            .unwrap();
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let after_password = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();
        let SignInOutcome::NextStep(SignInStep {
            token: sign_in_token,
            ..
        }) = after_password
        else {
            panic!("{after_password:?}");
        };

        let after_code = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await;

        assert_matches!(after_code, Err(ContinueSignInError::Suspended));
    }

    #[tokio::test]
    async fn continue_sign_in_with_the_correct_code_returns_a_session_from_both_methods() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let SignInOutcome::NextStep(SignInStep {
            token: sign_in_token,
            ..
        }) = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap()
        else {
            panic!("a user with a second factor gives it after the password");
        };

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await
            .unwrap();

        let SignInOutcome::SignedIn { token, .. } = outcome else {
            panic!("{outcome:?}");
        };
        let authenticated = service
            .authenticate(&mut TxContext::none(), &token)
            .await
            .unwrap();
        assert_eq!(authenticated.user.id, user.id);
        assert_eq!(
            signed_in_methods(&installation),
            [[AuthenticationMethod::Password, AuthenticationMethod::Totp]]
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_code_used_before_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let mut outcomes = Vec::new();

        for _ in 0..2 {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            let SignInOutcome::NextStep(SignInStep {
                token: sign_in_token,
                ..
            }) = service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(user.username.clone()),
                        credential: CredentialToVerify::Password(identity::password()),
                    },
                )
                .await
                .unwrap()
            else {
                panic!("a user with a second factor gives it after the password");
            };
            outcomes.push(
                service
                    .continue_sign_in(
                        &mut TxContext::none(),
                        ContinueSignInCommand {
                            token: sign_in_token,
                            device_token: None,
                            username: None,
                            credential: CredentialToVerify::Totp(code_at(NOW)),
                        },
                    )
                    .await,
            );
        }

        assert_matches!(outcomes[0], Ok(SignInOutcome::SignedIn { .. }));
        assert_matches!(outcomes[1], Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_after_the_sign_in_lifetime_returns_sign_in_ended() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let SignInOutcome::NextStep(SignInStep {
            token: sign_in_token,
            ..
        }) = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap()
        else {
            panic!("a user with a second factor gives it after the password");
        };
        let later = NOW + i64::try_from(SIGN_IN_POLICY.lifetime.as_secs()).unwrap();

        let outcome = sign_in_service(&installation, later)
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(later)),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::SignInEnded));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_recovery_code_works_one_time() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_recovery_code(&user).await;
        let service = sign_in_service(&installation, NOW);
        let mut outcomes = Vec::new();

        for _ in 0..2 {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            let SignInOutcome::NextStep(SignInStep {
                token: sign_in_token,
                ..
            }) = service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(user.username.clone()),
                        credential: CredentialToVerify::Password(identity::password()),
                    },
                )
                .await
                .unwrap()
            else {
                panic!("a user with a second factor gives it after the password");
            };
            outcomes.push(
                service
                    .continue_sign_in(
                        &mut TxContext::none(),
                        ContinueSignInCommand {
                            token: sign_in_token,
                            device_token: None,
                            username: None,
                            credential: CredentialToVerify::RecoveryCode(
                                RecoveryCode::parse(RECOVERY_CODE).unwrap(),
                            ),
                        },
                    )
                    .await,
            );
        }

        assert_matches!(outcomes[0], Ok(SignInOutcome::SignedIn { .. }));
        assert_matches!(outcomes[1], Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_passkey_alone_returns_a_session_from_the_passkey() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await
            .unwrap();

        let SignInOutcome::SignedIn { token, .. } = outcome else {
            panic!("{outcome:?}");
        };
        let authenticated = service
            .authenticate(&mut TxContext::none(), &token)
            .await
            .unwrap();
        assert_eq!(authenticated.user.id, user.id);
        assert_eq!(
            signed_in_methods(&installation),
            [[AuthenticationMethod::Passkey]]
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_passkey_without_user_handle_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let CredentialToVerify::Passkey(mut response) =
            passkey_response(&installation, &user, &sign_in_token).await
        else {
            unreachable!()
        };
        response.response.user_handle = None;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Passkey(response),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_passkey_whose_user_handle_names_another_user_returns_invalid_credentials()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let other_user = installation.user_with_password("bob").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let credential = passkey_response(&installation, &other_user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_with_a_passkey_without_user_handle_reauthenticates()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let session = installation
            .registry
            .create_session(&Session {
                last_used_at: timestamp(NOW),
                expires_at: timestamp(NOW + 3600),
                ..identity::session_for(&user)
            })
            .await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_reauthentication_token(&service, &user, &session).await;
        let CredentialToVerify::Passkey(mut response) =
            passkey_response(&installation, &user, &sign_in_token).await
        else {
            unreachable!()
        };
        response.response.user_handle = None;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Passkey(response),
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_passkey_in_a_step_that_offered_none_returns_invalid_credentials()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(
            &service,
            url::Url::parse("https://10.0.0.5:8443").unwrap().origin(),
        )
        .await;

        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_wrong_passkey_signature_returns_invalid_credentials() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let CredentialToVerify::Passkey(mut response) =
            passkey_response(&installation, &user, &sign_in_token).await
        else {
            unreachable!()
        };
        response.response.signature[0] ^= 1;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Passkey(response),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_passkey_without_user_verification_returns_invalid_credentials()
    {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let (origin, challenge) = step_challenge(&installation, &sign_in_token).await;

        let credential =
            CredentialToVerify::Passkey(passkey_assertion(&user, &origin, &challenge, false));
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    #[tokio::test]
    async fn continue_sign_in_with_passkey_after_the_password_returns_a_session_from_both_methods()
    {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let after_password = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token.clone(),
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();
        let SignInOutcome::NextStep(SignInStep {
            token: sign_in_token,
            ..
        }) = &after_password
        else {
            panic!("a user with a second factor gives it after the password");
        };

        let credential = passkey_response(&installation, &user, sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token.clone(),
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await
            .unwrap();

        assert_matches!(
            after_password,
            SignInOutcome::NextStep(SignInStep { accepted, .. })
                if matches!(
                    accepted.as_slice(),
                    [AcceptedCredential::Totp, AcceptedCredential::Passkey { .. }]
                )
        );
        assert_matches!(outcome, SignInOutcome::SignedIn { .. });
        assert_eq!(
            signed_in_methods(&installation),
            [[
                AuthenticationMethod::Password,
                AuthenticationMethod::Passkey
            ]]
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_the_passkey_of_another_user_as_second_factor_returns_invalid_credentials_and_leaves_that_passkey()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation
            .registry
            .passkeys
            .create(
                &mut TxContext::none(),
                &PasskeyCredential {
                    id: uuid::Uuid::now_v7(),
                    user_id: user.id,
                    credential_id: CredentialId(vec![8; 16]),
                    public_key: vec![1; 33],
                    name: PasskeyName::parse("Phone").unwrap(),
                    sign_count: 0,
                    backup: PasskeyBackup::default(),
                    created_at: timestamp(0),
                    last_used_at: None,
                },
            )
            .await
            .unwrap();
        let other_user = installation.user_with_password("bruno").await;
        let other_passkey = installation.add_passkey(&other_user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let SignInOutcome::NextStep(SignInStep {
            token: sign_in_token,
            ..
        }) = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap()
        else {
            panic!("a user with a second factor gives it after the password");
        };

        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
        let stored = installation
            .registry
            .passkeys
            .get_by_credential_id(
                &mut TxContext::none(),
                &CredentialId(CREDENTIAL_ID.to_vec()),
            )
            .await
            .unwrap();
        assert_eq!(stored, other_passkey);
    }

    async fn new_reauthentication_token(
        service: &AuthenticationService<SqliteTx>,
        user: &User,
        session: &Session,
    ) -> SecretString {
        service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user: user.clone(),
                    },
                },
            )
            .await
            .unwrap()
            .token
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_gives_the_session_a_new_proof_and_a_new_token()
    {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let session = installation
            .registry
            .create_session(&Session {
                last_used_at: timestamp(NOW),
                expires_at: timestamp(NOW + 3600),
                ..identity::session_for(&user)
            })
            .await;
        let service = sign_in_service(&installation, NOW);
        let reauthentication_token = new_reauthentication_token(&service, &user, &session).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: reauthentication_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap();

        let SignInOutcome::SignedIn { token, .. } = outcome else {
            panic!("{outcome:?}");
        };
        let old_token_lookup = installation
            .registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &session.mac)
            .await;
        assert_matches!(old_token_lookup, Err(RepositoryError::NotFound));
        let authenticated = service
            .authenticate(&mut TxContext::none(), &token)
            .await
            .unwrap();
        assert_eq!(authenticated.session_id, session.id);
        assert!(authenticated.recently_authenticated_until > timestamp(NOW));
        assert_eq!(
            signed_in_methods(&installation),
            Vec::<Vec<AuthenticationMethod>>::new()
        );
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_with_the_username_of_another_user_counts_no_attempt()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let other_user = installation.user_with_password("bruno").await;
        let session = installation
            .registry
            .create_session(&Session {
                last_used_at: timestamp(NOW),
                expires_at: timestamp(NOW + 3600),
                ..identity::session_for(&user)
            })
            .await;
        let service = sign_in_service(&installation, NOW);
        for _ in 0..SIGN_IN_POLICY.password_failure_delay.free_failures {
            let reauthentication_token =
                new_reauthentication_token(&service, &user, &session).await;
            let outcome = service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: reauthentication_token,
                        device_token: None,
                        username: Some(other_user.username.clone()),
                        credential: CredentialToVerify::Password(identity::password()),
                    },
                )
                .await;
            assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
        }
        let reauthentication_token = new_reauthentication_token(&service, &user, &session).await;
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let reauthenticated = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: reauthentication_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;
        let signed_in = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(other_user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(reauthenticated, Ok(SignInOutcome::SignedIn { .. }));
        assert_matches!(signed_in, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_of_an_expired_session_returns_sign_in_ended() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let expired_session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);
        let reauthentication_token =
            new_reauthentication_token(&service, &user, &expired_session).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: reauthentication_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::SignInEnded));
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_after_the_session_ended_returns_sign_in_ended()
    {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);
        let reauthentication_token = new_reauthentication_token(&service, &user, &session).await;
        installation
            .registry
            .sessions
            .delete(&mut TxContext::none(), session.id)
            .await
            .unwrap();

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: reauthentication_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::SignInEnded));
        assert_eq!(
            signed_in_methods(&installation),
            Vec::<Vec<AuthenticationMethod>>::new()
        );
    }

    #[tokio::test]
    async fn start_sign_in_of_a_reauthentication_offers_only_the_passkeys_of_the_user() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_passkey(&user).await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);

        let started = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user,
                    },
                },
            )
            .await
            .unwrap();

        assert_matches!(
            started.accepted.as_slice(),
            [AcceptedCredential::Password, AcceptedCredential::Passkey { options, .. }]
                if options
                    .allow_credentials
                    .iter()
                    .map(|allowed| allowed.id.as_slice())
                    .eq([CREDENTIAL_ID.as_slice()])
        );
    }

    #[tokio::test]
    async fn continue_sign_in_after_the_free_failures_of_a_username_returns_too_many_attempts() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        for _ in 0..SIGN_IN_POLICY.password_failure_delay.free_failures {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(user.username.clone()),
                        credential: CredentialToVerify::Password(wrong_password()),
                    },
                )
                .await
                .unwrap_err();
        }
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(
            outcome,
            Err(ContinueSignInError::TooManyAttempts { retry_after })
                if retry_after == Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_device_token_after_the_free_failures_of_the_username_returns_a_session()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let SignInOutcome::SignedIn { device_token, .. } = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap()
        else {
            panic!("a correct password of a user with no second factor signs in");
        };
        for _ in 0..SIGN_IN_POLICY.password_failure_delay.free_failures {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(user.username.clone()),
                        credential: CredentialToVerify::Password(wrong_password()),
                    },
                )
                .await
                .unwrap_err();
        }
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: Some(device_token),
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_after_the_delay_of_the_failures_returns_a_session() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let service = sign_in_service(&installation, NOW);
        for _ in 0..SIGN_IN_POLICY.password_failure_delay.free_failures {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(user.username.clone()),
                        credential: CredentialToVerify::Password(wrong_password()),
                    },
                )
                .await
                .unwrap_err();
        }
        let later_service = sign_in_service(&installation, NOW + 1);
        let sign_in_token = new_sign_in_token(&later_service, origin()).await;

        let outcome = later_service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_after_the_free_failures_of_an_unknown_username_returns_invalid_credentials()
     {
        let installation = Installation::new().await;
        let service = sign_in_service(&installation, NOW);
        for _ in 0..SIGN_IN_POLICY.password_failure_delay.free_failures {
            let sign_in_token = new_sign_in_token(&service, origin()).await;
            service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: Some(Username::parse("nobody").unwrap()),
                        credential: CredentialToVerify::Password(wrong_password()),
                    },
                )
                .await
                .unwrap_err();
        }
        let sign_in_token = new_sign_in_token(&service, origin()).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: Some(Username::parse("nobody").unwrap()),
                    credential: CredentialToVerify::Password(wrong_password()),
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
    }

    async fn send_the_free_wrong_codes(service: &AuthenticationService<SqliteTx>, user: &User) {
        for _ in 0..SIGN_IN_POLICY.totp_failure_delay.free_failures {
            let sign_in_token = second_step_token(service, user, None).await;
            service
                .continue_sign_in(
                    &mut TxContext::none(),
                    ContinueSignInCommand {
                        token: sign_in_token,
                        device_token: None,
                        username: None,
                        credential: CredentialToVerify::Totp(TotpCode::parse("000000").unwrap()),
                    },
                )
                .await
                .unwrap_err();
        }
    }

    async fn second_step_token(
        service: &AuthenticationService<SqliteTx>,
        user: &User,
        device_token: Option<SecretString>,
    ) -> SecretString {
        let sign_in_token = new_sign_in_token(service, origin()).await;
        let SignInOutcome::NextStep(SignInStep { token, .. }) = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token,
                    username: Some(user.username.clone()),
                    credential: CredentialToVerify::Password(identity::password()),
                },
            )
            .await
            .unwrap()
        else {
            panic!("a user with a second factor gives it after the password");
        };

        token
    }

    #[tokio::test]
    async fn continue_sign_in_after_the_free_failures_of_the_codes_waits_and_keeps_the_sign_in() {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        let service = sign_in_service(&installation, NOW);
        send_the_free_wrong_codes(&service, &user).await;
        let sign_in_token = second_step_token(&service, &user, None).await;

        let throttled = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token.clone(),
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await;
        let later = NOW + 60;
        let later_outcome = sign_in_service(&installation, later)
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(later)),
                },
            )
            .await;

        assert_matches!(throttled, Err(ContinueSignInError::TooManyAttempts { .. }));
        assert_matches!(later_outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_device_token_after_the_free_failures_of_the_codes_accepts_the_code()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = new_sign_in_token(&service, origin()).await;
        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let SignInOutcome::SignedIn { device_token, .. } = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await
            .unwrap()
        else {
            panic!("a passkey alone signs in");
        };
        send_the_free_wrong_codes(&service, &user).await;
        let sign_in_token = second_step_token(&service, &user, Some(device_token.clone())).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: Some(device_token),
                    username: None,
                    credential: CredentialToVerify::Totp(code_at(NOW)),
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_recovery_code_after_the_free_failures_of_the_codes_returns_a_session()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_recovery_code(&user).await;
        let service = sign_in_service(&installation, NOW);
        send_the_free_wrong_codes(&service, &user).await;
        let sign_in_token = second_step_token(&service, &user, None).await;

        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential: CredentialToVerify::RecoveryCode(
                        RecoveryCode::parse(RECOVERY_CODE).unwrap(),
                    ),
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_with_a_passkey_after_the_free_failures_of_the_codes_returns_a_session()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_passkey(&user).await;
        let service = sign_in_service(&installation, NOW);
        send_the_free_wrong_codes(&service, &user).await;
        let sign_in_token = second_step_token(&service, &user, None).await;

        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Ok(SignInOutcome::SignedIn { .. }));
    }

    #[tokio::test]
    async fn continue_sign_in_of_a_reauthentication_with_the_passkey_of_another_user_returns_invalid_credentials_and_leaves_that_passkey()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let other_user = installation.user_with_password("bruno").await;
        let other_passkey = installation.add_passkey(&other_user).await;
        installation
            .registry
            .passkeys
            .create(
                &mut TxContext::none(),
                &PasskeyCredential {
                    id: uuid::Uuid::now_v7(),
                    user_id: user.id,
                    credential_id: CredentialId(vec![8; 16]),
                    public_key: vec![1; 33],
                    name: PasskeyName::parse("Phone").unwrap(),
                    sign_count: 0,
                    backup: PasskeyBackup::default(),
                    created_at: timestamp(0),
                    last_used_at: None,
                },
            )
            .await
            .unwrap();
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);
        let sign_in_token = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user: user.clone(),
                    },
                },
            )
            .await
            .unwrap()
            .token;

        let credential = passkey_response(&installation, &user, &sign_in_token).await;
        let outcome = service
            .continue_sign_in(
                &mut TxContext::none(),
                ContinueSignInCommand {
                    token: sign_in_token,
                    device_token: None,
                    username: None,
                    credential,
                },
            )
            .await;

        assert_matches!(outcome, Err(ContinueSignInError::InvalidCredentials(_)));
        let stored = installation
            .registry
            .passkeys
            .get_by_credential_id(
                &mut TxContext::none(),
                &CredentialId(CREDENTIAL_ID.to_vec()),
            )
            .await
            .unwrap();
        assert_eq!(stored, other_passkey);
    }

    #[tokio::test]
    async fn start_sign_in_of_a_reauthentication_for_a_user_with_only_a_password_offers_only_the_password()
     {
        let installation = Installation::new().await;
        let user = installation.user_with_password("alice").await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);

        let started = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user,
                    },
                },
            )
            .await
            .unwrap();

        assert_matches!(started.accepted.as_slice(), [AcceptedCredential::Password]);
    }

    #[tokio::test]
    async fn start_sign_in_of_a_reauthentication_for_a_user_with_only_a_passkey_offers_only_the_passkey()
     {
        let installation = Installation::new().await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        installation.add_passkey(&user).await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);

        let started = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user,
                    },
                },
            )
            .await
            .unwrap();

        assert_matches!(
            started.accepted.as_slice(),
            [AcceptedCredential::Passkey { .. }]
        );
    }

    #[tokio::test]
    async fn start_sign_in_of_a_reauthentication_with_only_a_passkey_at_an_ip_address_returns_no_method_at_origin()
     {
        let installation = Installation::new().await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        installation.add_passkey(&user).await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;
        let service = sign_in_service(&installation, NOW);

        let started = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::Reauthentication {
                    origin: url::Url::parse("https://10.0.0.5:8443").unwrap().origin(),
                    user: AuthenticatedUser {
                        session_id: session.id,
                        recently_authenticated_until: jiff::Timestamp::MAX,
                        user,
                    },
                },
            )
            .await;

        assert_matches!(started, Err(StartSignInError::NoMethodAtOrigin));
    }

    #[tokio::test]
    async fn start_sign_in_at_an_ip_address_offers_no_passkey() {
        let installation = Installation::new().await;
        let service = sign_in_service(&installation, NOW);

        let started = service
            .start_sign_in(
                &mut TxContext::none(),
                StartSignInCommand::NewSession {
                    origin: url::Url::parse("https://10.0.0.5:8443").unwrap().origin(),
                },
            )
            .await
            .unwrap();

        assert_matches!(started.accepted.as_slice(), [AcceptedCredential::Password]);
    }
}
