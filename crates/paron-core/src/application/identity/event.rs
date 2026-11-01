use std::collections::BTreeSet;

use crate::application::events::EntityEvent;
use crate::application::identity::authentication::methods::AuthenticationMethod;
use crate::domain::entity::EntityKind;
use crate::domain::user::{Role, User, UserStatus, Username};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSnapshot {
    pub username: Username,
    pub display_name: String,
    pub roles: BTreeSet<Role>,
    pub status: UserStatus,
}

impl From<&User> for UserSnapshot {
    fn from(user: &User) -> Self {
        Self {
            username: user.username.clone(),
            display_name: user.display_name.to_string(),
            roles: user.roles.clone(),
            status: user.status,
        }
    }
}

/// The target of each of these events is the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditKind {
    SignedIn {
        session_id: uuid::Uuid,
        methods: Vec<AuthenticationMethod>,
    },
    SignedOut {
        session_id: uuid::Uuid,
    },
    Created(UserSnapshot),
    Updated {
        before: UserSnapshot,
        after: UserSnapshot,
    },
    Deleted,
    RecoveryStarted {
        expires_at: jiff::Timestamp,
    },
    CredentialAdded {
        kind: AuthenticationMethod,
    },
    CredentialRemoved {
        kind: AuthenticationMethod,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    SessionsDeleted { session_ids: Vec<uuid::Uuid> },
    RolesChanged { roles: BTreeSet<Role> },
}

impl EntityEvent for EventKind {
    const ENTITY: EntityKind = EntityKind::User;
}
