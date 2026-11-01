use crate::application::machine::ports::{TerminalSize, TerminalType};
use crate::domain::machine::ProtocolKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenTerminalCommand {
    pub machine_id: uuid::Uuid,
    pub protocol: ProtocolKind,
    pub size: TerminalSize,
    pub terminal: TerminalType,
}
