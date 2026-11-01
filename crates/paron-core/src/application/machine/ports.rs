use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::Sink;

use crate::application::repository::{ListQuery, Paginated, RepositoryError, TxContext};
use crate::domain::machine::{ConnectionTest, Connectivity, Machine, Terminal, ssh};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        query: &ListQuery,
    ) -> Result<Paginated<Machine>, RepositoryError>;
    async fn get_by_id(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<Machine, RepositoryError>;
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        machine: &Machine,
    ) -> Result<Machine, RepositoryError>;
    async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        machine: &Machine,
    ) -> Result<Machine, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<Machine, RepositoryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("the machine has not completed enrollment")]
    NotEnrolled,

    #[error("the machine could not be reached")]
    Unreachable(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("the host key does not match the one on record")]
    HostKeyMismatch,

    #[error("the credentials were rejected")]
    AuthenticationFailed,

    #[error("the machine refused to open a terminal")]
    TerminalRefused,

    #[error("the operation could not be attempted")]
    Unexpected(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: std::num::NonZeroU16,
    pub rows: std::num::NonZeroU16,
}

/// A terminal tells the machine two facts. The first is which escape sequences it understands:
/// `TERM`, a terminfo name such as `xterm-256color`, `xterm` (16 colours), `tmux-256color` or
/// `linux`. The second is whether it draws 24-bit colour: `COLORTERM=truecolor`, a convention
/// outside terminfo. Most terminals send `xterm-256color`, some without `COLORTERM`.
///
/// Paron has one client, xterm.js, so there is one variant. A second client makes this a pair,
/// emulation (xterm, tmux, …) and colours (16, 256, 24-bit), which each transport maps to its
/// own fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalType {
    /// xterm with 256 colours and 24-bit colour.
    XtermTrueColor,
}

pub type TerminalInputSink = Pin<Box<dyn Sink<TerminalInput, Error = TransportError> + Send>>;
pub type TerminalOutputStream = Pin<Box<dyn Stream<Item = TerminalEvent> + Send>>;

/// A terminal on a machine: characters in, and characters and an exit out.
pub struct TerminalSession {
    pub input: TerminalInputSink,
    pub output: TerminalOutputStream,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerminalInput {
    Data(Vec<u8>),
    Resize(TerminalSize),
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    Exit(TerminalExit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalExit {
    Status(u32),
    Signal(String),
    Disconnected,
}

#[derive(Clone)]
pub struct Target<C> {
    pub machine_id: uuid::Uuid,
    pub connectivity: Connectivity,
    pub capability: C,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MachineTransport: Send + Sync {
    async fn verify(&self, target: Target<ConnectionTest>) -> Result<(), TransportError>;
    async fn terminal(
        &self,
        target: Target<Terminal>,
        size: TerminalSize,
        terminal: TerminalType,
    ) -> Result<TerminalSession, TransportError>;
    async fn disconnect(&self, machine_id: uuid::Uuid);
}

#[derive(Clone, Copy)]
pub enum SshKeyAlgorithm {
    Ed25519,
}

#[cfg_attr(test, mockall::automock)]
pub trait SshKeypairs: Send + Sync {
    fn generate(&self, algorithm: SshKeyAlgorithm) -> ssh::PrivateKey;
}
