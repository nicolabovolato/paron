use super::machine::Machine;
use super::user::User;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    User,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRef {
    pub kind: EntityKind,
    pub id: uuid::Uuid,
}

impl From<&User> for EntityRef {
    fn from(user: &User) -> Self {
        Self {
            kind: EntityKind::User,
            id: user.id,
        }
    }
}

impl From<&Machine> for EntityRef {
    fn from(machine: &Machine) -> Self {
        Self {
            kind: EntityKind::Machine,
            id: machine.id,
        }
    }
}
