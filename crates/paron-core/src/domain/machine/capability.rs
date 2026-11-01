use std::collections::BTreeMap;

use super::{Machine, ProtocolKind, Protocols, ssh};

pub trait Capability: Sized {
    fn from_machine(machine: &Machine) -> BTreeMap<Provider, Result<Self, CapabilityUnavailable>>;
}

#[derive(Clone)]
pub enum ConnectionTest {
    Ssh(ssh::Access),
}

impl Capability for ConnectionTest {
    fn from_machine(machine: &Machine) -> BTreeMap<Provider, Result<Self, CapabilityUnavailable>> {
        let Protocols { ssh } = &machine.protocols;

        BTreeMap::from([(
            Provider::Protocol(ProtocolKind::Ssh),
            ssh.clone()
                .map(Self::Ssh)
                .ok_or(CapabilityUnavailable::Missing(ProtocolKind::Ssh)),
        )])
    }
}

#[derive(Clone)]
pub enum Terminal {
    Ssh(ssh::Access),
}

impl Capability for Terminal {
    fn from_machine(machine: &Machine) -> BTreeMap<Provider, Result<Self, CapabilityUnavailable>> {
        let Protocols { ssh } = &machine.protocols;

        BTreeMap::from([(
            Provider::Protocol(ProtocolKind::Ssh),
            ssh.clone()
                .map(Self::Ssh)
                .ok_or(CapabilityUnavailable::Missing(ProtocolKind::Ssh)),
        )])
    }
}

pub struct Capabilities {
    pub connection_test: Vec<Provider>,
    pub terminal: Vec<Provider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provider {
    Protocol(ProtocolKind),
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(kind) => kind.fmt(f),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityUnavailable {
    #[error("the machine has no {0} protocol")]
    Missing(ProtocolKind),

    #[error("{0} does not support this operation")]
    Unsupported(Provider),
}
