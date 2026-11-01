use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use secrecy::SecretString;
use webauthn_rs_proto::{PublicKeyCredentialCreationOptions, RegisterPublicKeyCredential};

use super::credentials::{
    CredentialService, CredentialToAdd, CredentialUpdateResult, UpdateCredentialsCommand,
};
use super::methods::AuthenticationMethod;
use super::methods::password::Password;
use super::ports::PendingAccountRecoveryRepository;
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::{Actor, Denied};
use crate::application::identity::errors::{
    GetUserError, RecoverAccountError, RecoveryOptionsError, StartRecoveryError,
    UpdateCredentialsError,
};
use crate::application::identity::event::AuditKind;
use crate::application::identity::use_cases::AccountRecovery;
use crate::application::identity::users::UserService;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;
use crate::domain::user::{Permission, User};

#[cfg_attr(test, derive(Debug))]
pub struct AccountRecoveryToken {
    pub token: SecretString,
    pub expires_at: jiff::Timestamp,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct RecoveryOptions {
    pub methods: Vec<AuthenticationMethod>,
    pub expires_at: jiff::Timestamp,
}

pub enum RecoveryAction {
    SetPassword(Password),
    StartPasskeyRegistration {
        origin: url::Origin,
    },
    FinishPasskeyRegistration {
        origin: url::Origin,
        response: RegisterPublicKeyCredential,
    },
}

pub struct RecoverAccountCommand {
    pub token: SecretString,
    pub action: RecoveryAction,
}

#[cfg_attr(test, derive(Debug))]
#[allow(clippy::large_enum_variant)]
pub enum RecoveryOutcome {
    PasskeyCreationOptions(PublicKeyCredentialCreationOptions),
    Recovered(User),
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct PendingAccountRecovery {
    pub user_id: uuid::Uuid,
    pub mac: TokenMac,
    pub expires_at: jiff::Timestamp,
}

pub struct AccountRecoveryDeps<Tx> {
    pub users: Arc<UserService<Tx>>,
    pub credentials: Arc<CredentialService<Tx>>,
    pub repository: Arc<dyn PendingAccountRecoveryRepository<Tx>>,
    pub tokens: Arc<dyn Tokens>,
    pub unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    pub audit: Arc<AuditService<Tx>>,
    pub clock: Arc<dyn Clock>,
    pub token_lifetime: Duration,
}

pub struct AccountRecoveryService<Tx> {
    deps: AccountRecoveryDeps<Tx>,
}

impl<Tx: Send + Sync + 'static> AccountRecoveryService<Tx> {
    pub fn new(deps: AccountRecoveryDeps<Tx>) -> Self {
        Self { deps }
    }

    pub async fn start_recovery(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountRecoveryToken, StartRecoveryError> {
        actor.authorize(Permission::UserWrite)?;
        if actor.is_user(user_id) {
            return Err(StartRecoveryError::Denied(Denied::NotAllowed));
        }
        actor.require_recent_authentication(self.deps.clock.now())?;

        self.deps
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let user = match self.deps.users.get_by_id(ctx, actor, user_id).await {
                    Ok(user) => user,
                    Err(GetUserError::Denied(denied)) => {
                        return Err(StartRecoveryError::Denied(denied));
                    }
                    Err(GetUserError::NotFound) => return Err(StartRecoveryError::UserNotFound),
                    Err(GetUserError::Unexpected(cause)) => return Err(cause.into()),
                };
                for permission in user.permissions() {
                    actor.authorize(permission)?;
                }

                self.deps.credentials.unenroll(ctx, actor, &user).await?;

                let (token, mac) = self.deps.tokens.issue();
                let recovery = self
                    .deps
                    .repository
                    .upsert(
                        ctx,
                        &PendingAccountRecovery {
                            user_id: user.id,
                            mac,
                            expires_at: self.deps.clock.now() + self.deps.token_lifetime,
                        },
                    )
                    .await
                    .context("could not upsert pending account recovery")?;
                self.deps
                    .audit
                    .append(
                        ctx,
                        actor,
                        &user,
                        AuditKind::RecoveryStarted {
                            expires_at: recovery.expires_at,
                        },
                    )
                    .await?;

                Ok(AccountRecoveryToken {
                    token,
                    expires_at: recovery.expires_at,
                })
            })
            .await
    }

    pub async fn options(
        &self,
        ctx: &mut TxContext<Tx>,
        token: &SecretString,
        origin: &url::Origin,
    ) -> Result<RecoveryOptions, RecoveryOptionsError> {
        let mac = self.deps.tokens.mac(token);
        let recovery = match self.deps.repository.get_by_mac(ctx, &mac).await {
            Ok(recovery) if self.deps.clock.now() < recovery.expires_at => recovery,
            Ok(_) => {
                return Err(RecoveryOptionsError::InvalidToken(anyhow::anyhow!(
                    "the account recovery expired"
                )));
            }
            Err(RepositoryError::NotFound) => {
                return Err(RecoveryOptionsError::InvalidToken(anyhow::anyhow!(
                    "no account recovery has this token"
                )));
            }
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("could not get pending account recovery by mac")
                    .into());
            }
        };

        Ok(RecoveryOptions {
            methods: self.deps.credentials.first_factors(origin),
            expires_at: recovery.expires_at,
        })
    }

    pub async fn recover(
        &self,
        ctx: &mut TxContext<Tx>,
        cmd: RecoverAccountCommand,
    ) -> Result<RecoveryOutcome, RecoverAccountError> {
        let mac = self.deps.tokens.mac(&cmd.token);
        let update = match cmd.action {
            RecoveryAction::SetPassword(password) => {
                UpdateCredentialsCommand::Add(CredentialToAdd::Password(password))
            }
            RecoveryAction::StartPasskeyRegistration { origin } => {
                UpdateCredentialsCommand::StartPasskeyRegistration { origin }
            }
            RecoveryAction::FinishPasskeyRegistration { origin, response } => {
                UpdateCredentialsCommand::Add(CredentialToAdd::Passkey { origin, response })
            }
        };

        self.deps
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let recovery = match self.deps.repository.get_by_mac(ctx, &mac).await {
                    Ok(recovery) if self.deps.clock.now() < recovery.expires_at => recovery,
                    Ok(_) => {
                        return Err(RecoverAccountError::InvalidToken(anyhow::anyhow!(
                            "the account recovery expired"
                        )));
                    }
                    Err(RepositoryError::NotFound) => {
                        return Err(RecoverAccountError::InvalidToken(anyhow::anyhow!(
                            "no account recovery has this token"
                        )));
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get pending account recovery by mac")
                            .into());
                    }
                };
                let updated = match self
                    .deps
                    .credentials
                    .update(ctx, &Actor::System, recovery.user_id, update)
                    .await
                {
                    Ok(updated) => updated,
                    Err(UpdateCredentialsError::InvalidCredentials(cause)) => {
                        return Err(RecoverAccountError::InvalidCredentials(cause));
                    }
                    Err(error) => return Err(anyhow::Error::new(error).into()),
                };

                match updated.result {
                    CredentialUpdateResult::PasskeyCreationOptions(options) => {
                        Ok(RecoveryOutcome::PasskeyCreationOptions(options))
                    }
                    CredentialUpdateResult::Done => {
                        self.deps
                            .repository
                            .delete_by_mac(ctx, &mac)
                            .await
                            .context("could not delete pending account recovery by mac")?;
                        let user = self
                            .deps
                            .users
                            .get_by_id(ctx, &Actor::System, recovery.user_id)
                            .await
                            .context("could not get user by id")?;

                        Ok(RecoveryOutcome::Recovered(user))
                    }
                    other @ CredentialUpdateResult::TotpRegistrationStarted(_) => {
                        unreachable!("a recovery got {other:?}")
                    }
                }
            })
            .await
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> AccountRecovery for AccountRecoveryService<Tx> {
    async fn start_recovery(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountRecoveryToken, StartRecoveryError> {
        Self::start_recovery(self, &mut TxContext::none(), actor, user_id).await
    }

    async fn options(
        &self,
        token: SecretString,
        origin: url::Origin,
    ) -> Result<RecoveryOptions, RecoveryOptionsError> {
        Self::options(self, &mut TxContext::none(), &token, &origin).await
    }

    async fn recover(
        &self,
        cmd: RecoverAccountCommand,
    ) -> Result<RecoveryOutcome, RecoverAccountError> {
        Self::recover(self, &mut TxContext::none(), cmd).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::audit;
    use crate::application::identity::authentication::methods::AuthenticationMethod;
    use crate::application::identity::authentication::methods::ports::{
        PasskeyCredentialRepository as _, PasswordCredentialRepository as _,
        TotpCredentialRepository as _,
    };
    use crate::application::identity::authentication::ports::SessionRepository as _;
    use crate::application::identity::authorization::actor::{AuthenticatedUser, Denied};
    use crate::application::identity::ports::UserRepository as _;
    use crate::domain::user::UserStatus;
    use crate::infrastructure::crypto::tokens::Hmac;
    use crate::infrastructure::sqlite::SqliteTx;
    use crate::testing::authentication::{self, Installation, NOW};
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::keys::symmetric_key;

    fn account_recovery_service(installation: &Installation) -> AccountRecoveryService<SqliteTx> {
        let services = installation.services(NOW);

        AccountRecoveryService::new(AccountRecoveryDeps {
            users: services.users,
            credentials: services.credentials,
            repository: Arc::clone(&installation.registry.pending_account_recoveries) as Arc<_>,
            tokens: Arc::new(Hmac::new(symmetric_key([3; 32]))),
            unit_of_work: services.unit_of_work as Arc<_>,
            audit: services.audit,
            clock: services.clock,
            token_lifetime: Duration::from_hours(24),
        })
    }

    #[tokio::test]
    async fn recover_sets_the_password() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let token = service
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap()
            .token;

        let recovered = service
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token,
                    action: RecoveryAction::SetPassword(identity::password()),
                },
            )
            .await
            .unwrap();

        assert_matches!(
            recovered,
            RecoveryOutcome::Recovered(recovered) if recovered.id == user.id
        );
        assert_matches!(
            installation
                .registry
                .passwords
                .get(&mut TxContext::none(), user.id)
                .await,
            Ok(_)
        );
    }

    #[tokio::test]
    async fn recover_twice_returns_unknown_token() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let token = service
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap()
            .token;
        service
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token: token.clone(),
                    action: RecoveryAction::SetPassword(identity::password()),
                },
            )
            .await
            .unwrap();

        let reused = service
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token,
                    action: RecoveryAction::SetPassword(identity::password()),
                },
            )
            .await;

        assert_matches!(reused, Err(RecoverAccountError::InvalidToken(_)));
    }

    #[tokio::test]
    async fn recover_of_a_suspended_user_sets_the_password() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let token = service
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap()
            .token;
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

        let outcome = service
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token,
                    action: RecoveryAction::SetPassword(identity::password()),
                },
            )
            .await;

        assert_matches!(
            outcome,
            Ok(RecoveryOutcome::Recovered(recovered)) if recovered.id == user.id
        );
        assert_matches!(
            installation
                .registry
                .passwords
                .get(&mut TxContext::none(), user.id)
                .await,
            Ok(_)
        );
    }

    #[tokio::test]
    async fn options_at_an_ip_address_offers_only_the_password() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let token = service
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap()
            .token;

        let options = service
            .options(
                &mut TxContext::none(),
                &token,
                &url::Url::parse("https://10.0.0.5:8443").unwrap().origin(),
            )
            .await
            .unwrap();

        assert_eq!(options.methods, [AuthenticationMethod::Password]);
    }

    #[tokio::test]
    async fn options_at_a_domain_name_offers_the_password_and_a_passkey() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let token = service
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap()
            .token;

        let options = service
            .options(&mut TxContext::none(), &token, &authentication::origin())
            .await
            .unwrap();

        assert_eq!(
            options.methods,
            [
                AuthenticationMethod::Password,
                AuthenticationMethod::Passkey
            ]
        );
    }

    #[tokio::test]
    async fn recover_with_each_offered_method_succeeds() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let service = account_recovery_service(&installation);
        let new_token = async || {
            service
                .start_recovery(
                    &mut TxContext::none(),
                    &identity::actor_for(administrator.clone()),
                    user.id,
                )
                .await
                .unwrap()
                .token
        };
        let token = new_token().await;
        let offered = service
            .options(&mut TxContext::none(), &token, &authentication::origin())
            .await
            .unwrap()
            .methods;

        for method in offered {
            // A new first factor fails to compile here until recovery has an action for it.
            let action = match method {
                AuthenticationMethod::Password => RecoveryAction::SetPassword(identity::password()),
                AuthenticationMethod::Passkey => RecoveryAction::StartPasskeyRegistration {
                    origin: authentication::origin(),
                },
                AuthenticationMethod::Totp | AuthenticationMethod::RecoveryCode => {
                    panic!("recovery offered {method}, which is not a first factor")
                }
            };
            let recovered = service
                .recover(
                    &mut TxContext::none(),
                    RecoverAccountCommand {
                        token: new_token().await,
                        action,
                    },
                )
                .await;

            assert_matches!(recovered, Ok(_), "{method}");
        }
    }

    #[tokio::test]
    async fn start_recovery_removes_every_credential_and_audits_each_method() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation.user_with_password("alice").await;
        installation.add_totp(&user).await;
        installation.add_passkey(&user).await;

        account_recovery_service(&installation)
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap();

        let password = installation
            .registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(password, Err(RepositoryError::NotFound));
        let totp = installation
            .registry
            .totps
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(totp, Err(RepositoryError::NotFound));
        let passkeys = installation
            .registry
            .passkeys
            .list_for_user(&mut TxContext::none(), user.id)
            .await
            .unwrap();
        assert_eq!(passkeys, []);
        let removed_methods: Vec<_> = installation
            .audit
            .recorded_kinds()
            .into_iter()
            .filter_map(|kind| match kind {
                audit::AuditKind::User(AuditKind::CredentialRemoved { kind }) => Some(kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            removed_methods,
            [
                AuthenticationMethod::Password,
                AuthenticationMethod::Totp,
                AuthenticationMethod::Passkey,
            ]
        );
    }

    #[tokio::test]
    async fn start_recovery_ends_every_session_of_the_user() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation.user_with_password("alice").await;
        let session = installation
            .registry
            .create_session(&identity::session_for(&user))
            .await;

        account_recovery_service(&installation)
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator),
                user.id,
            )
            .await
            .unwrap();

        let found = installation
            .registry
            .sessions
            .get_by_id(&mut TxContext::none(), session.id)
            .await;
        assert_matches!(found, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn start_recovery_for_the_own_account_returns_denied() {
        let installation = Installation::new().await;
        let administrator = installation
            .registry
            .create_user(&identity::user_builder().username("admin").build())
            .await;

        let issued = account_recovery_service(&installation)
            .start_recovery(
                &mut TxContext::none(),
                &identity::actor_for(administrator.clone()),
                administrator.id,
            )
            .await;

        assert_matches!(issued, Err(StartRecoveryError::Denied(Denied::NotAllowed)));
    }

    #[tokio::test]
    async fn start_recovery_without_a_recent_proof_returns_recent_authentication_required() {
        let installation = Installation::new().await;
        let administrator = installation.user_with_password("admin").await;
        let user = installation
            .registry
            .create_user(&identity::user_builder().username("alice").build())
            .await;
        let actor = Actor::User(AuthenticatedUser {
            session_id: uuid::Uuid::now_v7(),
            recently_authenticated_until: timestamp(NOW),
            user: administrator,
        });

        let issued = account_recovery_service(&installation)
            .start_recovery(&mut TxContext::none(), &actor, user.id)
            .await;

        assert_matches!(
            issued,
            Err(StartRecoveryError::Denied(
                Denied::RecentAuthenticationRequired
            ))
        );
    }
}
