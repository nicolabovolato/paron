mod authentication;
mod commands;
mod connection;
mod errors;
pub mod event;
mod ports;
mod service;
mod use_cases;

pub use authentication::{EnrollmentPolicy, EnrollmentToken, MachineAuthenticationService};
pub use commands::create::{
    AuthenticationCommand, ConnectivityCommand, CreateMachineCommand, ProtocolsCommand, SshCommand,
};
pub use commands::issue_enrollment_token::IssueEnrollmentTokenCommand;
pub use commands::open_terminal::OpenTerminalCommand;
pub use commands::test_connection;
pub use commands::test_connection::TestConnectionCommand;
pub use commands::update::{
    InvalidUpdate, ProtocolUpdate, UpdateMachineCommand, UpdateProtocolsCommand, UpdateSshCommand,
};
pub use connection::MachineConnectionService;
pub use errors::{
    CreateMachineError, DeleteMachineError, EnrollError, GetMachineError,
    IssueEnrollmentTokenError, ListMachinesError, OpenTerminalError, TestConnectionError,
    UnenrollError, UpdateMachineError,
};
pub use ports::{
    MachineTransport, Repository, SshKeyAlgorithm, SshKeypairs, Target, TerminalEvent,
    TerminalExit, TerminalInput, TerminalInputSink, TerminalOutputStream, TerminalSession,
    TerminalSize, TerminalType, TransportError,
};
pub use service::MachineService;
pub use use_cases::{MachineConnections, MachineEnrollment, MachineManagement};

#[cfg(test)]
pub use ports::{MockMachineTransport, MockRepository, MockSshKeypairs};
#[cfg(test)]
pub use use_cases::{MockMachineConnections, MockMachineEnrollment, MockMachineManagement};
