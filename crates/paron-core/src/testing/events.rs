use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;

use crate::application::audit::{
    self, Actor, AuditEntry, AuditFilter, AuditService, Entity, MockRepository, Repository,
    UnknownAuditKind,
};
use crate::application::clock::Clock;
use crate::application::events::EventService;
use crate::application::identity::event::AuditKind;
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::entity::{EntityKind, EntityRef};
use crate::infrastructure::in_memory::events::InMemoryEventStream;
use crate::testing::clock::clock_at;

/// An audit log that also keeps each record that it was given, in order, for a test to read.
/// It keeps a record also if the transaction does not commit.
pub struct RecordedAudit<Tx> {
    audit: Arc<dyn Repository<Tx>>,
    recorded: watch::Sender<Vec<AuditEntry>>,
}

impl<Tx> RecordedAudit<Tx> {
    pub fn new(audit: Arc<dyn Repository<Tx>>) -> Self {
        Self {
            audit,
            recorded: watch::Sender::new(Vec::new()),
        }
    }

    pub fn recorded(&self) -> Vec<AuditEntry> {
        self.recorded.borrow().clone()
    }

    pub async fn wait_for_recorded(&self, count: usize) -> Vec<AuditEntry> {
        self.recorded
            .subscribe()
            .wait_for(|recorded| recorded.len() >= count)
            .await
            .unwrap()
            .clone()
    }

    pub fn recorded_kinds(&self) -> Vec<audit::AuditKind> {
        self.recorded()
            .into_iter()
            .map(|record| record.kind)
            .collect()
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> Repository<Tx> for RecordedAudit<Tx> {
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        record: &AuditEntry,
    ) -> Result<(), RepositoryError> {
        self.audit.create(ctx, record).await?;
        self.recorded
            .send_modify(|recorded| recorded.push(record.clone()));

        Ok(())
    }

    async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditEntry<Result<audit::AuditKind, UnknownAuditKind>>>, RepositoryError> {
        self.audit.list(ctx, filter).await
    }
}

/// For a test with no database that reads what the service recorded.
pub fn recorded_audit<Tx: Send + Sync + 'static>() -> Arc<RecordedAudit<Tx>> {
    let mut audit = MockRepository::new();
    audit.expect_create().returning(|_, _| Ok(()));

    Arc::new(RecordedAudit::new(Arc::new(audit)))
}

/// For a test that does not read the audit log.
pub fn audit_service<Tx: Send + Sync + 'static>() -> Arc<AuditService<Tx>> {
    audit_service_with_clock(clock_at(jiff::Timestamp::UNIX_EPOCH))
}

/// For a test with no database: the records go nowhere, and the list has one entry.
pub fn audit_service_with_clock<Tx: Send + Sync + 'static>(
    clock: Arc<dyn Clock>,
) -> Arc<AuditService<Tx>> {
    let mut audit = MockRepository::new();
    audit.expect_create().returning(|_, _| Ok(()));
    audit.expect_list().returning(|_, _| {
        let entry = user_deleted_at(jiff::Timestamp::UNIX_EPOCH);

        Ok(vec![AuditEntry {
            id: entry.id,
            created_at: entry.created_at,
            actor: entry.actor,
            target: entry.target,
            kind: Ok(entry.kind),
        }])
    });

    Arc::new(AuditService::new(Arc::new(audit), clock))
}

/// An administrator deletes a user. Each call gives a new record, with new ids.
pub fn user_deleted_at(created_at: jiff::Timestamp) -> AuditEntry {
    AuditEntry {
        id: uuid::Uuid::now_v7(),
        created_at,
        actor: Actor::Entity(Entity {
            reference: EntityRef {
                kind: EntityKind::User,
                id: uuid::Uuid::now_v7(),
            },
            name: "administrator".to_owned(),
        }),
        target: Entity {
            reference: EntityRef {
                kind: EntityKind::User,
                id: uuid::Uuid::now_v7(),
            },
            name: "giulia".to_owned(),
        },
        kind: AuditKind::Deleted.into(),
    }
}

/// For a test that does not read the events.
pub fn event_service() -> Arc<EventService> {
    event_service_on(
        Arc::new(InMemoryEventStream::default()),
        clock_at(jiff::Timestamp::UNIX_EPOCH),
    )
}

pub fn event_service_on(
    stream: Arc<InMemoryEventStream>,
    clock: Arc<dyn Clock>,
) -> Arc<EventService> {
    Arc::new(EventService::new(
        Arc::clone(&stream) as Arc<_>,
        stream,
        clock,
    ))
}
