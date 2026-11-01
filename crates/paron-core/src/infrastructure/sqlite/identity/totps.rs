use async_trait::async_trait;
use indoc::indoc;
use secrecy::{ExposeSecret as _, SecretSlice};

use super::Error;
use crate::application::identity::authentication::methods::ports::TotpCredentialRepository;
use crate::application::identity::authentication::methods::totp::{
    TotpCredential, TotpSecret, TotpStep,
};
use crate::application::repository::{RepositoryError, TxContext};
use crate::infrastructure::crypto::cipher::AeadCipher;
use crate::infrastructure::sqlite::{EncryptedColumn, SqliteRepository, SqliteTx};

pub struct EncryptedSqliteTotpRepository {
    pool: sqlx::SqlitePool,
    cipher: std::sync::Arc<dyn AeadCipher>,
}

impl SqliteRepository for EncryptedSqliteTotpRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl EncryptedSqliteTotpRepository {
    pub fn new(pool: sqlx::SqlitePool, cipher: std::sync::Arc<dyn AeadCipher>) -> Self {
        Self { pool, cipher }
    }

    fn to_entity(&self, row: TotpRow) -> Result<TotpCredential, RepositoryError> {
        let user_id = row.user_id;

        row.try_into_entity(self.cipher.as_ref()).map_err(|error| {
            RepositoryError::Unexpected(anyhow::Error::from_boxed(error).context(format!(
                "error reading the TOTP credential of user {user_id}"
            )))
        })
    }
}

#[derive(sqlx::FromRow)]
struct TotpRow {
    user_id: uuid::Uuid,
    secret_encrypted: Vec<u8>,
    last_used_step: i64,
    created_at_unix_ms: i64,
}

impl TotpRow {
    fn from_entity(credential: &TotpCredential, cipher: &dyn AeadCipher) -> Self {
        Self {
            user_id: credential.user_id,
            secret_encrypted: cipher.encrypt(
                &EncryptedColumn::TotpSecret.associated_data(credential.user_id),
                credential.secret.expose_secret(),
            ),
            last_used_step: i64::try_from(credential.last_used_step.0).unwrap_or(i64::MAX),
            created_at_unix_ms: credential.created_at.as_millisecond(),
        }
    }

    fn try_into_entity(self, cipher: &dyn AeadCipher) -> Result<TotpCredential, Error> {
        let secret = cipher.decrypt(
            &EncryptedColumn::TotpSecret.associated_data(self.user_id),
            &self.secret_encrypted,
        )?;

        Ok(TotpCredential {
            user_id: self.user_id,
            secret: TotpSecret::from(SecretSlice::from(secret.expose_secret().to_vec())),
            last_used_step: TotpStep(u64::try_from(self.last_used_step)?),
            created_at: jiff::Timestamp::from_millisecond(self.created_at_unix_ms)?,
        })
    }
}

#[async_trait]
impl TotpCredentialRepository<SqliteTx> for EncryptedSqliteTotpRepository {
    async fn get(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpCredential, RepositoryError> {
        let row: TotpRow = sqlx::query_as("SELECT * FROM user_credential_totp WHERE user_id = ?1")
            .bind(user_id)
            .fetch_one(self.executing(ctx))
            .await?;

        self.to_entity(row)
    }

    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        credential: &TotpCredential,
    ) -> Result<TotpCredential, RepositoryError> {
        let row = TotpRow::from_entity(credential, self.cipher.as_ref());

        let row: TotpRow = sqlx::query_as(indoc! {"
            INSERT INTO user_credential_totp
                (user_id, secret_encrypted, last_used_step, created_at_unix_ms)
            VALUES (?1, ?2, ?3, ?4)
            RETURNING *
        "})
        .bind(row.user_id)
        .bind(&row.secret_encrypted)
        .bind(row.last_used_step)
        .bind(row.created_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        self.to_entity(row)
    }

    async fn update(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        credential: &TotpCredential,
    ) -> Result<TotpCredential, RepositoryError> {
        let row = TotpRow::from_entity(credential, self.cipher.as_ref());

        let row: TotpRow = sqlx::query_as(
            "UPDATE user_credential_totp SET last_used_step = ?2 WHERE user_id = ?1 RETURNING *",
        )
        .bind(row.user_id)
        .bind(row.last_used_step)
        .fetch_one(self.executing(ctx))
        .await?;

        self.to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpCredential, RepositoryError> {
        let row: TotpRow =
            sqlx::query_as("DELETE FROM user_credential_totp WHERE user_id = ?1 RETURNING *")
                .bind(user_id)
                .fetch_one(self.executing(ctx))
                .await?;

        self.to_entity(row)
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

    fn credential_of(user: &User) -> TotpCredential {
        TotpCredential {
            user_id: user.id,
            secret: TotpSecret::from(SecretSlice::from(vec![9; 20])),
            last_used_step: TotpStep(10),
            created_at: timestamp(1),
        }
    }

    #[tokio::test]
    async fn get_after_create_returns_the_decrypted_secret() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .totps
            .create(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        let stored = registry
            .totps
            .get(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        assert_eq!(stored.secret.expose_secret(), &[9; 20]);
        assert_eq!(stored.last_used_step, TotpStep(10));
    }

    #[tokio::test]
    async fn create_for_a_user_with_one_returns_already_exists() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .totps
            .create(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        let created = registry
            .totps
            .create(&mut TxContext::none(), &credential_of(&user))
            .await;

        assert_matches!(created, Err(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn update_returns_the_credential_with_the_new_step() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let created = registry
            .totps
            .create(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        let updated = registry
            .totps
            .update(
                &mut TxContext::none(),
                &TotpCredential {
                    last_used_step: TotpStep(11),
                    ..created
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.last_used_step, TotpStep(11));
        assert_eq!(updated.secret.expose_secret(), &[9; 20]);
    }

    #[tokio::test]
    async fn get_after_delete_returns_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .totps
            .create(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        registry
            .totps
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let stored = registry.totps.get(&mut TxContext::none(), user.id).await;
        assert_matches!(stored, Err(RepositoryError::NotFound));
    }
}
