use async_trait::async_trait;
use indoc::indoc;

use super::Error;
use crate::application::identity::authentication::methods::passkey::{
    CredentialId, PasskeyBackup, PasskeyCredential, PasskeyName,
};
use crate::application::identity::authentication::methods::ports::PasskeyCredentialRepository;
use crate::application::repository::{RepositoryError, TxContext};
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqlitePasskeyRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqlitePasskeyRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqlitePasskeyRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(row: PasskeyRow) -> Result<PasskeyCredential, RepositoryError> {
        let id = row.id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context(format!("error reading passkey {id}")),
            )
        })
    }
}

#[derive(sqlx::FromRow)]
struct PasskeyRow {
    id: uuid::Uuid,
    user_id: uuid::Uuid,
    credential_id: Vec<u8>,
    public_key: Vec<u8>,
    name: String,
    sign_count: i64,
    backup_eligible: bool,
    backup_state: bool,
    created_at_unix_ms: i64,
    last_used_at_unix_ms: Option<i64>,
}

impl PasskeyRow {
    fn from_entity(passkey: &PasskeyCredential) -> Self {
        Self {
            id: passkey.id,
            user_id: passkey.user_id,
            credential_id: passkey.credential_id.0.clone(),
            public_key: passkey.public_key.clone(),
            name: passkey.name.to_string(),
            sign_count: i64::from(passkey.sign_count),
            backup_eligible: passkey.backup.eligible,
            backup_state: passkey.backup.state,
            created_at_unix_ms: passkey.created_at.as_millisecond(),
            last_used_at_unix_ms: passkey.last_used_at.map(jiff::Timestamp::as_millisecond),
        }
    }

    fn try_into_entity(self) -> Result<PasskeyCredential, Error> {
        Ok(PasskeyCredential {
            id: self.id,
            user_id: self.user_id,
            credential_id: CredentialId(self.credential_id),
            public_key: self.public_key,
            name: PasskeyName::parse(self.name)?,
            sign_count: u32::try_from(self.sign_count)?,
            backup: PasskeyBackup {
                eligible: self.backup_eligible,
                state: self.backup_state,
            },
            created_at: jiff::Timestamp::from_millisecond(self.created_at_unix_ms)?,
            last_used_at: self
                .last_used_at_unix_ms
                .map(jiff::Timestamp::from_millisecond)
                .transpose()?,
        })
    }
}

#[async_trait]
impl PasskeyCredentialRepository<SqliteTx> for SqlitePasskeyRepository {
    async fn list_for_user(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<Vec<PasskeyCredential>, RepositoryError> {
        let rows: Vec<PasskeyRow> = sqlx::query_as(
            "SELECT * FROM user_credential_passkey WHERE user_id = ?1 ORDER BY created_at_unix_ms",
        )
        .bind(user_id)
        .fetch_all(self.executing(ctx))
        .await?;

        rows.into_iter().map(Self::to_entity).collect()
    }

    async fn get_by_credential_id(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        credential_id: &CredentialId,
    ) -> Result<PasskeyCredential, RepositoryError> {
        let row: PasskeyRow =
            sqlx::query_as("SELECT * FROM user_credential_passkey WHERE credential_id = ?1")
                .bind(&credential_id.0)
                .fetch_one(self.executing(ctx))
                .await?;

        Self::to_entity(row)
    }

    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        passkey: &PasskeyCredential,
    ) -> Result<PasskeyCredential, RepositoryError> {
        let row = PasskeyRow::from_entity(passkey);

        let row: PasskeyRow = sqlx::query_as(indoc! {"
            INSERT INTO user_credential_passkey
                (id, user_id, credential_id, public_key, name, sign_count, backup_eligible,
                 backup_state, created_at_unix_ms, last_used_at_unix_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            RETURNING *
        "})
        .bind(row.id)
        .bind(row.user_id)
        .bind(&row.credential_id)
        .bind(&row.public_key)
        .bind(&row.name)
        .bind(row.sign_count)
        .bind(row.backup_eligible)
        .bind(row.backup_state)
        .bind(row.created_at_unix_ms)
        .bind(row.last_used_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn update(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        passkey: &PasskeyCredential,
    ) -> Result<PasskeyCredential, RepositoryError> {
        let row = PasskeyRow::from_entity(passkey);

        let row: PasskeyRow = sqlx::query_as(indoc! {"
            UPDATE user_credential_passkey
            SET name = ?2, sign_count = ?3, backup_state = ?4, last_used_at_unix_ms = ?5
            WHERE id = ?1
            RETURNING *
        "})
        .bind(row.id)
        .bind(&row.name)
        .bind(row.sign_count)
        .bind(row.backup_state)
        .bind(row.last_used_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<PasskeyCredential, RepositoryError> {
        let row: PasskeyRow = sqlx::query_as(
            "DELETE FROM user_credential_passkey WHERE user_id = ?1 AND id = ?2 RETURNING *",
        )
        .bind(user_id)
        .bind(id)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn delete_for_user(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user_id: uuid::Uuid,
    ) -> Result<Vec<PasskeyCredential>, RepositoryError> {
        let rows: Vec<PasskeyRow> =
            sqlx::query_as("DELETE FROM user_credential_passkey WHERE user_id = ?1 RETURNING *")
                .bind(user_id)
                .fetch_all(self.executing(ctx))
                .await?;

        rows.into_iter().map(Self::to_entity).collect()
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

    fn passkey_of(user: &User, credential_byte: u8) -> PasskeyCredential {
        PasskeyCredential {
            id: uuid::Uuid::now_v7(),
            user_id: user.id,
            credential_id: CredentialId(vec![credential_byte; 16]),
            public_key: vec![2; 33],
            name: PasskeyName::parse("Work laptop").unwrap(),
            sign_count: 0,
            backup: PasskeyBackup::default(),
            created_at: timestamp(i64::from(credential_byte)),
            last_used_at: None,
        }
    }

    #[tokio::test]
    async fn list_for_user_returns_only_the_passkeys_of_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let other_user = registry
            .create_user(&identity::user_builder().username("other").build())
            .await;
        let first = passkey_of(&user, 1);
        let second = passkey_of(&user, 2);
        for passkey in [&first, &second, &passkey_of(&other_user, 3)] {
            registry
                .passkeys
                .create(&mut TxContext::none(), passkey)
                .await
                .unwrap();
        }

        let listed = registry
            .passkeys
            .list_for_user(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        assert_eq!(listed, [first, second]);
    }

    #[tokio::test]
    async fn create_with_a_registered_credential_id_returns_already_exists() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .passkeys
            .create(&mut TxContext::none(), &passkey_of(&user, 1))
            .await
            .unwrap();

        let created = registry
            .passkeys
            .create(&mut TxContext::none(), &passkey_of(&user, 1))
            .await;

        assert_matches!(created, Err(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn update_stores_the_last_use() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let passkey = passkey_of(&user, 1);
        registry
            .passkeys
            .create(&mut TxContext::none(), &passkey)
            .await
            .unwrap();

        registry
            .passkeys
            .update(
                &mut TxContext::none(),
                &PasskeyCredential {
                    last_used_at: Some(timestamp(50)),
                    ..passkey.clone()
                },
            )
            .await
            .unwrap();

        let stored = registry
            .passkeys
            .get_by_credential_id(&mut TxContext::none(), &passkey.credential_id)
            .await
            .unwrap();
        assert_eq!(
            stored,
            PasskeyCredential {
                last_used_at: Some(timestamp(50)),
                ..passkey
            }
        );
    }

    #[tokio::test]
    async fn delete_of_a_passkey_of_another_user_returns_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let other_user = registry
            .create_user(&identity::user_builder().username("other").build())
            .await;
        let passkey = passkey_of(&user, 1);
        registry
            .passkeys
            .create(&mut TxContext::none(), &passkey)
            .await
            .unwrap();

        let deleted = registry
            .passkeys
            .delete(&mut TxContext::none(), other_user.id, passkey.id)
            .await;

        assert_matches!(deleted, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_for_user_returns_the_deleted_passkeys() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let passkey = passkey_of(&user, 1);
        registry
            .passkeys
            .create(&mut TxContext::none(), &passkey)
            .await
            .unwrap();

        let deleted = registry
            .passkeys
            .delete_for_user(&mut TxContext::none(), user.id)
            .await;

        let listed = registry
            .passkeys
            .list_for_user(&mut TxContext::none(), user.id)
            .await
            .unwrap();
        assert_eq!(deleted.unwrap(), [passkey]);
        assert_eq!(listed, []);
    }
}
