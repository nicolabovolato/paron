use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::application::repository::{ListQuery, Paginated, RepositoryError, TxContext};
use crate::domain::user::{Role, User, UserStatus, Username};

pub struct UserFilter {
    pub status: Option<UserStatus>,
    /// A user matches when the user has one or more of these roles.
    pub roles: Option<BTreeSet<Role>>,
    pub excluding: Option<uuid::Uuid>,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserRepository<Tx: Send + Sync + 'static>: Send + Sync {
    async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        query: &ListQuery,
    ) -> Result<Paginated<User>, RepositoryError>;
    async fn get_by_id(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<User, RepositoryError>;
    async fn get_by_username(
        &self,
        ctx: &mut TxContext<Tx>,
        username: &Username,
    ) -> Result<User, RepositoryError>;
    async fn create(&self, ctx: &mut TxContext<Tx>, user: &User) -> Result<User, RepositoryError>;
    async fn update(&self, ctx: &mut TxContext<Tx>, user: &User) -> Result<User, RepositoryError>;
    async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        id: uuid::Uuid,
    ) -> Result<User, RepositoryError>;
    async fn count(
        &self,
        ctx: &mut TxContext<Tx>,
        filter: &UserFilter,
    ) -> Result<u64, RepositoryError>;
}
