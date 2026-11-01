use super::authentication::EnrollmentToken;
use super::commands::create::CreateMachineCommand;
use super::commands::issue_enrollment_token::IssueEnrollmentTokenCommand;
use super::commands::open_terminal::OpenTerminalCommand;
use super::commands::test_connection::TestConnectionCommand;
use super::commands::update::UpdateMachineCommand;
use super::errors::{
    CreateMachineError, DeleteMachineError, GetMachineError, IssueEnrollmentTokenError,
    ListMachinesError, OpenTerminalError, TestConnectionError, UnenrollError, UpdateMachineError,
};
use super::ports::TerminalSession;
use crate::application::identity::authorization::actor::Actor;
use crate::application::repository::{ListQuery, Paginated};
use crate::domain::machine::Machine;
use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MachineManagement: Send + Sync {
    async fn list(
        &self,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<Machine>, ListMachinesError>;
    async fn get(&self, actor: &Actor, id: uuid::Uuid) -> Result<Machine, GetMachineError>;
    async fn create(
        &self,
        actor: &Actor,
        cmd: CreateMachineCommand,
    ) -> Result<Machine, CreateMachineError>;
    async fn update(
        &self,
        actor: &Actor,
        cmd: UpdateMachineCommand,
    ) -> Result<Machine, UpdateMachineError>;
    async fn delete(&self, actor: &Actor, id: uuid::Uuid) -> Result<Machine, DeleteMachineError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MachineEnrollment: Send + Sync {
    async fn issue_enrollment_token(
        &self,
        actor: &Actor,
        cmd: IssueEnrollmentTokenCommand,
    ) -> Result<EnrollmentToken, IssueEnrollmentTokenError>;
    async fn unenroll(&self, actor: &Actor, machine_id: uuid::Uuid) -> Result<(), UnenrollError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MachineConnections: Send + Sync {
    async fn test(
        &self,
        actor: &Actor,
        cmd: TestConnectionCommand,
    ) -> Result<(), TestConnectionError>;
    async fn open_terminal(
        &self,
        actor: Actor,
        cmd: OpenTerminalCommand,
    ) -> Result<TerminalSession, OpenTerminalError>;
}
