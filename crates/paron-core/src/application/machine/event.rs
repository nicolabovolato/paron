use std::collections::BTreeSet;
use std::num::NonZeroU16;

use crate::application::events::EntityEvent;
use crate::domain::entity::EntityKind;
use crate::domain::machine::{self, Machine, ProtocolKind, ssh};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMethod {
    Password,
    ManagedKeypair,
    ImportedKeypair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentSnapshot {
    NotEnrolled,
    Invited { expires_at: jiff::Timestamp },
    Enrolled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectivitySnapshot {
    Direct { host: String },
    Tunneled { enrollment: EnrollmentSnapshot },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSnapshot {
    pub user: String,
    pub port: NonZeroU16,
    pub host_key_fingerprint: String,
    pub authentication: AuthenticationMethod,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolsSnapshot {
    pub ssh: Option<SshSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSnapshot {
    pub name: String,
    pub connectivity: ConnectivitySnapshot,
    pub protocols: ProtocolsSnapshot,
}

impl From<&ssh::Access> for SshSnapshot {
    fn from(access: &ssh::Access) -> Self {
        Self {
            user: access.user.to_string(),
            port: access.port,
            host_key_fingerprint: access.authentication.host_key().fingerprint(),
            authentication: match access.authentication.method() {
                ssh::AuthMethod::Password(_) => AuthenticationMethod::Password,
                ssh::AuthMethod::Keypair {
                    source: ssh::KeySource::Managed,
                    ..
                } => AuthenticationMethod::ManagedKeypair,
                ssh::AuthMethod::Keypair {
                    source: ssh::KeySource::Imported,
                    ..
                } => AuthenticationMethod::ImportedKeypair,
            },
        }
    }
}

impl From<&Machine> for MachineSnapshot {
    fn from(machine: &Machine) -> Self {
        Self {
            name: machine.name.to_string(),
            connectivity: match &machine.connectivity {
                machine::Connectivity::Direct { host } => ConnectivitySnapshot::Direct {
                    host: host.to_string(),
                },
                machine::Connectivity::Tunneled(enrollment) => ConnectivitySnapshot::Tunneled {
                    enrollment: match enrollment {
                        machine::tunnel::Enrollment::NotEnrolled => EnrollmentSnapshot::NotEnrolled,
                        machine::tunnel::Enrollment::Invited { expires_at, .. } => {
                            EnrollmentSnapshot::Invited {
                                expires_at: *expires_at,
                            }
                        }
                        machine::tunnel::Enrollment::Enrolled { .. } => {
                            EnrollmentSnapshot::Enrolled
                        }
                    },
                },
            },
            protocols: ProtocolsSnapshot {
                ssh: machine.protocols.ssh.as_ref().map(SshSnapshot::from),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Deleted,
    Updated {
        connectivity_changed: bool,
        protocols_changed: BTreeSet<ProtocolKind>,
        enrollment_ended: bool,
    },
}

impl EntityEvent for EventKind {
    const ENTITY: EntityKind = EntityKind::Machine;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditKind {
    Created(MachineSnapshot),
    Updated {
        before: MachineSnapshot,
        after: MachineSnapshot,
        credentials_replaced: BTreeSet<ProtocolKind>,
    },
    Deleted,
    TerminalOpened {
        session_id: uuid::Uuid,
        protocol: ProtocolKind,
    },
    TerminalClosed {
        session_id: uuid::Uuid,
    },
}
