use async_trait::async_trait;
use indoc::indoc;

use super::Error;
use crate::application::identity::authentication::account_recovery::PendingAccountRecovery;
use crate::application::identity::authentication::ports::PendingAccountRecoveryRepository;
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::token_mac::TokenMac;
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqlitePendingAccountRecoveryRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqlitePendingAccountRecoveryRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqlitePendingAccountRecoveryRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(
        row: PendingAccountRecoveryRow,
    ) -> Result<PendingAccountRecovery, RepositoryError> {
        let user_id = row.user_id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(anyhow::Error::from_boxed(error).context(format!(
                "error reading the account recovery of user {user_id}"
            )))
        })
    }
}

#[derive(sqlx::FromRow)]
struct PendingAccountRecoveryRow {
    user_id: uuid::Uuid,
    token_mac: Vec<u8>,
    expires_at_unix_ms: i64,
}

impl PendingAccountRecoveryRow {
    fn from_entity(recovery: &PendingAccountRecovery) -> Self {
        Self {
            user_id: recovery.user_id,
            token_mac: recovery.mac.to_vec(),
            expires_at_unix_ms: recovery.expires_at.as_millisecond(),
        }
    }

    fn try_into_entity(self) -> Result<PendingAccountRecovery, Error> {
        Ok(PendingAccountRecovery {
            user_id: self.user_id,
            mac: TokenMac::from(self.token_mac),
            expires_at: jiff::Timestamp::from_millisecond(self.expires_at_unix_ms)?,
        })
    }
}

#[async_trait]
impl PendingAccountRecoveryRepository<SqliteTx> for SqlitePendingAccountRecoveryRepository {
    async fn upsert(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        recovery: &PendingAccountRecovery,
    ) -> Result<PendingAccountRecovery, RepositoryError> {
        let row = PendingAccountRecoveryRow::from_entity(recovery);

        let row: PendingAccountRecoveryRow = sqlx::query_as(indoc! {"
            INSERT INTO user_pending_account_recovery (user_id, token_mac, expires_at_unix_ms)
            VALUES (?1, ?2, ?3)
            ON CONFLICT (user_id) DO UPDATE SET
                token_mac = excluded.token_mac,
                expires_at_unix_ms = excluded.expires_at_unix_ms
            RETURNING *
        "})
        .bind(row.user_id)
        .bind(&row.token_mac)
        .bind(row.expires_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        mac: &TokenMac,
    ) -> Result<PendingAccountRecovery, RepositoryError> {
        let row: PendingAccountRecoveryRow =
            sqlx::query_as("SELECT * FROM user_pending_account_recovery WHERE token_mac = ?1")
                .bind(&mac[..])
                .fetch_one(self.executing(ctx))
                .await?;

        Self::to_entity(row)
    }

    async fn delete_by_mac(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        mac: &TokenMac,
    ) -> Result<PendingAccountRecovery, RepositoryError> {
        let row: PendingAccountRecoveryRow = sqlx::query_as(
            "DELETE FROM user_pending_account_recovery WHERE token_mac = ?1 RETURNING *",
        )
        .bind(&mac[..])
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::identity::ports::UserRepository as _;
    use crate::domain::user::User;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn mac(byte: u8) -> TokenMac {
        TokenMac::from(vec![byte; 32])
    }

    fn recovery_of(user: &User, mac_byte: u8) -> PendingAccountRecovery {
        PendingAccountRecovery {
            user_id: user.id,
            mac: mac(mac_byte),
            expires_at: timestamp(100),
        }
    }

    #[tokio::test]
    async fn delete_by_mac_twice_returns_the_recovery_then_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .pending_account_recoveries
            .upsert(&mut TxContext::none(), &recovery_of(&user, 1))
            .await
            .unwrap();

        let first_delete = registry
            .pending_account_recoveries
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;
        let second_delete = registry
            .pending_account_recoveries
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;

        assert_eq!(first_delete.unwrap(), recovery_of(&user, 1));
        assert_matches!(second_delete, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn upsert_for_a_user_with_a_recovery_replaces_the_old_recovery() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .pending_account_recoveries
            .upsert(&mut TxContext::none(), &recovery_of(&user, 1))
            .await
            .unwrap();

        registry
            .pending_account_recoveries
            .upsert(&mut TxContext::none(), &recovery_of(&user, 2))
            .await
            .unwrap();

        let old_recovery = registry
            .pending_account_recoveries
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;
        let new_recovery = registry
            .pending_account_recoveries
            .delete_by_mac(&mut TxContext::none(), &mac(2))
            .await;
        assert_matches!(old_recovery, Err(RepositoryError::NotFound));
        assert_eq!(new_recovery.unwrap(), recovery_of(&user, 2));
    }

    #[tokio::test]
    async fn delete_of_the_user_removes_the_recovery() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .pending_account_recoveries
            .upsert(&mut TxContext::none(), &recovery_of(&user, 1))
            .await
            .unwrap();

        registry
            .users
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let taken = registry
            .pending_account_recoveries
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;
        assert_matches!(taken, Err(RepositoryError::NotFound));
    }
}
