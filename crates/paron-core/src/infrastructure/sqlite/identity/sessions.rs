use async_trait::async_trait;
use indoc::indoc;

use super::Error;
use crate::application::identity::authentication::ports::SessionRepository;
use crate::application::identity::authentication::sessions::Session;
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::token_mac::TokenMac;
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqliteSessionRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqliteSessionRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqliteSessionRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(row: SessionRow) -> Result<Session, RepositoryError> {
        let id = row.id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context(format!("error reading session {id}")),
            )
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: uuid::Uuid,
    user_id: uuid::Uuid,
    token_mac: Vec<u8>,
    issued_at_unix_ms: i64,
    recently_authenticated_until_unix_ms: i64,
    last_used_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

impl SessionRow {
    fn from_entity(session: &Session) -> Self {
        Self {
            id: session.id,
            user_id: session.user_id,
            token_mac: session.mac.to_vec(),
            issued_at_unix_ms: session.issued_at.as_millisecond(),
            recently_authenticated_until_unix_ms: session
                .recently_authenticated_until
                .as_millisecond(),
            last_used_at_unix_ms: session.last_used_at.as_millisecond(),
            expires_at_unix_ms: session.expires_at.as_millisecond(),
        }
    }

    fn try_into_entity(self) -> Result<Session, Error> {
        Ok(Session {
            id: self.id,
            user_id: self.user_id,
            issued_at: jiff::Timestamp::from_millisecond(self.issued_at_unix_ms)?,
            recently_authenticated_until: jiff::Timestamp::from_millisecond(
                self.recently_authenticated_until_unix_ms,
            )?,
            last_used_at: jiff::Timestamp::from_millisecond(self.last_used_at_unix_ms)?,
            expires_at: jiff::Timestamp::from_millisecond(self.expires_at_unix_ms)?,
            mac: TokenMac::from(self.token_mac),
        })
    }
}

#[async_trait]
impl SessionRepository<SqliteTx> for SqliteSessionRepository {
    async fn get_by_id(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<Session, RepositoryError> {
        let row: SessionRow = sqlx::query_as("SELECT * FROM user_sessions WHERE id = ?1")
            .bind(id)
            .fetch_one(self.executing(ctx))
            .await?;

        Self::to_entity(row)
    }

    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        mac: &TokenMac,
    ) -> Result<Session, RepositoryError> {
        let row: SessionRow = sqlx::query_as("SELECT * FROM user_sessions WHERE token_mac = ?1")
            .bind(&mac[..])
            .fetch_one(self.executing(ctx))
            .await?;

        Self::to_entity(row)
    }

    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        session: &Session,
    ) -> Result<Session, RepositoryError> {
        let row = SessionRow::from_entity(session);

        let row: SessionRow = sqlx::query_as(indoc! {"
            INSERT INTO user_sessions
                (id, user_id, token_mac, issued_at_unix_ms, recently_authenticated_until_unix_ms,
                 last_used_at_unix_ms, expires_at_unix_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            RETURNING *
        "})
        .bind(row.id)
        .bind(row.user_id)
        .bind(&row.token_mac)
        .bind(row.issued_at_unix_ms)
        .bind(row.recently_authenticated_until_unix_ms)
        .bind(row.last_used_at_unix_ms)
        .bind(row.expires_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn update(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        session: &Session,
    ) -> Result<Session, RepositoryError> {
        let row = SessionRow::from_entity(session);

        let row: SessionRow = sqlx::query_as(indoc! {"
            UPDATE user_sessions
            SET token_mac = ?2, issued_at_unix_ms = ?3, recently_authenticated_until_unix_ms = ?4,
                last_used_at_unix_ms = ?5, expires_at_unix_ms = ?6
            WHERE id = ?1
            RETURNING *
        "})
        .bind(row.id)
        .bind(&row.token_mac)
        .bind(row.issued_at_unix_ms)
        .bind(row.recently_authenticated_until_unix_ms)
        .bind(row.last_used_at_unix_ms)
        .bind(row.expires_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<Session, RepositoryError> {
        let row: SessionRow = sqlx::query_as("DELETE FROM user_sessions WHERE id = ?1 RETURNING *")
            .bind(id)
            .fetch_one(self.executing(ctx))
            .await?;

        Self::to_entity(row)
    }

    async fn delete_for_user(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
        keep_newest: usize,
        except: Option<uuid::Uuid>,
    ) -> Result<Vec<Session>, RepositoryError> {
        let rows: Vec<SessionRow> = sqlx::query_as(indoc! {"
            DELETE FROM user_sessions
            WHERE user_id = ?1 AND id IS NOT ?3 AND id NOT IN (
                SELECT id FROM user_sessions
                WHERE user_id = ?1
                ORDER BY issued_at_unix_ms DESC, id DESC
                LIMIT ?2
            )
            RETURNING *
        "})
        .bind(user_id)
        .bind(i64::try_from(keep_newest).unwrap_or(i64::MAX))
        .bind(except)
        .fetch_all(self.executing(ctx))
        .await?;

        rows.into_iter().map(Self::to_entity).collect()
    }

    async fn delete_expired(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        now: jiff::Timestamp,
        unused_since: jiff::Timestamp,
        limit: usize,
    ) -> Result<Vec<Session>, RepositoryError> {
        let rows: Vec<SessionRow> = sqlx::query_as(indoc! {"
            DELETE FROM user_sessions
            WHERE id IN (
                SELECT id FROM user_sessions
                WHERE expires_at_unix_ms <= ?1 OR last_used_at_unix_ms <= ?2
                LIMIT ?3
            )
            RETURNING *
        "})
        .bind(now.as_millisecond())
        .bind(unused_since.as_millisecond())
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(self.executing(ctx))
        .await?;

        rows.into_iter().map(Self::to_entity).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::BTreeSet;

    use super::*;
    use crate::application::identity::ports::UserRepository as _;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn mac(byte: u8) -> TokenMac {
        TokenMac::from(vec![byte; 32])
    }

    #[tokio::test]
    async fn round_trip_by_mac_preserves_every_field() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let session = Session {
            mac: mac(1),
            recently_authenticated_until: timestamp(30),
            last_used_at: timestamp(50),
            ..identity::session_for(&user)
        };
        registry.create_session(&session).await;

        let stored = registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &mac(1))
            .await;

        assert_eq!(stored.unwrap(), session);
    }

    #[tokio::test]
    async fn get_by_mac_unknown_returns_not_found() {
        let registry = Registry::in_memory().await;

        let stored = registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &mac(9))
            .await;

        assert_matches!(stored, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn create_with_a_taken_mac_returns_already_exists() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry.create_session(&identity::session_for(&user)).await;

        let created = registry
            .sessions
            .create(&mut TxContext::none(), &identity::session_for(&user))
            .await;

        assert_matches!(created, Err(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn update_stores_the_new_last_used_at() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let session = registry.create_session(&identity::session_for(&user)).await;
        let used_session = Session {
            last_used_at: timestamp(42),
            ..session
        };

        registry
            .sessions
            .update(&mut TxContext::none(), &used_session)
            .await
            .unwrap();

        let stored = registry
            .sessions
            .get_by_id(&mut TxContext::none(), used_session.id)
            .await;
        assert_eq!(stored.unwrap(), used_session);
    }

    #[tokio::test]
    async fn update_stores_the_new_end_of_the_recent_authentication() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let session = registry.create_session(&identity::session_for(&user)).await;
        let refreshed_session = Session {
            recently_authenticated_until: timestamp(42),
            ..session
        };

        registry
            .sessions
            .update(&mut TxContext::none(), &refreshed_session)
            .await
            .unwrap();

        let stored = registry
            .sessions
            .get_by_id(&mut TxContext::none(), refreshed_session.id)
            .await;
        assert_eq!(stored.unwrap(), refreshed_session);
    }

    #[tokio::test]
    async fn delete_for_user_spares_the_excepted_session() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let excepted_session = registry
            .create_session(&Session {
                mac: mac(1),
                ..identity::session_for(&user)
            })
            .await;
        registry
            .create_session(&Session {
                mac: mac(2),
                ..identity::session_for(&user)
            })
            .await;

        let deleted = registry
            .sessions
            .delete_for_user(
                &mut TxContext::none(),
                user.id,
                0,
                Some(excepted_session.id),
            )
            .await
            .unwrap();

        assert_eq!(deleted.len(), 1);
        let stored = registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &mac(1))
            .await;
        assert_eq!(stored.unwrap(), excepted_session);
    }

    #[tokio::test]
    async fn delete_for_user_keeps_the_newest_sessions_of_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let other_user = registry
            .create_user(&identity::user_builder().username("bruno").build())
            .await;
        let oldest = registry
            .create_session(&Session {
                mac: mac(1),
                issued_at: timestamp(10),
                ..identity::session_for(&user)
            })
            .await;
        for (mac_byte, seconds) in [(2, 20), (3, 30)] {
            registry
                .create_session(&Session {
                    mac: mac(mac_byte),
                    issued_at: timestamp(seconds),
                    ..identity::session_for(&user)
                })
                .await;
        }
        let session_of_other_user = registry
            .create_session(&Session {
                mac: mac(4),
                issued_at: timestamp(0),
                ..identity::session_for(&other_user)
            })
            .await;

        let deleted = registry
            .sessions
            .delete_for_user(&mut TxContext::none(), user.id, 2, None)
            .await
            .unwrap();

        assert_eq!(deleted, [oldest]);
        let stored = registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &mac(4))
            .await;
        assert_eq!(stored.unwrap(), session_of_other_user);
    }

    #[tokio::test]
    async fn delete_expired_returns_the_sessions_past_expiry_or_unused() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let past_expiry = registry
            .create_session(&Session {
                mac: mac(1),
                last_used_at: timestamp(90),
                expires_at: timestamp(100),
                ..identity::session_for(&user)
            })
            .await;
        let unused = registry
            .create_session(&Session {
                mac: mac(2),
                last_used_at: timestamp(50),
                expires_at: timestamp(10_000),
                ..identity::session_for(&user)
            })
            .await;
        let live = registry
            .create_session(&Session {
                mac: mac(3),
                last_used_at: timestamp(90),
                expires_at: timestamp(10_000),
                ..identity::session_for(&user)
            })
            .await;

        let deleted = registry
            .sessions
            .delete_expired(&mut TxContext::none(), timestamp(100), timestamp(60), 10)
            .await
            .unwrap();

        assert_eq!(
            deleted
                .iter()
                .map(|session| session.id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([past_expiry.id, unused.id])
        );
        let stored = registry
            .sessions
            .get_by_mac(&mut TxContext::none(), &mac(3))
            .await;
        assert_eq!(stored.unwrap(), live);
    }

    #[tokio::test]
    async fn delete_expired_over_the_limit_leaves_the_rest_for_the_next_call() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        for mac_byte in [1, 2, 3] {
            registry
                .create_session(&Session {
                    mac: mac(mac_byte),
                    ..identity::session_for(&user)
                })
                .await;
        }
        let after_expiry = identity::session_for(&user).expires_at;

        let first_batch = registry
            .sessions
            .delete_expired(&mut TxContext::none(), after_expiry, timestamp(0), 2)
            .await
            .unwrap();
        let second_batch = registry
            .sessions
            .delete_expired(&mut TxContext::none(), after_expiry, timestamp(0), 2)
            .await
            .unwrap();

        assert_eq!((first_batch.len(), second_batch.len()), (2, 1));
    }

    #[tokio::test]
    async fn delete_user_removes_the_sessions_of_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let session = registry.create_session(&identity::session_for(&user)).await;

        registry
            .users
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let stored = registry
            .sessions
            .get_by_id(&mut TxContext::none(), session.id)
            .await;
        assert_matches!(stored, Err(RepositoryError::NotFound));
    }
}
