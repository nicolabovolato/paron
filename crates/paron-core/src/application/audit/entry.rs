use crate::application::{identity, machine};
use crate::domain::entity::EntityRef;
use crate::domain::machine::Machine;
use crate::domain::user::User;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry<Kind = AuditKind> {
    pub id: uuid::Uuid,
    pub created_at: jiff::Timestamp,
    pub actor: Actor,
    pub target: Entity,
    pub kind: Kind,
}

string_enum! {
    pub enum AuditEntryType {
        UserSignedIn => "user.signed_in",
        UserSignedOut => "user.signed_out",
        UserCreated => "user.created",
        UserUpdated => "user.updated",
        UserDeleted => "user.deleted",
        UserRecoveryStarted => "user.recovery_started",
        UserCredentialAdded => "user.credential_added",
        UserCredentialRemoved => "user.credential_removed",
        MachineCreated => "machine.created",
        MachineUpdated => "machine.updated",
        MachineDeleted => "machine.deleted",
        MachineTerminalOpened => "machine.terminal_opened",
        MachineTerminalClosed => "machine.terminal_closed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAuditKind {
    pub entry_type: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, derive_more::From)]
pub enum AuditKind {
    User(identity::event::AuditKind),
    Machine(machine::event::AuditKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    System,
    Entity(Entity),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub reference: EntityRef,
    pub name: String,
}

impl From<&User> for Entity {
    fn from(user: &User) -> Self {
        Self {
            reference: user.into(),
            name: user.username.to_string(),
        }
    }
}

impl From<&User> for Actor {
    fn from(user: &User) -> Self {
        Self::Entity(user.into())
    }
}

impl From<&identity::authorization::actor::Actor> for Actor {
    fn from(actor: &identity::authorization::actor::Actor) -> Self {
        match actor {
            identity::authorization::actor::Actor::System => Self::System,
            identity::authorization::actor::Actor::User(authenticated) => {
                Self::Entity((&authenticated.user).into())
            }
        }
    }
}

impl From<&Machine> for Entity {
    fn from(machine: &Machine) -> Self {
        Self {
            reference: machine.into(),
            name: machine.name.to_string(),
        }
    }
}
