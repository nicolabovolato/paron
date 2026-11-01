use async_trait::async_trait;
use indoc::indoc;
use serde_with::hex::Hex;

use super::Error;
use crate::application::identity::authentication::methods::ports::RecoveryCodeRepository;
use crate::application::identity::authentication::methods::recovery_code::RecoveryCodeCredential;
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::token_mac::TokenMac;
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqliteRecoveryCodeRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqliteRecoveryCodeRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqliteRecoveryCodeRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(row: RecoveryCodesRow) -> Result<RecoveryCodeCredential, RepositoryError> {
        let user_id = row.user_id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(anyhow::Error::from_boxed(error).context(format!(
                "error reading the recovery codes of user {user_id}"
            )))
        })
    }
}

#[derive(sqlx::FromRow)]
struct RecoveryCodesRow {
    user_id: uuid::Uuid,
    #[sqlx(json)]
    codes: Vec<RecoveryCodeJson>,
    generated_at_unix_ms: i64,
}

#[serde_with::serde_as]
#[derive(serde::Serialize, serde::Deserialize)]
struct RecoveryCodeJson {
    #[serde_as(as = "Hex")]
    mac: Vec<u8>,
}

impl RecoveryCodesRow {
    fn from_entity(credential: &RecoveryCodeCredential) -> Self {
        Self {
            user_id: credential.user_id,
            codes: credential
                .code_macs
                .iter()
                .map(RecoveryCodeJson::from)
                .collect(),
            generated_at_unix_ms: credential.generated_at.as_millisecond(),
        }
    }

    fn try_into_entity(self) -> Result<RecoveryCodeCredential, Error> {
        Ok(RecoveryCodeCredential {
            user_id: self.user_id,
            code_macs: self
                .codes
                .into_iter()
                .map(|code| TokenMac::from(code.mac))
                .collect(),
            generated_at: jiff::Timestamp::from_millisecond(self.generated_at_unix_ms)?,
        })
    }
}

impl From<&TokenMac> for RecoveryCodeJson {
    fn from(mac: &TokenMac) -> Self {
        Self { mac: mac.to_vec() }
    }
}

#[async_trait]
impl RecoveryCodeRepository<SqliteTx> for SqliteRecoveryCodeRepository {
    async fn get(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<RecoveryCodeCredential, RepositoryError> {
        let row: RecoveryCodesRow =
            sqlx::query_as("SELECT * FROM user_credential_recovery_codes WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(self.executing(ctx))
                .await?;

        Self::to_entity(row)
    }

    async fn upsert(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        credential: &RecoveryCodeCredential,
    ) -> Result<RecoveryCodeCredential, RepositoryError> {
        let row = RecoveryCodesRow::from_entity(credential);

        let row: RecoveryCodesRow = sqlx::query_as(indoc! {"
            INSERT INTO user_credential_recovery_codes (user_id, codes, generated_at_unix_ms)
            VALUES (?1, ?2, ?3)
            ON CONFLICT (user_id) DO UPDATE SET
                codes = excluded.codes,
                generated_at_unix_ms = excluded.generated_at_unix_ms
            RETURNING *
        "})
        .bind(row.user_id)
        .bind(sqlx::types::Json(&row.codes))
        .bind(row.generated_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<RecoveryCodeCredential, RepositoryError> {
        let row: RecoveryCodesRow = sqlx::query_as(
            "DELETE FROM user_credential_recovery_codes WHERE user_id = ?1 RETURNING *",
        )
        .bind(user_id)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::domain::user::User;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn codes_of(user: &User, mac_bytes: &[u8]) -> RecoveryCodeCredential {
        RecoveryCodeCredential {
            user_id: user.id,
            code_macs: mac_bytes
                .iter()
                .map(|byte| TokenMac::from(vec![*byte; 32]))
                .collect(),
            generated_at: timestamp(1),
        }
    }

    #[tokio::test]
    async fn upsert_for_a_user_with_codes_replaces_the_old_codes() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .recovery_codes
            .upsert(&mut TxContext::none(), &codes_of(&user, &[1, 2]))
            .await
            .unwrap();

        let stored = registry
            .recovery_codes
            .upsert(&mut TxContext::none(), &codes_of(&user, &[3]))
            .await
            .unwrap();

        let got = registry
            .recovery_codes
            .get(&mut TxContext::none(), user.id)
            .await
            .unwrap();
        assert_eq!(stored, codes_of(&user, &[3]));
        assert_eq!(got, codes_of(&user, &[3]));
    }

    #[tokio::test]
    async fn delete_twice_returns_the_codes_then_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .recovery_codes
            .upsert(&mut TxContext::none(), &codes_of(&user, &[1, 2]))
            .await
            .unwrap();

        let first_delete = registry
            .recovery_codes
            .delete(&mut TxContext::none(), user.id)
            .await;
        let second_delete = registry
            .recovery_codes
            .delete(&mut TxContext::none(), user.id)
            .await;

        assert_eq!(first_delete.unwrap(), codes_of(&user, &[1, 2]));
        assert_matches!(second_delete, Err(RepositoryError::NotFound));
    }
}
