use super::commands::update::InvalidUpdate;
use super::ports::TransportError;
use crate::application::identity::authorization::actor::Denied;
use crate::domain::machine::CapabilityUnavailable;

#[derive(Debug, thiserror::Error)]
pub enum ListMachinesError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum GetMachineError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CreateMachineError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateMachineError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error(transparent)]
    InvalidUpdate(#[from] InvalidUpdate),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DeleteMachineError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum IssueEnrollmentTokenError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error("the machine is not reached by a reverse tunnel")]
    NotEnrollable,

    #[error("the machine has already enrolled")]
    AlreadyEnrolled,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl From<GetMachineError> for IssueEnrollmentTokenError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::Denied(denied) => Self::Denied(denied),
            GetMachineError::NotFound => Self::NotFound,
            GetMachineError::Unexpected(cause) => Self::Unexpected(cause),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("machine not found")]
    NotFound,

    #[error("the machine is not reached by a reverse tunnel")]
    NotEnrollable,

    #[error("the machine has already enrolled")]
    AlreadyEnrolled,

    #[error("the machine has not been invited to enrol")]
    NotInvited,

    #[error("the enrollment token has expired")]
    Expired,

    #[error("the enrollment token is not valid")]
    InvalidToken,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl From<GetMachineError> for EnrollError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::NotFound => Self::NotFound,
            GetMachineError::Denied(denied) => Self::Unexpected(denied.into()),
            GetMachineError::Unexpected(cause) => Self::Unexpected(cause),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UnenrollError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error("the machine is not reached by a reverse tunnel")]
    NotEnrollable,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl From<GetMachineError> for UnenrollError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::Denied(denied) => Self::Denied(denied),
            GetMachineError::NotFound => Self::NotFound,
            GetMachineError::Unexpected(cause) => Self::Unexpected(cause),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TestConnectionError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error(transparent)]
    InvalidUpdate(#[from] InvalidUpdate),

    #[error(transparent)]
    CapabilityUnavailable(#[from] CapabilityUnavailable),

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl From<GetMachineError> for TestConnectionError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::Denied(denied) => Self::Denied(denied),
            GetMachineError::NotFound => Self::NotFound,
            GetMachineError::Unexpected(cause) => Self::Unexpected(cause),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenTerminalError {
    #[error(transparent)]
    Denied(#[from] Denied),

    #[error("machine not found")]
    NotFound,

    #[error(transparent)]
    CapabilityUnavailable(#[from] CapabilityUnavailable),

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl From<GetMachineError> for OpenTerminalError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::Denied(denied) => Self::Denied(denied),
            GetMachineError::NotFound => Self::NotFound,
            GetMachineError::Unexpected(cause) => Self::Unexpected(cause),
        }
    }
}
