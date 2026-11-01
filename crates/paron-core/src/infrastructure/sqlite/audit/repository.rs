use async_trait::async_trait;
use indoc::{formatdoc, indoc};

use super::{AuditRow, TargetType};
use crate::application::audit::{
    AuditEntry, AuditEntryType, AuditFilter, AuditKind, AuditOrder, EntityRole, Repository,
    UnknownAuditKind,
};
use crate::application::repository::{RepositoryError, TxContext};
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqliteAuditRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqliteAuditRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqliteAuditRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(
        row: AuditRow,
    ) -> Result<AuditEntry<Result<AuditKind, UnknownAuditKind>>, RepositoryError> {
        let id = row.id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context(format!("error reading audit entry {id}")),
            )
        })
    }
}

#[async_trait]
impl Repository<SqliteTx> for SqliteAuditRepository {
    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        entry: &AuditEntry,
    ) -> Result<(), RepositoryError> {
        let row = AuditRow::from_entity(entry);

        sqlx::query(indoc! {"
            INSERT INTO audit_log
                (id, created_at_unix_ms, actor_type, actor_id, actor_name,
                 target_type, target_id, target_name, type, payload)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "})
        .bind(row.id)
        .bind(row.created_at_unix_ms)
        .bind(row.actor_type)
        .bind(row.actor_id)
        .bind(row.actor_name)
        .bind(row.target_type)
        .bind(row.target_id)
        .bind(row.target_name)
        .bind(row.entry_type)
        .bind(row.payload)
        .execute(self.executing(ctx))
        .await?;

        Ok(())
    }

    async fn list(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditEntry<Result<AuditKind, UnknownAuditKind>>>, RepositoryError> {
        let target_types: Vec<TargetType> = filter
            .target_entity_kinds
            .iter()
            .copied()
            .map(Into::into)
            .collect();

        let order = match filter.order {
            AuditOrder::NewestFirst => "DESC",
            AuditOrder::OldestFirst => "ASC",
        };

        let for_entity_role = |query: &dyn Fn(&str) -> String| {
            let by_target_types = "target_type IN (SELECT value FROM json_each(?2))";
            let by_actor = format!("actor_type = ?3 AND actor_id = ?4 AND {by_target_types}");
            let by_target = format!("target_id = ?4 AND target_type = ?3 AND {by_target_types}");

            match filter.entity.map(|entity| entity.role) {
                None => query("target_type IN (SELECT value FROM json_each(?2))"),
                Some(EntityRole::Actor) => query(&by_actor),
                Some(EntityRole::Target) => query(&by_target),
                Some(EntityRole::ActorOrTarget) => formatdoc! {"
                    SELECT * FROM ({by_actor})
                    UNION
                    SELECT * FROM ({by_target})
                    ORDER BY id {order}
                    LIMIT ?7
                    ",
                    by_actor = query(&by_actor),
                    by_target = query(&by_target),
                },
            }
        };

        let rows: Vec<AuditRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(for_entity_role(&|condition| {
                formatdoc! {"
                    SELECT * FROM audit_log
                    WHERE ({cursor})
                      AND ({condition})
                      AND ({from})
                      AND ({to})
                      AND ({entry_type})
                    ORDER BY id {order}
                    LIMIT ?7
                    ",
                    cursor = match filter.order {
                        AuditOrder::NewestFirst => "id < ?1",
                        AuditOrder::OldestFirst => "id > ?1",
                    },
                    from = filter.from.map_or("TRUE", |_| "created_at_unix_ms >= ?5"),
                    to = filter.to.map_or("TRUE", |_| "created_at_unix_ms < ?6"),
                    entry_type = filter.entry_type.map_or("TRUE", |_| "type = ?8"),
                }
            })))
            .bind(filter.cursor.unwrap_or(match filter.order {
                AuditOrder::NewestFirst => uuid::Uuid::max(),
                AuditOrder::OldestFirst => uuid::Uuid::nil(),
            })) // ?1
            .bind(sqlx::types::Json(target_types)) // ?2
            .bind(
                filter
                    .entity
                    .map(|entity| TargetType::from(entity.entity.kind)),
            ) // ?3
            .bind(filter.entity.map(|entity| entity.entity.id)) // ?4
            .bind(filter.from.map(jiff::Timestamp::as_millisecond)) // ?5
            .bind(filter.to.map(jiff::Timestamp::as_millisecond)) // ?6
            .bind(filter.limit) // ?7
            .bind(filter.entry_type.map(AuditEntryType::as_str)) // ?8
            .fetch_all(self.executing(ctx))
            .await?;

        rows.into_iter().map(Self::to_entity).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::audit::{self, EntityFilter};
    use crate::application::machine::event::AuditKind as MachineAuditKind;
    use crate::application::repository::{TxGuarantee, UnitOfWork as _};
    use crate::domain::entity::{EntityKind, EntityRef};
    use crate::testing::clock::timestamp;
    use crate::testing::events::user_deleted_at;
    use crate::testing::registry::Registry;

    fn newest_first() -> AuditFilter {
        AuditFilter {
            from: None,
            to: None,
            target_entity_kinds: vec![EntityKind::User, EntityKind::Machine],
            entity: None,
            entry_type: None,
            order: AuditOrder::NewestFirst,
            cursor: None,
            limit: 10,
        }
    }

    async fn create_entries(registry: &Registry, entries: &[&AuditEntry]) {
        for entry in entries {
            registry
                .audit
                .create(&mut TxContext::none(), entry)
                .await
                .unwrap();
        }
    }

    async fn list_ids(registry: &Registry, filter: &AuditFilter) -> Vec<uuid::Uuid> {
        registry
            .audit
            .list(&mut TxContext::none(), filter)
            .await
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect()
    }

    #[tokio::test]
    async fn round_trip_preserves_every_field() {
        let registry = Registry::in_memory().await;
        let entry = user_deleted_at(timestamp(1));
        create_entries(&registry, &[&entry]).await;

        let listed = registry
            .audit
            .list(&mut TxContext::none(), &newest_first())
            .await;

        assert_eq!(
            listed.unwrap(),
            [AuditEntry {
                id: entry.id,
                created_at: entry.created_at,
                actor: entry.actor,
                target: entry.target,
                kind: Ok(entry.kind),
            }]
        );
    }

    #[tokio::test]
    async fn create_with_a_taken_id_returns_already_exists() {
        let registry = Registry::in_memory().await;
        let entry = user_deleted_at(timestamp(1));
        create_entries(&registry, &[&entry]).await;

        let created = registry.audit.create(&mut TxContext::none(), &entry).await;

        assert_matches!(created, Err(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn create_in_a_transaction_that_does_not_commit_writes_nothing() {
        let registry = Registry::in_memory().await;
        let mut ctx = registry
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await
            .unwrap();

        registry
            .audit
            .create(&mut ctx, &user_deleted_at(timestamp(1)))
            .await
            .unwrap();
        drop(ctx);

        assert_eq!(
            list_ids(&registry, &newest_first()).await,
            Vec::<uuid::Uuid>::new()
        );
    }

    #[tokio::test]
    async fn list_with_an_entry_of_an_unknown_type_returns_it_as_unknown() {
        let registry = Registry::in_memory().await;
        sqlx::query(indoc! {"
            INSERT INTO audit_log
                (id, created_at_unix_ms, actor_type, target_type, target_id, target_name,
                 type, payload)
            VALUES (?1, 1, 'system', 'user', ?1, 'nicola', 'user.from_a_later_release',
                    '{\"any\":\"thing\"}')
        "})
        .bind(uuid::Uuid::now_v7())
        .execute(&registry.pool)
        .await
        .unwrap();

        let listed = registry
            .audit
            .list(&mut TxContext::none(), &newest_first())
            .await;

        assert_eq!(
            listed
                .unwrap()
                .into_iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            [Err(UnknownAuditKind {
                entry_type: "user.from_a_later_release".to_owned(),
                payload: r#"{"any":"thing"}"#.to_owned(),
            })]
        );
    }

    #[tokio::test]
    async fn list_returns_the_newest_entries_older_than_the_cursor() {
        let registry = Registry::in_memory().await;
        let entries = [1, 2, 3, 4].map(|_| user_deleted_at(timestamp(1)));
        create_entries(&registry, &entries.each_ref()).await;

        for (cursor, expected_entries) in [
            (None, [&entries[3], &entries[2]]),
            (Some(entries[2].id), [&entries[1], &entries[0]]),
        ] {
            let listed_ids = list_ids(
                &registry,
                &AuditFilter {
                    cursor,
                    limit: 2,
                    ..newest_first()
                },
            )
            .await;

            assert_eq!(
                listed_ids,
                expected_entries.map(|entry| entry.id),
                "{cursor:?}"
            );
        }
    }

    #[tokio::test]
    async fn list_oldest_first_returns_the_entries_in_the_order_they_happened_across_pages() {
        let registry = Registry::in_memory().await;
        let entries = [1, 2, 3].map(|second| user_deleted_at(timestamp(second)));
        create_entries(&registry, &entries.each_ref()).await;
        let first_page_filter = AuditFilter {
            order: AuditOrder::OldestFirst,
            limit: 2,
            ..newest_first()
        };

        let first_page = list_ids(&registry, &first_page_filter).await;
        let second_page = list_ids(
            &registry,
            &AuditFilter {
                cursor: first_page.last().copied(),
                ..first_page_filter.clone()
            },
        )
        .await;

        assert_eq!(
            [first_page, second_page].concat(),
            entries.map(|entry| entry.id)
        );
    }

    #[tokio::test]
    async fn list_with_an_entity_returns_the_entries_where_it_has_that_role() {
        let registry = Registry::in_memory().await;
        let giulia = audit::Entity {
            reference: EntityRef {
                kind: EntityKind::User,
                id: uuid::Uuid::now_v7(),
            },
            name: "giulia".to_owned(),
        };
        let by_giulia = AuditEntry {
            actor: audit::Actor::Entity(giulia.clone()),
            ..user_deleted_at(timestamp(1))
        };
        let about_giulia = AuditEntry {
            target: giulia.clone(),
            ..user_deleted_at(timestamp(2))
        };
        let unrelated = user_deleted_at(timestamp(3));
        let by_and_about_giulia = AuditEntry {
            actor: audit::Actor::Entity(giulia.clone()),
            target: giulia.clone(),
            ..user_deleted_at(timestamp(4))
        };
        create_entries(
            &registry,
            &[&by_giulia, &about_giulia, &unrelated, &by_and_about_giulia],
        )
        .await;

        for (role, expected_entries) in [
            (EntityRole::Actor, vec![&by_and_about_giulia, &by_giulia]),
            (
                EntityRole::Target,
                vec![&by_and_about_giulia, &about_giulia],
            ),
            (
                EntityRole::ActorOrTarget,
                vec![&by_and_about_giulia, &about_giulia, &by_giulia],
            ),
        ] {
            let listed_ids = list_ids(
                &registry,
                &AuditFilter {
                    entity: Some(EntityFilter {
                        entity: giulia.reference,
                        role,
                    }),
                    ..newest_first()
                },
            )
            .await;

            assert_eq!(
                listed_ids,
                expected_entries
                    .iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>(),
                "{role:?}"
            );
        }
    }

    #[tokio::test]
    async fn list_with_one_kind_of_target_returns_only_the_entries_of_that_kind() {
        let registry = Registry::in_memory().await;
        let about_a_user = user_deleted_at(timestamp(1));
        let about_a_machine = AuditEntry {
            target: audit::Entity {
                reference: EntityRef {
                    kind: EntityKind::Machine,
                    id: uuid::Uuid::now_v7(),
                },
                name: "build".to_owned(),
            },
            kind: MachineAuditKind::Deleted.into(),
            ..user_deleted_at(timestamp(2))
        };
        create_entries(&registry, &[&about_a_user, &about_a_machine]).await;

        let listed_ids = list_ids(
            &registry,
            &AuditFilter {
                target_entity_kinds: vec![EntityKind::Machine],
                ..newest_first()
            },
        )
        .await;

        assert_eq!(listed_ids, [about_a_machine.id]);
    }

    #[tokio::test]
    async fn list_with_an_entry_type_returns_only_the_entries_of_that_type() {
        let registry = Registry::in_memory().await;
        let user_deleted = user_deleted_at(timestamp(1));
        let machine_deleted = AuditEntry {
            kind: MachineAuditKind::Deleted.into(),
            ..user_deleted_at(timestamp(2))
        };
        create_entries(&registry, &[&user_deleted, &machine_deleted]).await;

        let listed_ids = list_ids(
            &registry,
            &AuditFilter {
                entry_type: Some(AuditEntryType::MachineDeleted),
                ..newest_first()
            },
        )
        .await;

        assert_eq!(listed_ids, [machine_deleted.id]);
    }

    #[tokio::test]
    async fn list_with_a_time_range_returns_the_entries_from_the_start_to_before_the_end() {
        let registry = Registry::in_memory().await;
        let entries = [1, 2, 3, 4].map(|second| user_deleted_at(timestamp(second)));
        create_entries(&registry, &entries.each_ref()).await;

        let listed_ids = list_ids(
            &registry,
            &AuditFilter {
                from: Some(timestamp(2)),
                to: Some(timestamp(4)),
                ..newest_first()
            },
        )
        .await;

        assert_eq!(listed_ids, [entries[2].id, entries[1].id]);
    }
}
