use crate::application::{identity, machine};
use crate::domain::entity::{EntityKind, EntityRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event<K: EntityEvent> {
    pub id: uuid::Uuid,
    pub created_at: jiff::Timestamp,
    pub entity_id: uuid::Uuid,
    pub kind: K,
}

impl<K: EntityEvent> Event<K> {
    pub fn entity(&self) -> EntityRef {
        EntityRef {
            kind: K::ENTITY,
            id: self.entity_id,
        }
    }
}

pub trait EntityEvent {
    const ENTITY: EntityKind;
}

#[derive(Debug, Clone, PartialEq, Eq, derive_more::From, derive_more::TryInto)]
pub enum AnyEvent {
    User(Event<identity::event::EventKind>),
    Machine(Event<machine::event::EventKind>),
}

impl AnyEvent {
    pub fn entity(&self) -> EntityRef {
        match self {
            Self::User(event) => event.entity(),
            Self::Machine(event) => event.entity(),
        }
    }
}
