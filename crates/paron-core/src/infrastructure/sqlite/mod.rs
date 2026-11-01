/// Declares the type discriminator from an enum and validates against the type system to keep them in sync.
///
/// ```text
/// enum Example {
///     One(String),
///     Two { value: usize },
/// }
///
/// extract_discriminant! {
///     #[derive(sqlx::Type, Clone, Copy)]
///     #[sqlx(rename_all = "snake_case")]
///     enum ExampleKind from Example { One, Two }
/// }
/// ```
macro_rules! extract_discriminant {
    (
        $(#[$meta:meta])*
        enum $dest:ident from $source:path { $($(#[$variant_meta:meta])* $variant:ident),+ $(,)? }
    ) => {
        $(#[$meta])*
        enum $dest {
            $($(#[$variant_meta])* $variant),+
        }

        impl From<&$source> for $dest {
            fn from(value: &$source) -> Self {
                type Source = $source;
                match value {
                    $(Source::$variant { .. } => Self::$variant),+
                }
            }
        }
    };
}

pub mod audit;
pub mod identity;
pub mod machine;
pub mod transaction;

pub use transaction::SqliteTx;

use futures_core::future::BoxFuture;
use futures_core::stream::BoxStream;
use sqlx::{Database, Either, Execute, SqlStr, Sqlite, SqliteConnection, SqlitePool};

use crate::application::repository::{RepositoryError, TxContext};

// a number must not change once a value is stored with it
#[derive(Clone, Copy)]
#[repr(u8)]
enum EncryptedColumn {
    PasswordPhc = 1,
    SshPassword = 2,
    SshPrivateKey = 3,
    TotpSecret = 4,
}

impl EncryptedColumn {
    fn associated_data(self, row_id: uuid::Uuid) -> Vec<u8> {
        [&[self as u8], row_id.as_bytes().as_slice()].concat()
    }
}

pub trait SqliteRepository {
    fn pool(&self) -> &SqlitePool;

    /// Where the next statement runs: the transaction the caller is in, or the pool.
    fn executing<'a>(&'a self, ctx: &'a mut TxContext<SqliteTx>) -> Executor<'a> {
        match ctx.transaction() {
            Some(transaction) => Executor::Transaction(transaction),
            None => Executor::Pool(self.pool()),
        }
    }
}

pub enum Executor<'a> {
    Pool(&'a SqlitePool),
    Transaction(&'a mut SqliteConnection),
}

impl std::fmt::Debug for Executor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pool(_) => f.write_str("Executor::Pool"),
            Self::Transaction(_) => f.write_str("Executor::Transaction"),
        }
    }
}

impl<'c> sqlx::Executor<'c> for Executor<'c> {
    type Database = Sqlite;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<
        'e,
        Result<Either<<Sqlite as Database>::QueryResult, <Sqlite as Database>::Row>, sqlx::Error>,
    >
    where
        'c: 'e,
        E: 'q + Execute<'q, Sqlite>,
    {
        match self {
            Self::Pool(pool) => pool.fetch_many(query),
            Self::Transaction(transaction) => transaction.fetch_many(query),
        }
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<<Sqlite as Database>::Row>, sqlx::Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, Sqlite>,
    {
        match self {
            Self::Pool(pool) => pool.fetch_optional(query),
            Self::Transaction(transaction) => transaction.fetch_optional(query),
        }
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [<Sqlite as Database>::TypeInfo],
    ) -> BoxFuture<'e, Result<<Sqlite as Database>::Statement, sqlx::Error>>
    where
        'c: 'e,
    {
        match self {
            Self::Pool(pool) => pool.prepare_with(sql, parameters),
            Self::Transaction(transaction) => transaction.prepare_with(sql, parameters),
        }
    }

    fn describe<'e>(self, sql: SqlStr) -> BoxFuture<'e, Result<sqlx::Describe<Sqlite>, sqlx::Error>>
    where
        'c: 'e,
    {
        match self {
            Self::Pool(pool) => pool.describe(sql),
            Self::Transaction(transaction) => transaction.describe(sql),
        }
    }
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("src/infrastructure/sqlite/migrations");

pub fn requirements() -> (
    sqlx::sqlite::SqliteConnectOptions,
    sqlx::sqlite::SqlitePoolOptions,
) {
    (
        sqlx::sqlite::SqliteConnectOptions::new().foreign_keys(true),
        sqlx::sqlite::SqlitePoolOptions::new(),
    )
}

/// For a pool somebody else built, which is what the tests do to contend on one deliberately
pub async fn migrate(pool: &sqlx::SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

impl From<sqlx::Error> for RepositoryError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::RowNotFound => Self::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::AlreadyExists,
            _ => Self::Unexpected(anyhow::Error::from(error)),
        }
    }
}
