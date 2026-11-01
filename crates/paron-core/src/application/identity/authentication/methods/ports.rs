use async_trait::async_trait;

use super::passkey::{CredentialId, PasskeyCredential, PasskeyRegistration};
use super::password::{Password, PasswordCredential, PasswordPhc};
use super::recovery_code::RecoveryCodeCredential;
use super::totp::{TotpCredential, TotpRegistration};
use crate::application::repository::{RepositoryError, TxContext};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PasswordCredentialRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<PasswordCredential, RepositoryError>;
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        credential: &PasswordCredential,
    ) -> Result<PasswordCredential, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<PasswordCredential, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash(&self, password: &Password) -> PasswordPhc;
    async fn verify(&self, password: &Password, hash: &PasswordPhc) -> bool;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TotpCredentialRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpCredential, RepositoryError>;
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        credential: &TotpCredential,
    ) -> Result<TotpCredential, RepositoryError>;
    async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        credential: &TotpCredential,
    ) -> Result<TotpCredential, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpCredential, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TotpRegistrationRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        registration: &TotpRegistration,
    ) -> Result<TotpRegistration, RepositoryError>;
    async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpRegistration, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpRegistration, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PasskeyCredentialRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn list_for_user(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Vec<PasskeyCredential>, RepositoryError>;
    async fn get_by_credential_id(
        &self,
        ctx: &mut TxContext<Tx>,
        credential_id: &CredentialId,
    ) -> Result<PasskeyCredential, RepositoryError>;
    async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        passkey: &PasskeyCredential,
    ) -> Result<PasskeyCredential, RepositoryError>;
    async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        passkey: &PasskeyCredential,
    ) -> Result<PasskeyCredential, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<PasskeyCredential, RepositoryError>;
    async fn delete_for_user(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<Vec<PasskeyCredential>, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PasskeyRegistrationRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        registration: &PasskeyRegistration,
    ) -> Result<PasskeyRegistration, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<PasskeyRegistration, RepositoryError>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RecoveryCodeRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<RecoveryCodeCredential, RepositoryError>;
    async fn upsert(
        &self,
        ctx: &mut TxContext<Tx>,
        credential: &RecoveryCodeCredential,
    ) -> Result<RecoveryCodeCredential, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<RecoveryCodeCredential, RepositoryError>;
}
