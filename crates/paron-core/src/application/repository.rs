use anyhow::Context as _;
use async_trait::async_trait;

use crate::domain::text::LimitedText;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("no such entity")]
    NotFound,

    #[error("entity already exists")]
    AlreadyExists,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: u32,
    pub offset: u32,
}

pub type SearchText = LimitedText<0, 512>;

/// What a list asks for: a text that each item must contain, if any, and the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListQuery {
    pub search: Option<SearchText>,
    pub page: Page,
}

pub struct Paginated<T> {
    pub items: Vec<T>,
    pub total: u64,
}

/// Represent a possible ongoing transaction
pub struct TxContext<Tx>(Option<Ongoing<Tx>>);

struct Ongoing<Tx> {
    transaction: Tx,
    guarantee: TxGuarantee,
}

impl<Tx> TxContext<Tx> {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn new(transaction: Tx, guarantee: TxGuarantee) -> Self {
        Self(Some(Ongoing {
            transaction,
            guarantee,
        }))
    }

    pub fn transaction(&mut self) -> Option<&mut Tx> {
        self.0.as_mut().map(|ongoing| &mut ongoing.transaction)
    }

    pub fn into_transaction(self) -> Option<Tx> {
        self.0.map(|ongoing| ongoing.transaction)
    }

    fn requires(&self, guarantee: TxGuarantee) -> Result<(), anyhow::Error> {
        match &self.0 {
            Some(ongoing) if ongoing.guarantee < guarantee => Err(anyhow::anyhow!(
                "this operation needs {guarantee:?}, and the transaction it was given is {:?}",
                ongoing.guarantee
            )),
            _ => Ok(()),
        }
    }
}

/// Ordered weakest first: a transaction giving the stronger one also gives the weaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TxGuarantee {
    ConsistentReads,
    ReadModifyWrite,
}

#[cfg_attr(test, mockall::automock(type Tx = ();))]
#[async_trait]
pub trait UnitOfWork: Send + Sync {
    type Tx: Send;

    async fn begin(&self, guarantee: TxGuarantee) -> Result<TxContext<Self::Tx>, RepositoryError>;
    async fn commit(&self, ctx: TxContext<Self::Tx>) -> Result<(), RepositoryError>;

    async fn savepoint(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError>;
    async fn release(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError>;
    async fn rollback_to(&self, ctx: &mut TxContext<Self::Tx>) -> Result<(), RepositoryError>;
}

#[allow(async_fn_in_trait)]
pub trait UnitOfWorkExt: UnitOfWork {
    async fn transaction<T, E>(
        &self,
        ctx: &mut TxContext<Self::Tx>,
        guarantee: TxGuarantee,
        work: impl AsyncFnOnce(&mut TxContext<Self::Tx>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<anyhow::Error>,
    {
        if ctx.transaction().is_some() {
            ctx.requires(guarantee)?;

            self.savepoint(ctx)
                .await
                .context("could not create tx savepoint")?;

            return match work(ctx).await {
                Ok(outcome) => {
                    self.release(ctx)
                        .await
                        .context("could not release tx savepoint")?;

                    Ok(outcome)
                }
                Err(error) => {
                    self.rollback_to(ctx)
                        .await
                        .context("could not rollback to tx savepoint")?;
                    self.release(ctx)
                        .await
                        .context("could not release tx savepoint")?;

                    Err(error)
                }
            };
        }

        let mut own = self.begin(guarantee).await.context("could not start tx")?;
        let outcome = work(&mut own).await?;
        self.commit(own).await.context("could not commit tx")?;

        Ok(outcome)
    }
}

impl<U: UnitOfWork + ?Sized> UnitOfWorkExt for U {}
