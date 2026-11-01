use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretBox;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::application::identity::authentication::ports::SessionRepository as _;
use crate::application::identity::authentication::sessions::Session;
use crate::application::identity::ports::UserRepository as _;
use crate::application::machine::Repository as _;
use crate::application::repository::TxContext;
use crate::domain::machine::Machine;
use crate::domain::user::User;
use crate::infrastructure::crypto::cipher::XChaCha20Poly1305Cipher;
use crate::infrastructure::sqlite::audit::repository::SqliteAuditRepository;
use crate::infrastructure::sqlite::identity::failed_sign_in_attempts::SqliteFailedSignInAttemptRepository;
use crate::infrastructure::sqlite::identity::passkeys::SqlitePasskeyRepository;
use crate::infrastructure::sqlite::identity::passwords::EncryptedSqlitePasswordRepository;
use crate::infrastructure::sqlite::identity::pending_account_recoveries::SqlitePendingAccountRecoveryRepository;
use crate::infrastructure::sqlite::identity::pending_sign_ins::SqlitePendingSignInRepository;
use crate::infrastructure::sqlite::identity::recovery_codes::SqliteRecoveryCodeRepository;
use crate::infrastructure::sqlite::identity::sessions::SqliteSessionRepository;
use crate::infrastructure::sqlite::identity::totps::EncryptedSqliteTotpRepository;
use crate::infrastructure::sqlite::identity::users::SqliteUserRepository;
use crate::infrastructure::sqlite::machine::repository::EncryptedSqliteMachineRepository;
use crate::infrastructure::sqlite::transaction::SqliteUnitOfWork;

const CIPHER_KEY: [u8; 32] = [7; 32];
const CONTENDING_CONNECTIONS: u32 = 4;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The repositories on one SQLite database, for the tests of the SQLite adapters.
pub struct Registry {
    pub pool: SqlitePool,
    pub machines: Arc<EncryptedSqliteMachineRepository>,
    pub users: Arc<SqliteUserRepository>,
    pub passwords: Arc<EncryptedSqlitePasswordRepository>,
    pub totps: Arc<EncryptedSqliteTotpRepository>,
    pub passkeys: Arc<SqlitePasskeyRepository>,
    pub recovery_codes: Arc<SqliteRecoveryCodeRepository>,
    pub pending_account_recoveries: Arc<SqlitePendingAccountRecoveryRepository>,
    pub sessions: Arc<SqliteSessionRepository>,
    pub pending_sign_ins: Arc<SqlitePendingSignInRepository>,
    pub failed_sign_in_attempts: Arc<SqliteFailedSignInAttemptRepository>,
    pub transactions: Arc<SqliteUnitOfWork>,
    pub audit: Arc<SqliteAuditRepository>,
}

impl Registry {
    /// One connection, so `sqlite::memory:` is one database and not one database for each
    /// connection of the pool.
    pub async fn in_memory() -> Self {
        Self::with_pool(
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        )
        .await
    }

    /// A database file in WAL mode, as in production, that several connections open at the same
    /// time, so two callers can contend.
    pub async fn on_disk() -> TemporaryRegistry {
        let directory = std::env::temp_dir().join(format!(
            "paron-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let pool = SqlitePoolOptions::new()
            .max_connections(CONTENDING_CONNECTIONS)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(directory.join("paron.db"))
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(BUSY_TIMEOUT),
            )
            .await
            .unwrap();

        TemporaryRegistry {
            registry: Self::with_pool(pool).await,
            directory,
        }
    }

    /// The same database, with repositories that return an error at once when another
    /// connection holds the lock.
    pub async fn without_busy_wait(&self) -> Self {
        Self::with_pool(
            SqlitePool::connect_with(
                self.pool
                    .connect_options()
                    .as_ref()
                    .clone()
                    .busy_timeout(Duration::ZERO),
            )
            .await
            .unwrap(),
        )
        .await
    }

    pub async fn create_machine(&self, machine: &Machine) -> Machine {
        self.machines
            .create(&mut TxContext::none(), machine)
            .await
            .unwrap()
    }

    pub async fn get_machine(&self, id: uuid::Uuid) -> Machine {
        self.machines
            .get_by_id(&mut TxContext::none(), id)
            .await
            .unwrap()
    }

    pub async fn create_user(&self, user: &User) -> User {
        self.users
            .create(&mut TxContext::none(), user)
            .await
            .unwrap()
    }

    pub async fn create_session(&self, session: &Session) -> Session {
        self.sessions
            .create(&mut TxContext::none(), session)
            .await
            .unwrap()
    }

    async fn with_pool(pool: SqlitePool) -> Self {
        crate::infrastructure::sqlite::migrate(&pool).await.unwrap();

        let transactions = Arc::new(SqliteUnitOfWork::new(pool.clone()));

        Self {
            machines: Arc::new(EncryptedSqliteMachineRepository::new(
                pool.clone(),
                Arc::clone(&transactions) as Arc<_>,
                cipher(),
            )),
            users: Arc::new(SqliteUserRepository::new(
                pool.clone(),
                Arc::clone(&transactions) as Arc<_>,
            )),
            passwords: Arc::new(EncryptedSqlitePasswordRepository::new(
                pool.clone(),
                cipher(),
            )),
            totps: Arc::new(EncryptedSqliteTotpRepository::new(pool.clone(), cipher())),
            passkeys: Arc::new(SqlitePasskeyRepository::new(pool.clone())),
            recovery_codes: Arc::new(SqliteRecoveryCodeRepository::new(pool.clone())),
            pending_account_recoveries: Arc::new(SqlitePendingAccountRecoveryRepository::new(
                pool.clone(),
            )),
            sessions: Arc::new(SqliteSessionRepository::new(pool.clone())),
            pending_sign_ins: Arc::new(SqlitePendingSignInRepository::new(pool.clone())),
            failed_sign_in_attempts: Arc::new(SqliteFailedSignInAttemptRepository::new(
                pool.clone(),
            )),
            transactions,
            audit: Arc::new(SqliteAuditRepository::new(pool.clone())),
            pool,
        }
    }
}

/// A [`Registry`] that removes its database directory when the test ends.
pub struct TemporaryRegistry {
    registry: Registry,
    directory: PathBuf,
}

impl Deref for TemporaryRegistry {
    type Target = Registry;

    fn deref(&self) -> &Registry {
        &self.registry
    }
}

impl Drop for TemporaryRegistry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn cipher() -> Arc<XChaCha20Poly1305Cipher> {
    Arc::new(XChaCha20Poly1305Cipher::new(&SecretBox::new(Box::new(
        CIPHER_KEY,
    ))))
}
