use async_trait::async_trait;

use super::account_recovery::PendingAccountRecovery;
use super::failed_sign_in_attempts::{AttemptKey, FailedSignInAttempts};
use super::sessions::Session;
use super::sign_in::PendingSignIn;
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::token_mac::TokenMac;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SessionRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get_by_id(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<Session, RepositoryError>;
    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<Tx>,
        mac: &TokenMac,
    ) -> Result<Session, RepositoryError>;
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        session: &Session,
    ) -> Result<Session, RepositoryError>;
    async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        session: &Session,
    ) -> Result<Session, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<Session, RepositoryError>;
    async fn delete_for_user(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        keep_newest: usize,
        except: Option<uuid::Uuid>,
    ) -> Result<Vec<Session>, RepositoryError>;
    async fn delete_expired(
        &self,
        ctx: &mut TxContext<Tx>,
        now: jiff::Timestamp,
        unused_since: jiff::Timestamp,
        limit: usize,
    ) -> Result<Vec<Session>, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PendingSignInRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        pending_sign_in: &PendingSignIn,
    ) -> Result<PendingSignIn, RepositoryError>;
    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<Tx>,
        mac: &TokenMac,
    ) -> Result<PendingSignIn, RepositoryError>;
    async fn delete_by_mac(
        &self,
        ctx: &mut TxContext<Tx>,
        mac: &TokenMac,
    ) -> Result<PendingSignIn, RepositoryError>;
    async fn delete_expired(
        &self,
        ctx: &mut TxContext<Tx>,
        now: jiff::Timestamp,
    ) -> Result<u64, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PendingAccountRecoveryRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get_by_mac(
        &self,
        ctx: &mut TxContext<Tx>,
        mac: &TokenMac,
    ) -> Result<PendingAccountRecovery, RepositoryError>;
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        recovery: &PendingAccountRecovery,
    ) -> Result<PendingAccountRecovery, RepositoryError>;
    async fn delete_by_mac(
        &self,
        ctx: &mut TxContext<Tx>,
        mac: &TokenMac,
    ) -> Result<PendingAccountRecovery, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FailedSignInAttemptRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        key: &AttemptKey,
    ) -> Result<FailedSignInAttempts, RepositoryError>;
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        attempts: &FailedSignInAttempts,
    ) -> Result<FailedSignInAttempts, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        key: &AttemptKey,
    ) -> Result<FailedSignInAttempts, RepositoryError>;
    async fn delete_expired(
        &self,
        ctx: &mut TxContext<Tx>,
        now: jiff::Timestamp,
    ) -> Result<u64, RepositoryError>;
}
