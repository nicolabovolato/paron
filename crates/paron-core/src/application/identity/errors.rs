use super::authentication::methods::AuthenticationMethod;
use super::authorization::actor::Denied;

#[derive(Debug, thiserror::Error)]
pub enum ListUsersError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum GetUserError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("user not found")]
    NotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CreateUserError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("this username is already in use")]
    UsernameTaken,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateUserError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("user not found")]
    NotFound,

    #[error("this username is already in use")]
    UsernameTaken,

    #[error("this change removes the last administrator")]
    LastAdministrator,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DeleteUserError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("user not found")]
    NotFound,

    #[error("this change removes the last administrator")]
    LastAdministrator,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum StartSignInError {
    #[error("no way to sign in works at this address")]
    NoMethodAtOrigin,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ContinueSignInError {
    #[error("the credentials are not correct")]
    InvalidCredentials(#[source] anyhow::Error),

    #[error("too many wrong attempts, try again later")]
    TooManyAttempts { retry_after: std::time::Duration },

    #[error("this account is suspended")]
    Suspended,

    #[error("sign-in ended")]
    SignInEnded,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SignOutError {
    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthenticateError {
    #[error("session ended")]
    SessionEnded,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum TouchSessionError {
    #[error("session ended")]
    SessionEnded,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ListCredentialsError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

/// A credential that a user can have only one time.
#[derive(Debug, thiserror::Error)]
pub enum ExistingCredential {
    #[error("an authenticator app is already set up")]
    Totp,

    #[error("this passkey is already registered")]
    Passkey,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateCredentialsError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("user not found")]
    UserNotFound,

    #[error("the credentials are not correct")]
    InvalidCredentials(#[source] anyhow::Error),

    #[error(transparent)]
    AlreadyExists(ExistingCredential),

    #[error("credential not found")]
    CredentialNotFound,

    #[error("add another way to sign in first")]
    LastSignInMethod,

    #[error("the account has the maximum number of {0} credentials")]
    LimitReached(AuthenticationMethod),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum StartRecoveryError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("user not found")]
    UserNotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryOptionsError {
    #[error("the token is not correct")]
    InvalidToken(#[source] anyhow::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RecoverAccountError {
    #[error("the token is not correct")]
    InvalidToken(#[source] anyhow::Error),

    #[error("the credentials are not correct")]
    InvalidCredentials(#[source] anyhow::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
