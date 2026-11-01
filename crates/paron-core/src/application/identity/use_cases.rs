use async_trait::async_trait;
use secrecy::SecretString;

use super::authentication::account_recovery::{
    AccountRecoveryToken, RecoverAccountCommand, RecoveryOptions, RecoveryOutcome,
};
use super::authentication::credentials::{
    AccountCredentials, UpdateCredentialsCommand, UpdatedCredentials,
};
use super::authentication::sign_in::{
    ContinueSignInCommand, SignInOutcome, SignInStep, StartSignInCommand,
};
use super::authorization::actor::{Actor, AuthenticatedUser};
use super::users::{CreateUserCommand, UpdateUserCommand};
use crate::application::identity::errors::{
    AuthenticateError, ContinueSignInError, CreateUserError, DeleteUserError, GetUserError,
    ListCredentialsError, ListUsersError, RecoverAccountError, RecoveryOptionsError, SignOutError,
    StartRecoveryError, StartSignInError, TouchSessionError, UpdateCredentialsError,
    UpdateUserError,
};
use crate::application::repository::{ListQuery, Paginated};
use crate::domain::user::User;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserManagement: Send + Sync {
    async fn list(
        &self,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<User>, ListUsersError>;
    async fn get_by_id(&self, actor: &Actor, id: uuid::Uuid) -> Result<User, GetUserError>;
    async fn create(&self, actor: &Actor, cmd: CreateUserCommand) -> Result<User, CreateUserError>;
    async fn update(&self, actor: &Actor, cmd: UpdateUserCommand) -> Result<User, UpdateUserError>;
    async fn delete(&self, actor: &Actor, id: uuid::Uuid) -> Result<User, DeleteUserError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Authentication: Send + Sync {
    async fn start_sign_in(&self, cmd: StartSignInCommand) -> Result<SignInStep, StartSignInError>;
    async fn continue_sign_in(
        &self,
        cmd: ContinueSignInCommand,
    ) -> Result<SignInOutcome, ContinueSignInError>;
    async fn sign_out(&self, actor: &Actor) -> Result<(), SignOutError>;
    async fn authenticate(
        &self,
        session_token: &SecretString,
    ) -> Result<AuthenticatedUser, AuthenticateError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Sessions: Send + Sync {
    async fn touch_session(&self, actor: &Actor) -> Result<(), TouchSessionError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CredentialManagement: Send + Sync {
    async fn list(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountCredentials, ListCredentialsError>;
    async fn update(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
        cmd: UpdateCredentialsCommand,
    ) -> Result<UpdatedCredentials, UpdateCredentialsError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AccountRecovery: Send + Sync {
    async fn start_recovery(
        &self,
        actor: &Actor,
        user_id: uuid::Uuid,
    ) -> Result<AccountRecoveryToken, StartRecoveryError>;
    async fn options(
        &self,
        token: SecretString,
        origin: url::Origin,
    ) -> Result<RecoveryOptions, RecoveryOptionsError>;
    async fn recover(
        &self,
        cmd: RecoverAccountCommand,
    ) -> Result<RecoveryOutcome, RecoverAccountError>;
}
