use async_trait::async_trait;

use super::entry::{AuditEntry, AuditEntryType, AuditKind, UnknownAuditKind};
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::entity::{EntityKind, EntityRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditFilter {
    pub from: Option<jiff::Timestamp>,
    pub to: Option<jiff::Timestamp>,
    pub target_entity_kinds: Vec<EntityKind>,
    pub entity: Option<EntityFilter>,
    pub entry_type: Option<AuditEntryType>,

    pub order: AuditOrder,
    pub cursor: Option<uuid::Uuid>,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOrder {
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityFilter {
    pub entity: EntityRef,
    pub role: EntityRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityRole {
    ActorOrTarget,
    Actor,
    Target,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        event: &AuditEntry,
    ) -> Result<(), RepositoryError>;

    async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditEntry<Result<AuditKind, UnknownAuditKind>>>, RepositoryError>;
}
