use async_trait::async_trait;
use indoc::indoc;

use super::Error;
use crate::application::identity::authentication::methods::AuthenticationMethod;
use crate::application::identity::authentication::methods::passkey::Challenge;
use crate::application::identity::authentication::ports::PendingSignInRepository;
use crate::application::identity::authentication::sign_in::{
    PendingSignIn, SignInPurpose, SignInUser,
};
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::token_mac::TokenMac;
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqlitePendingSignInRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteRepository for SqlitePendingSignInRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqlitePendingSignInRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    fn to_entity(row: PendingSignInRow) -> Result<PendingSignIn, RepositoryError> {
        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context("error reading a pending sign-in"),
            )
        })
    }
}

#[derive(sqlx::FromRow)]
struct PendingSignInRow {
    token_mac: Vec<u8>,
    origin: String,
    renewed_session_id: Option<uuid::Uuid>,
    user_id: Option<uuid::Uuid>,
    verified_methods: sqlx::types::Json<Vec<String>>,
    accepted_methods: sqlx::types::Json<Vec<String>>,
    challenge: Option<Vec<u8>>,
    expires_at_unix_ms: i64,
}

impl PendingSignInRow {
    fn from_entity(pending_sign_in: &PendingSignIn) -> Self {
        let renewed_session_id = match pending_sign_in.purpose {
            SignInPurpose::NewSession(_) => None,
            SignInPurpose::Reauthentication { session_id, .. } => Some(session_id),
        };
        let user = pending_sign_in.purpose.user();

        Self {
            token_mac: pending_sign_in.mac.to_vec(),
            origin: pending_sign_in.origin.ascii_serialization(),
            renewed_session_id,
            user_id: user.map(|user| user.user_id),
            verified_methods: sqlx::types::Json(user.map_or_else(Vec::new, |user| {
                user.verified_methods
                    .iter()
                    .map(|method| method.as_str().to_owned())
                    .collect()
            })),
            accepted_methods: sqlx::types::Json(
                pending_sign_in
                    .accepted_methods
                    .iter()
                    .map(|method| method.as_str().to_owned())
                    .collect(),
            ),
            challenge: pending_sign_in
                .challenge
                .as_ref()
                .map(|challenge| challenge.0.to_vec()),
            expires_at_unix_ms: pending_sign_in.expires_at.as_millisecond(),
        }
    }

    fn try_into_entity(self) -> Result<PendingSignIn, Error> {
        let verified_methods = self
            .verified_methods
            .0
            .iter()
            .map(|method| method.parse())
            .collect::<Result<Vec<AuthenticationMethod>, _>>()?;
        let accepted_methods = self
            .accepted_methods
            .0
            .iter()
            .map(|method| method.parse())
            .collect::<Result<Vec<AuthenticationMethod>, _>>()?;
        let user = self.user_id.map(|user_id| SignInUser {
            user_id,
            verified_methods,
        });
        let purpose = match (self.renewed_session_id, user) {
            (None, user) => SignInPurpose::NewSession(user),
            (Some(session_id), Some(user)) => SignInPurpose::Reauthentication { session_id, user },
            (Some(_), None) => return Err("a reauthentication has no user".into()),
        };

        Ok(PendingSignIn {
            mac: TokenMac::from(self.token_mac),
            origin: url::Url::parse(&self.origin)?.origin(),
            purpose,
            accepted_methods,
            challenge: self
                .challenge
                .map(|challenge| <[u8; 32]>::try_from(challenge.as_slice()).map(Challenge))
                .transpose()?,
            expires_at: jiff::Timestamp::from_millisecond(self.expires_at_unix_ms)?,
        })
    }
}

#[async_trait]
impl PendingSignInRepository<SqliteTx> for SqlitePendingSignInRepository {
    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        pending_sign_in: &PendingSignIn,
    ) -> Result<PendingSignIn, RepositoryError> {
        let row = PendingSignInRow::from_entity(pending_sign_in);

        let row: PendingSignInRow = sqlx::query_as(indoc! {"
            INSERT INTO user_pending_sign_ins
                (token_mac, origin, renewed_session_id, user_id, verified_methods,
                 accepted_methods, challenge, expires_at_unix_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            RETURNING *
        "})
        .bind(&row.token_mac)
        .bind(&row.origin)
        .bind(row.renewed_session_id)
        .bind(row.user_id)
        .bind(&row.verified_methods)
        .bind(&row.accepted_methods)
        .bind(&row.challenge)
        .bind(row.expires_at_unix_ms)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        mac: &TokenMac,
    ) -> Result<PendingSignIn, RepositoryError> {
        let row: PendingSignInRow =
            sqlx::query_as("SELECT * FROM user_pending_sign_ins WHERE token_mac = ?1")
                .bind(&mac[..])
                .fetch_one(self.executing(ctx))
                .await?;

        Self::to_entity(row)
    }

    async fn delete_by_mac(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        mac: &TokenMac,
    ) -> Result<PendingSignIn, RepositoryError> {
        let row: PendingSignInRow =
            sqlx::query_as("DELETE FROM user_pending_sign_ins WHERE token_mac = ?1 RETURNING *")
                .bind(&mac[..])
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
            sqlx::query("DELETE FROM user_pending_sign_ins WHERE expires_at_unix_ms <= ?1")
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
    use crate::domain::user::User;
    use crate::testing::authentication::origin;
    use crate::testing::clock::timestamp;
    use crate::testing::identity;
    use crate::testing::registry::Registry;

    fn mac(byte: u8) -> TokenMac {
        TokenMac::from(vec![byte; 32])
    }

    fn pending_sign_in(user: &User, mac_byte: u8, expires_at: jiff::Timestamp) -> PendingSignIn {
        PendingSignIn {
            mac: mac(mac_byte),
            origin: origin(),
            purpose: SignInPurpose::NewSession(Some(SignInUser {
                user_id: user.id,
                verified_methods: vec![AuthenticationMethod::Password],
            })),
            accepted_methods: vec![AuthenticationMethod::Totp, AuthenticationMethod::Passkey],
            challenge: Some(Challenge([9; 32])),
            expires_at,
        }
    }

    #[tokio::test]
    async fn get_by_mac_after_create_returns_the_pending_sign_in() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .pending_sign_ins
            .create(
                &mut TxContext::none(),
                &pending_sign_in(&user, 1, timestamp(100)),
            )
            .await
            .unwrap();

        let pending = registry
            .pending_sign_ins
            .get_by_mac(&mut TxContext::none(), &mac(1))
            .await;

        assert_eq!(pending.unwrap(), pending_sign_in(&user, 1, timestamp(100)));
    }

    #[tokio::test]
    async fn delete_by_mac_twice_returns_the_pending_sign_in_then_not_found() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .pending_sign_ins
            .create(
                &mut TxContext::none(),
                &pending_sign_in(&user, 1, timestamp(100)),
            )
            .await
            .unwrap();

        let first_delete = registry
            .pending_sign_ins
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;
        let second_delete = registry
            .pending_sign_ins
            .delete_by_mac(&mut TxContext::none(), &mac(1))
            .await;

        assert_eq!(
            first_delete.unwrap(),
            pending_sign_in(&user, 1, timestamp(100))
        );
        assert_matches!(second_delete, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_expired_removes_only_the_expired_sign_ins() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        for (mac_byte, expires_at) in [(1, timestamp(100)), (2, timestamp(200))] {
            registry
                .pending_sign_ins
                .create(
                    &mut TxContext::none(),
                    &pending_sign_in(&user, mac_byte, expires_at),
                )
                .await
                .unwrap();
        }

        let deleted = registry
            .pending_sign_ins
            .delete_expired(&mut TxContext::none(), timestamp(150))
            .await;

        assert_eq!(deleted.unwrap(), 1);
        assert_matches!(
            registry
                .pending_sign_ins
                .get_by_mac(&mut TxContext::none(), &mac(1))
                .await,
            Err(RepositoryError::NotFound)
        );
        assert_matches!(
            registry
                .pending_sign_ins
                .get_by_mac(&mut TxContext::none(), &mac(2))
                .await,
            Ok(_)
        );
    }
}
