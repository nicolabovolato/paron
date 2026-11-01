use async_trait::async_trait;
use indoc::indoc;
use secrecy::ExposeSecret as _;

use super::Error;
use crate::application::identity::authentication::methods::password::{
    PasswordCredential, PasswordPhc,
};
use crate::application::identity::authentication::methods::ports::PasswordCredentialRepository;
use crate::application::repository::{RepositoryError, TxContext};
use crate::infrastructure::crypto::cipher::AeadCipher;
use crate::infrastructure::sqlite::{EncryptedColumn, SqliteRepository, SqliteTx};

pub struct EncryptedSqlitePasswordRepository {
    pool: sqlx::SqlitePool,
    cipher: std::sync::Arc<dyn AeadCipher>,
}

impl SqliteRepository for EncryptedSqlitePasswordRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl EncryptedSqlitePasswordRepository {
    pub fn new(pool: sqlx::SqlitePool, cipher: std::sync::Arc<dyn AeadCipher>) -> Self {
        Self { pool, cipher }
    }

    fn to_entity(&self, row: PasswordRow) -> Result<PasswordCredential, RepositoryError> {
        let user_id = row.user_id;

        row.try_into_entity(self.cipher.as_ref()).map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error)
                    .context(format!("error reading the password of user {user_id}")),
            )
        })
    }
}

#[derive(sqlx::FromRow)]
struct PasswordRow {
    user_id: uuid::Uuid,
    phc_encrypted: Vec<u8>,
    updated_at_unix_ms: i64,
}

impl PasswordRow {
    fn from_entity(credential: &PasswordCredential, cipher: &dyn AeadCipher) -> Self {
        Self {
            user_id: credential.user_id,
            phc_encrypted: cipher.encrypt(
                &EncryptedColumn::PasswordPhc.associated_data(credential.user_id),
                credential.phc.as_bytes(),
            ),
            updated_at_unix_ms: credential.updated_at.as_millisecond(),
        }
    }

    fn try_into_entity(self, cipher: &dyn AeadCipher) -> Result<PasswordCredential, Error> {
        let phc = cipher.decrypt(
            &EncryptedColumn::PasswordPhc.associated_data(self.user_id),
            &self.phc_encrypted,
        )?;

        Ok(PasswordCredential {
            user_id: self.user_id,
            phc: PasswordPhc::parse(str::from_utf8(phc.expose_secret())?.to_owned())?,
            updated_at: jiff::Timestamp::from_millisecond(self.updated_at_unix_ms)?,
        })
    }
}

#[async_trait]
impl PasswordCredentialRepository<SqliteTx> for EncryptedSqlitePasswordRepository {
    async fn get(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<PasswordCredential, RepositoryError> {
        let row: PasswordRow =
            sqlx::query_as("SELECT * FROM user_credential_password WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(self.executing(ctx))
                .await?;

        self.to_entity(row)
    }

    async fn upsert(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        credential: &PasswordCredential,
    ) -> Result<PasswordCredential, RepositoryError> {
        let row = PasswordRow::from_entity(credential, self.cipher.as_ref());

        let row: PasswordRow = sqlx::query_as(indoc! {"
            INSERT INTO user_credential_password (user_id, phc_encrypted, updated_at_unix_ms)
            VALUES (?1, ?2, ?3)
            ON CONFLICT (user_id) DO UPDATE SET
                phc_encrypted = excluded.phc_encrypted,
                updated_at_unix_ms = excluded.updated_at_unix_ms
            RETURNING *
        "})
        .bind(row.user_id)
        .bind(&row.phc_encrypted)
        .bind(row.updated_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        self.to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<PasswordCredential, RepositoryError> {
        let row: PasswordRow =
            sqlx::query_as("DELETE FROM user_credential_password WHERE user_id = ?1 RETURNING *")
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
    use crate::application::identity::ports::UserRepository as _;
    use crate::domain::user::User;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn credential_of(user: &User) -> PasswordCredential {
        PasswordCredential {
            user_id: user.id,
            phc: identity::password_phc(),
            updated_at: timestamp(1),
        }
    }

    #[tokio::test]
    async fn upsert_then_get_returns_the_credential() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;

        registry
            .passwords
            .upsert(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        let stored = registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_eq!(stored.unwrap(), credential_of(&user));
    }

    #[tokio::test]
    async fn upsert_twice_keeps_the_second_credential() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let replacement = PasswordCredential {
            phc: PasswordPhc::parse(
                "$scrypt$ln=17,r=8,p=1$c29tZXNhbHR2YWx1ZQ$3Vh0YkFDR0hJSktMTU5PUFFSU1RVVldY"
                    .to_owned(),
            )
            .unwrap(),
            updated_at: timestamp(2),
            ..credential_of(&user)
        };
        registry
            .passwords
            .upsert(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        registry
            .passwords
            .upsert(&mut TxContext::none(), &replacement)
            .await
            .unwrap();

        let stored = registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_eq!(stored.unwrap(), replacement);
    }

    #[tokio::test]
    async fn upsert_stores_the_hash_as_ciphertext() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .passwords
            .upsert(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        let stored_bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT phc_encrypted FROM user_credential_password WHERE user_id = ?",
        )
        .bind(user.id)
        .fetch_one(&registry.pool)
        .await
        .unwrap();

        let phc = identity::PASSWORD_PHC.as_bytes();
        assert!(!stored_bytes.windows(phc.len()).any(|window| window == phc));
    }

    #[tokio::test]
    async fn delete_removes_the_credential() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .passwords
            .upsert(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        registry
            .passwords
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let stored = registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(stored, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_of_user_without_password_returns_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;

        let deleted = registry
            .passwords
            .delete(&mut TxContext::none(), user.id)
            .await;

        assert_matches!(deleted, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_of_the_user_removes_the_credential() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .passwords
            .upsert(&mut TxContext::none(), &credential_of(&user))
            .await
            .unwrap();

        registry
            .users
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let stored = registry
            .passwords
            .get(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(stored, Err(RepositoryError::NotFound));
    }
}
