use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;

use super::entry::{self, AuditEntry, AuditEntryType, AuditKind, Entity, UnknownAuditKind};
use super::errors::ListAuditEntriesError;
use super::ports::{AuditFilter, AuditOrder, EntityFilter, EntityRole, Repository};
use super::use_cases::AuditReader;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::repository::TxContext;
use crate::domain::entity::EntityKind;
use crate::domain::user::Permission;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    pub from: Option<jiff::Timestamp>,
    pub to: Option<jiff::Timestamp>,
    pub entity: Option<EntityFilter>,
    pub entry_type: Option<AuditEntryType>,

    pub order: AuditOrder,
    pub cursor: Option<uuid::Uuid>,
    pub limit: u32,
}

pub struct AuditService<Tx> {
    audit: Arc<dyn Repository<Tx>>,
    clock: Arc<dyn Clock>,
}

impl<Tx: Send + Sync + 'static> AuditService<Tx> {
    pub fn new(audit: Arc<dyn Repository<Tx>>, clock: Arc<dyn Clock>) -> Self {
        Self { audit, clock }
    }

    pub async fn append(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: impl Into<entry::Actor>,
        target: impl Into<Entity>,
        kind: impl Into<AuditKind>,
    ) -> Result<(), anyhow::Error> {
        let entry = AuditEntry {
            id: uuid::Uuid::now_v7(),
            created_at: self.clock.now(),
            actor: actor.into(),
            target: target.into(),
            kind: kind.into(),
        };

        Ok(self.audit.create(ctx, &entry).await?)
    }

    pub async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        query: &AuditQuery,
    ) -> Result<Vec<AuditEntry<Result<AuditKind, UnknownAuditKind>>>, ListAuditEntriesError> {
        let authorized_target_entity_kinds: Vec<EntityKind> = [
            (EntityKind::User, Permission::UserRead),
            (EntityKind::Machine, Permission::MachineRead),
        ]
        .into_iter()
        .filter(|(_, permission)| actor.authorize(*permission).is_ok())
        .map(|(kind, _)| kind)
        .collect();

        let entity = match query.entity {
            Some(filter) if !authorized_target_entity_kinds.contains(&filter.entity.kind) => {
                match filter.role {
                    EntityRole::Target => return Ok(Vec::new()),
                    EntityRole::ActorOrTarget | EntityRole::Actor => Some(EntityFilter {
                        role: EntityRole::Actor,
                        ..filter
                    }),
                }
            }
            entity => entity,
        };

        Ok(self
            .audit
            .list(
                ctx,
                &AuditFilter {
                    from: query.from,
                    to: query.to,
                    target_entity_kinds: authorized_target_entity_kinds,
                    entity,
                    entry_type: query.entry_type,
                    order: query.order,
                    cursor: query.cursor,
                    limit: query.limit,
                },
            )
            .await
            .context("could not list audit entries")?)
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> AuditReader for AuditService<Tx> {
    async fn list(
        &self,
        actor: &Actor,
        query: &AuditQuery,
    ) -> Result<Vec<AuditEntry<Result<AuditKind, UnknownAuditKind>>>, ListAuditEntriesError> {
        Self::list(self, &mut TxContext::none(), actor, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::audit::MockRepository;
    use crate::domain::entity::EntityRef;
    use crate::domain::user::Role;
    use crate::testing::clock::clock_at;
    use crate::testing::identity;

    const USER: EntityRef = EntityRef {
        kind: EntityKind::User,
        id: uuid::Uuid::from_u128(1),
    };

    fn service_on(audit: MockRepository<()>) -> AuditService<()> {
        AuditService::new(Arc::new(audit), clock_at(jiff::Timestamp::UNIX_EPOCH))
    }

    fn query_about(entity: Option<EntityFilter>) -> AuditQuery {
        AuditQuery {
            from: None,
            to: None,
            entity,
            entry_type: None,
            order: AuditOrder::NewestFirst,
            cursor: None,
            limit: 10,
        }
    }

    #[tokio::test]
    async fn list_limits_target_kinds_to_what_the_actor_can_read() {
        for (roles, readable_kinds) in [
            (vec![], vec![]),
            (vec![Role::Operator], vec![EntityKind::Machine]),
            (
                vec![Role::Administrator],
                vec![EntityKind::User, EntityKind::Machine],
            ),
        ] {
            let mut audit = MockRepository::new();
            audit
                .expect_list()
                .once()
                .withf(move |_, filter| filter.target_entity_kinds == readable_kinds)
                .returning(|_, _| Ok(Vec::new()));

            service_on(audit)
                .list(
                    &mut TxContext::none(),
                    &identity::actor_with_roles(&roles),
                    &query_about(None),
                )
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn list_about_readable_entity_keeps_the_entity_role() {
        let entity = EntityFilter {
            entity: USER,
            role: EntityRole::ActorOrTarget,
        };
        let mut audit = MockRepository::new();
        audit
            .expect_list()
            .once()
            .withf(move |_, filter| filter.entity == Some(entity))
            .returning(|_, _| Ok(Vec::new()));

        service_on(audit)
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Administrator]),
                &query_about(Some(entity)),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_about_unreadable_entity_as_actor_or_target_lists_it_only_as_actor() {
        let mut audit = MockRepository::new();
        audit
            .expect_list()
            .once()
            .withf(|_, filter| {
                filter.entity
                    == Some(EntityFilter {
                        entity: USER,
                        role: EntityRole::Actor,
                    })
            })
            .returning(|_, _| Ok(Vec::new()));

        service_on(audit)
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Operator]),
                &query_about(Some(EntityFilter {
                    entity: USER,
                    role: EntityRole::ActorOrTarget,
                })),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_about_unreadable_entity_as_target_returns_no_entries() {
        let service = service_on(MockRepository::new());

        let entries = service
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Operator]),
                &query_about(Some(EntityFilter {
                    entity: USER,
                    role: EntityRole::Target,
                })),
            )
            .await
            .unwrap();

        assert_eq!(entries, Vec::new());
    }
}
