use crate::application::identity::ports::MockUserRepository;
use crate::application::machine::MockRepository;
use crate::application::repository::{MockUnitOfWork, RepositoryError, TxContext};
use crate::domain::machine::Machine;
use crate::domain::user::User;

/// The mock calls `machine` for each lookup, because [`Machine`] is not `Clone`.
pub fn machine_repository_returning(machine: fn() -> Machine) -> MockRepository<()> {
    let mut repository = MockRepository::new();
    repository
        .expect_get_by_id()
        .returning(move |_, _| Ok(machine()));

    repository
}

pub fn machine_repository_without_machines() -> MockRepository<()> {
    let mut repository = MockRepository::new();
    repository
        .expect_get_by_id()
        .returning(|_, _| Err(RepositoryError::NotFound));

    repository
}

pub fn transactions_that_commit() -> MockUnitOfWork {
    let mut transactions = MockUnitOfWork::new();
    transactions
        .expect_begin()
        .returning(|guarantee| Ok(TxContext::new((), guarantee)));
    transactions.expect_commit().returning(|_| Ok(()));
    transactions.expect_savepoint().returning(|_| Ok(()));
    transactions.expect_release().returning(|_| Ok(()));
    transactions.expect_rollback_to().returning(|_| Ok(()));

    transactions
}

pub fn user_repository_returning<F>(user: F) -> MockUserRepository<()>
where
    F: Fn() -> User + Clone + Send + Sync + 'static,
{
    let mut users = MockUserRepository::new();
    let user_by_username = user.clone();
    users
        .expect_get_by_username()
        .returning(move |_, _| Ok(user_by_username()));
    users.expect_get_by_id().returning(move |_, _| Ok(user()));

    users
}
