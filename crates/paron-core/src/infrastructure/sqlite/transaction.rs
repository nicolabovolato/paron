use async_trait::async_trait;
use sqlx::{Sqlite, SqlitePool};

use crate::application::repository::{RepositoryError, TxContext, TxGuarantee, UnitOfWork};

pub struct SqliteTx(sqlx::Transaction<'static, Sqlite>);

impl std::ops::Deref for SqliteTx {
    type Target = sqlx::Transaction<'static, Sqlite>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SqliteTx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct SqliteUnitOfWork {
    pool: SqlitePool,
}

impl SqliteUnitOfWork {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn execute(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        sql: &'static str,
    ) -> Result<(), RepositoryError> {
        let transaction = ctx
            .transaction()
            .ok_or_else(|| anyhow::anyhow!("{sql} needs an open transaction"))?;

        sqlx::raw_sql(sql).execute(&mut ***transaction).await?;

        Ok(())
    }
}

#[async_trait]
impl UnitOfWork for SqliteUnitOfWork {
    type Tx = SqliteTx;

    async fn begin(&self, guarantee: TxGuarantee) -> Result<TxContext<Self::Tx>, RepositoryError> {
        let sql = match guarantee {
            TxGuarantee::ConsistentReads => "BEGIN DEFERRED",
            TxGuarantee::ReadModifyWrite => "BEGIN IMMEDIATE",
        };

        let tx = self.pool.begin_with(sql).await?;

        Ok(TxContext::new(SqliteTx(tx), guarantee))
    }

    async fn commit(&self, ctx: TxContext<Self::Tx>) -> Result<(), RepositoryError> {
        match ctx.into_transaction() {
            Some(transaction) => Ok(transaction.0.commit().await?),
            None => Ok(()),
        }
    }

    async fn savepoint(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError> {
        self.execute(ctx, "SAVEPOINT paron_savepoint").await
    }

    async fn release(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError> {
        self.execute(ctx, "RELEASE paron_savepoint").await
    }

    async fn rollback_to(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError> {
        self.execute(ctx, "ROLLBACK TO paron_savepoint").await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::machine::Repository;
    use crate::application::repository::UnitOfWorkExt as _;
    use crate::domain::machine::{Machine, MachineName};
    use crate::infrastructure::sqlite::machine::repository::EncryptedSqliteMachineRepository;
    use crate::testing::machine;
    use crate::testing::registry::Registry;

    async fn rename_machine(
        machines: &EncryptedSqliteMachineRepository,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
        name: &str,
    ) -> Result<(), RepositoryError> {
        let machine = machines.get_by_id(ctx, id).await?;

        machines
            .update(
                ctx,
                &Machine {
                    name: MachineName::parse(name).unwrap(),
                    ..machine
                },
            )
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn commit_keeps_the_writes() {
        let registry = Registry::in_memory().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let mut ctx = registry
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await
            .unwrap();
        rename_machine(&registry.machines, &mut ctx, machine.id, "renamed")
            .await
            .unwrap();

        registry.transactions.commit(ctx).await.unwrap();

        assert_eq!(&*registry.get_machine(machine.id).await.name, "renamed");
    }

    #[tokio::test]
    async fn begin_without_commit_discards_the_writes() {
        let registry = Registry::in_memory().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let mut ctx = registry
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await
            .unwrap();
        rename_machine(&registry.machines, &mut ctx, machine.id, "renamed")
            .await
            .unwrap();

        drop(ctx);

        assert_eq!(registry.get_machine(machine.id).await.name, machine.name);
    }

    #[tokio::test]
    async fn savepoint_without_a_transaction_returns_error() {
        let registry = Registry::in_memory().await;

        let savepoint = registry
            .transactions
            .savepoint(&mut TxContext::none())
            .await;

        assert_matches!(savepoint, Err(_));
    }

    #[tokio::test]
    async fn release_without_a_transaction_returns_error() {
        let registry = Registry::in_memory().await;

        let released = registry.transactions.release(&mut TxContext::none()).await;

        assert_matches!(released, Err(_));
    }

    #[tokio::test]
    async fn rollback_to_without_a_transaction_returns_error() {
        let registry = Registry::in_memory().await;

        let rolled_back = registry
            .transactions
            .rollback_to(&mut TxContext::none())
            .await;

        assert_matches!(rolled_back, Err(_));
    }

    #[tokio::test]
    async fn rollback_to_a_savepoint_of_another_transaction_returns_error_and_keeps_its_writes() {
        let registry = Registry::on_disk().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let mut write_ctx = registry
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await
            .unwrap();
        registry
            .transactions
            .savepoint(&mut write_ctx)
            .await
            .unwrap();
        rename_machine(&registry.machines, &mut write_ctx, machine.id, "renamed")
            .await
            .unwrap();
        let mut read_ctx = registry
            .transactions
            .begin(TxGuarantee::ConsistentReads)
            .await
            .unwrap();

        let rolled_back = registry.transactions.rollback_to(&mut read_ctx).await;

        assert_matches!(rolled_back, Err(_));
        drop(read_ctx);
        registry.transactions.commit(write_ctx).await.unwrap();
        assert_eq!(&*registry.get_machine(machine.id).await.name, "renamed");
    }

    #[tokio::test]
    async fn transaction_inside_a_weaker_guarantee_returns_error_and_skips_the_work() {
        let registry = Registry::in_memory().await;
        let transactions = &registry.transactions;
        let mut work_ran = false;

        let outcome: Result<(), RepositoryError> = transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ConsistentReads,
                async |ctx| {
                    transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |_| {
                            work_ran = true;
                            Ok::<_, RepositoryError>(())
                        })
                        .await
                },
            )
            .await;

        assert_matches!(outcome, Err(RepositoryError::Unexpected(_)));
        assert!(!work_ran);
    }

    #[tokio::test]
    async fn transaction_inside_a_stronger_guarantee_succeeds() {
        let registry = Registry::in_memory().await;
        let transactions = &registry.transactions;

        let outcome: Result<(), RepositoryError> = transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ReadModifyWrite,
                async |ctx| {
                    transactions
                        .transaction(ctx, TxGuarantee::ConsistentReads, async |_| Ok(()))
                        .await
                },
            )
            .await;

        assert_matches!(outcome, Ok(()));
    }

    #[tokio::test]
    async fn transaction_inside_another_that_fails_keeps_the_writes_of_the_outer_one() {
        let registry = Registry::in_memory().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let (machines, transactions) = (&registry.machines, &registry.transactions);

        transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ReadModifyWrite,
                async |ctx| {
                    rename_machine(machines, ctx, machine.id, "kept").await?;

                    let inner_outcome: Result<(), RepositoryError> = transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                            rename_machine(machines, ctx, machine.id, "discarded").await?;

                            Err(RepositoryError::Unexpected(anyhow::anyhow!(
                                "the work failed"
                            )))
                        })
                        .await;
                    assert_matches!(inner_outcome, Err(_));

                    Ok::<_, RepositoryError>(())
                },
            )
            .await
            .unwrap();

        assert_eq!(&*registry.get_machine(machine.id).await.name, "kept");
    }

    // Two machines: with one machine, the second write can hide a rollback of the first that did
    // not occur.
    #[tokio::test]
    async fn transaction_inside_another_twice_rolls_back_only_the_one_that_fails() {
        let registry = Registry::in_memory().await;
        let rolled_back_machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let committed_machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let (machines, transactions) = (&registry.machines, &registry.transactions);

        transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ReadModifyWrite,
                async |ctx| {
                    let first_outcome: Result<(), RepositoryError> = transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                            rename_machine(machines, ctx, rolled_back_machine.id, "renamed")
                                .await?;

                            Err(RepositoryError::Unexpected(anyhow::anyhow!(
                                "the work failed"
                            )))
                        })
                        .await;
                    assert_matches!(first_outcome, Err(_));

                    transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                            rename_machine(machines, ctx, committed_machine.id, "renamed").await
                        })
                        .await
                },
            )
            .await
            .unwrap();

        assert_eq!(
            [
                &*registry.get_machine(rolled_back_machine.id).await.name,
                &*registry.get_machine(committed_machine.id).await.name,
            ],
            [&*rolled_back_machine.name, "renamed"]
        );
    }

    #[tokio::test]
    async fn transaction_inside_another_that_fails_rolls_back_every_level_below_it() {
        let registry = Registry::in_memory().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let (machines, transactions) = (&registry.machines, &registry.transactions);

        transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ReadModifyWrite,
                async |ctx| {
                    rename_machine(machines, ctx, machine.id, "outermost").await?;

                    let middle_outcome: Result<(), RepositoryError> = transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                            rename_machine(machines, ctx, machine.id, "middle").await?;

                            transactions
                                .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                                    rename_machine(machines, ctx, machine.id, "innermost").await
                                })
                                .await?;

                            Err(RepositoryError::Unexpected(anyhow::anyhow!(
                                "the work failed"
                            )))
                        })
                        .await;
                    assert_matches!(middle_outcome, Err(_));

                    Ok::<_, RepositoryError>(())
                },
            )
            .await
            .unwrap();

        assert_eq!(&*registry.get_machine(machine.id).await.name, "outermost");
    }

    #[tokio::test]
    async fn transaction_inside_another_commits_only_with_the_outer_one() {
        let registry = Registry::on_disk().await;
        let machine = registry
            .create_machine(&machine::unenrolled_machine())
            .await;
        let (machines, transactions) = (&registry.machines, &registry.transactions);

        let outcome: Result<(), RepositoryError> = transactions
            .transaction(
                &mut TxContext::none(),
                TxGuarantee::ReadModifyWrite,
                async |ctx| {
                    let inner_outcome = transactions
                        .transaction(ctx, TxGuarantee::ReadModifyWrite, async |ctx| {
                            rename_machine(machines, ctx, machine.id, "renamed").await
                        })
                        .await;
                    assert_matches!(inner_outcome, Ok(()));

                    Err(RepositoryError::Unexpected(anyhow::anyhow!(
                        "the work failed"
                    )))
                },
            )
            .await;

        assert_matches!(outcome, Err(_));
        assert_eq!(registry.get_machine(machine.id).await.name, machine.name);
    }

    #[tokio::test]
    async fn begin_read_modify_write_while_another_is_open_returns_busy() {
        let registry = Registry::on_disk().await;
        let without_wait = registry.without_busy_wait().await;
        let first = registry
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await
            .unwrap();

        let second = without_wait
            .transactions
            .begin(TxGuarantee::ReadModifyWrite)
            .await;

        let Err(RepositoryError::Unexpected(error)) = second else {
            panic!("the second transaction began while the first was open");
        };
        assert_matches!(
            error.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::Database(database)) if database.code().as_deref() == Some("5")
        );
        drop(first);
    }
}
