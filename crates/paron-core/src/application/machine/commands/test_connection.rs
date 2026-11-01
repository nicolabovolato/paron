use super::create::CreateMachineCommand;
use super::update::UpdateMachineCommand;
use crate::domain::machine::ProtocolKind;

pub struct TestConnectionCommand {
    pub protocol: ProtocolKind,
    pub machine: Machine,
}

pub enum Machine {
    Stored(uuid::Uuid),
    Changed(UpdateMachineCommand),
    New(CreateMachineCommand),
}
