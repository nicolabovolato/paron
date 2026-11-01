use async_trait::async_trait;

use super::entry::{AuditEntry, AuditKind, UnknownAuditKind};
use super::errors::ListAuditEntriesError;
use super::service::AuditQuery;
use crate::application::identity::authorization::actor::Actor;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AuditReader: Send + Sync {
    async fn list(
        &self,
        actor: &Actor,
        query: &AuditQuery,
    ) -> Result<Vec<AuditEntry<Result<AuditKind, UnknownAuditKind>>>, ListAuditEntriesError>;
}
