use async_trait::async_trait;
use indoc::indoc;

use super::Error;
use crate::application::identity::authentication::failed_sign_in_attempts::{
    AttemptKey, FailedSignInAttempts, ThrottledMethod,
};
use crate::application::identity::authentication::methods::AuthenticationMethod;
use crate::application::identity::authentication::ports::FailedSignInAttemptRepository;
use crate::application::repository::{RepositoryError, TxContext};
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqliteFailedSignInAttemptRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqliteFailedSignInAttemptRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqliteFailedSignInAttemptRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(row: FailedSignInAttemptsRow) -> Result<FailedSignInAttempts, RepositoryError> {
        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context("error reading failed attempts"),
            )
        })
    }
}

#[derive(sqlx::FromRow)]
struct FailedSignInAttemptsRow {
    method: String,
    user_id: uuid::Uuid,
    device_id: Option<uuid::Uuid>,
    attempts: i64,
    next_attempt_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

impl FailedSignInAttemptsRow {
    fn from_entity(attempts: &FailedSignInAttempts) -> Self {
        Self {
            method: AuthenticationMethod::from(attempts.key.method)
                .as_str()
                .to_owned(),
            user_id: attempts.key.user_id,
            device_id: attempts.key.device_id,
            attempts: i64::from(attempts.count),
            next_attempt_at_unix_ms: attempts.next_attempt_at.as_millisecond(),
            expires_at_unix_ms: attempts.expires_at.as_millisecond(),
        }
    }

    fn try_into_entity(self) -> Result<FailedSignInAttempts, Error> {
        let method = ThrottledMethod::try_from(self.method.parse::<AuthenticationMethod>()?)
            .map_err(|method| format!("{} is not a throttled method", method.as_str()))?;

        Ok(FailedSignInAttempts {
            key: AttemptKey {
                method,
                user_id: self.user_id,
                device_id: self.device_id,
            },
            count: u32::try_from(self.attempts)?,
            next_attempt_at: jiff::Timestamp::from_millisecond(self.next_attempt_at_unix_ms)?,
            expires_at: jiff::Timestamp::from_millisecond(self.expires_at_unix_ms)?,
        })
    }
}

#[async_trait]
impl FailedSignInAttemptRepository<SqliteTx> for SqliteFailedSignInAttemptRepository {
    async fn get(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        key: &AttemptKey,
    ) -> Result<FailedSignInAttempts, RepositoryError> {
        let row: FailedSignInAttemptsRow = sqlx::query_as(indoc! {"
            SELECT * FROM user_failed_sign_in_attempts
            WHERE method = ?1 AND user_id = ?2 AND device_id IS ?3
        "})
        .bind(AuthenticationMethod::from(key.method).as_str())
        .bind(key.user_id)
        .bind(key.device_id)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn upsert(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        attempts: &FailedSignInAttempts,
    ) -> Result<FailedSignInAttempts, RepositoryError> {
        let row = FailedSignInAttemptsRow::from_entity(attempts);

        let row: FailedSignInAttemptsRow = sqlx::query_as(indoc! {"
            INSERT INTO user_failed_sign_in_attempts
                (method, user_id, device_id, attempts, next_attempt_at_unix_ms, expires_at_unix_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT DO UPDATE SET
                attempts = excluded.attempts,
                next_attempt_at_unix_ms = excluded.next_attempt_at_unix_ms,
                expires_at_unix_ms = excluded.expires_at_unix_ms
            RETURNING *
        "})
        .bind(&row.method)
        .bind(row.user_id)
        .bind(row.device_id)
        .bind(row.attempts)
        .bind(row.next_attempt_at_unix_ms)
        .bind(row.expires_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        key: &AttemptKey,
    ) -> Result<FailedSignInAttempts, RepositoryError> {
        let row: FailedSignInAttemptsRow = sqlx::query_as(indoc! {"
            DELETE FROM user_failed_sign_in_attempts
            WHERE method = ?1 AND user_id = ?2 AND device_id IS ?3
            RETURNING *
        "})
        .bind(AuthenticationMethod::from(key.method).as_str())
        .bind(key.user_id)
        .bind(key.device_id)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete_expired(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        now: jiff::Timestamp,
    ) -> Result<u64, RepositoryError> {
        let deleted =
            sqlx::query("DELETE FROM user_failed_sign_in_attempts WHERE expires_at_unix_ms <= ?1")
                .bind(now.as_millisecond())
                .execute(self.executing(ctx))
                .await?;

        Ok(deleted.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::identity::authentication::failed_sign_in_attempts::ThrottledMethod;
    use crate::domain::user::User;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn totp_key(user: &User) -> AttemptKey {
        AttemptKey {
            method: ThrottledMethod::Totp,
            user_id: user.id,
            device_id: None,
        }
    }

    fn totp_attempts(user: &User, count: u32, seconds: i64) -> FailedSignInAttempts {
        FailedSignInAttempts {
            key: totp_key(user),
            count,
            next_attempt_at: timestamp(seconds),
            expires_at: timestamp(seconds + 100),
        }
    }

    #[tokio::test]
    async fn get_after_upsert_returns_the_last_upsert() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        for attempts in [totp_attempts(&user, 1, 10), totp_attempts(&user, 2, 20)] {
            registry
                .failed_sign_in_attempts
                .upsert(&mut TxContext::none(), &attempts)
                .await
                .unwrap();
        }

        let found = registry
            .failed_sign_in_attempts
            .get(&mut TxContext::none(), &totp_key(&user))
            .await;

        assert_eq!(found.unwrap(), totp_attempts(&user, 2, 20));
    }

    #[tokio::test]
    async fn get_for_a_device_does_not_return_the_attempts_of_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .failed_sign_in_attempts
            .upsert(&mut TxContext::none(), &totp_attempts(&user, 1, 10))
            .await
            .unwrap();

        let found = registry
            .failed_sign_in_attempts
            .get(
                &mut TxContext::none(),
                &AttemptKey {
                    device_id: Some(uuid::Uuid::now_v7()),
                    ..totp_key(&user)
                },
            )
            .await;

        assert_matches!(found, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_twice_returns_the_attempts_then_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .failed_sign_in_attempts
            .upsert(&mut TxContext::none(), &totp_attempts(&user, 1, 10))
            .await
            .unwrap();

        let first_delete = registry
            .failed_sign_in_attempts
            .delete(&mut TxContext::none(), &totp_key(&user))
            .await;
        let second_delete = registry
            .failed_sign_in_attempts
            .delete(&mut TxContext::none(), &totp_key(&user))
            .await;

        assert_eq!(first_delete.unwrap(), totp_attempts(&user, 1, 10));
        assert_matches!(second_delete, Err(RepositoryError::NotFound));
    }
}
