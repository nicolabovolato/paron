use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;

use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use paron_core::application::audit::AuditService;
use paron_core::application::events::{EventPublisher, EventService, SubscriptionScope};
use paron_core::application::identity::authentication::account_recovery::{
    AccountRecoveryDeps, AccountRecoveryService,
};
use paron_core::application::identity::authentication::credentials::CredentialService;
use paron_core::application::identity::authentication::failed_sign_in_attempts::FailureDelay;
use paron_core::application::identity::authentication::methods::passkey::{
    PasskeyRegistration, PasskeyService,
};
use paron_core::application::identity::authentication::methods::password::PasswordService;
use paron_core::application::identity::authentication::methods::recovery_code::RecoveryCodeService;
use paron_core::application::identity::authentication::methods::totp::{
    TotpRegistration, TotpService,
};
use paron_core::application::identity::authentication::sessions::{SessionPolicy, SessionService};
use paron_core::application::identity::authentication::sign_in::{
    AuthenticationDeps, AuthenticationService, SignInPolicy,
};
use paron_core::application::identity::users::UserService;
use paron_core::application::machine::{
    self, EnrollmentPolicy, MachineAuthenticationService, MachineConnectionService, MachineService,
};
use paron_core::application::repository::TxContext;
use paron_core::application::tasks::Tasks;
use paron_core::infrastructure::clock::SystemClock;
use paron_core::infrastructure::crypto::cipher::XChaCha20Poly1305Cipher;
use paron_core::infrastructure::crypto::keys::RusshKeypairs;
use paron_core::infrastructure::crypto::password::BoundedArgon2PasswordHasher;
use paron_core::infrastructure::crypto::tokens::Hmac;
use paron_core::infrastructure::http;
use paron_core::infrastructure::http::public_origins::PublicOrigins;
use paron_core::infrastructure::http::state::Deps;
use paron_core::infrastructure::in_memory::credential_registrations::InMemoryRegistrationRepository;
use paron_core::infrastructure::in_memory::events::InMemoryEventStream;
use paron_core::infrastructure::in_memory::sequences::InMemorySequenceStore;
use paron_core::infrastructure::sqlite::audit::repository::SqliteAuditRepository;
use paron_core::infrastructure::sqlite::identity::failed_sign_in_attempts::SqliteFailedSignInAttemptRepository;
use paron_core::infrastructure::sqlite::identity::passkeys::SqlitePasskeyRepository;
use paron_core::infrastructure::sqlite::identity::passwords::EncryptedSqlitePasswordRepository;
use paron_core::infrastructure::sqlite::identity::pending_account_recoveries::SqlitePendingAccountRecoveryRepository;
use paron_core::infrastructure::sqlite::identity::pending_sign_ins::SqlitePendingSignInRepository;
use paron_core::infrastructure::sqlite::identity::recovery_codes::SqliteRecoveryCodeRepository;
use paron_core::infrastructure::sqlite::identity::sessions::SqliteSessionRepository;
use paron_core::infrastructure::sqlite::identity::totps::EncryptedSqliteTotpRepository;
use paron_core::infrastructure::sqlite::identity::users::SqliteUserRepository;
use paron_core::infrastructure::sqlite::machine::repository::EncryptedSqliteMachineRepository;
use paron_core::infrastructure::sqlite::transaction::SqliteUnitOfWork;
use paron_core::infrastructure::sqlite::{self, SqliteTx};
use paron_core::infrastructure::transport::Transport;
use paron_core::infrastructure::transport::tunnel::MachinePolicy;
use paron_tunnel_protocol::{Broker, BrokerAddress, ConnectedAgents, Liveness};
use std::time::Duration;

use super::cli::PublicUrl;
use super::data_dir;
use crate::Result;

const DATABASE_ENCRYPTION_HKDF: &[u8] = b"paron database v1";
const ENROLLMENT_TOKEN_MAC_HKDF: &[u8] = b"paron enrollment token v1";
const SESSION_TOKEN_MAC_HKDF: &[u8] = b"paron session token v1";
const ACCOUNT_RECOVERY_TOKEN_MAC_HKDF: &[u8] = b"paron account recovery token v1";
const SIGN_IN_TOKEN_MAC_HKDF: &[u8] = b"paron sign-in token v1";
const RECOVERY_CODE_MAC_HKDF: &[u8] = b"paron recovery code v1";
const DEVICE_TOKEN_MAC_HKDF: &[u8] = b"paron device token v1";

const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const HTTP_CONFIG: http::Config = http::Config {
    expose_docs: cfg!(debug_assertions),
    websocket_keepalive_interval: Duration::from_secs(30),
};

const ENROLLMENT_POLICY: EnrollmentPolicy = EnrollmentPolicy {
    invitation_lifetime: Duration::from_mins(15),
};

const ACCOUNT_RECOVERY_LIFETIME: Duration = Duration::from_hours(24);
const PASSKEY_CEREMONY_LIFETIME: Duration = Duration::from_mins(5);
const TOTP_REGISTRATION_LIFETIME: Duration = Duration::from_mins(5);
const RECOVERY_CODE_COUNT: usize = 10;

const SIGN_IN_POLICY: SignInPolicy = SignInPolicy {
    lifetime: PASSKEY_CEREMONY_LIFETIME,
    password_failure_delay: FailureDelay {
        free_failures: 5,
        first_delay: Duration::from_secs(1),
        max_delay: Duration::from_mins(15),
        forget_after: Duration::from_hours(12),
    },
    totp_failure_delay: FailureDelay {
        free_failures: 10,
        first_delay: Duration::from_hours(24),
        max_delay: Duration::from_hours(24),
        forget_after: Duration::from_hours(24),
    },
    recovery_code_failure_delay: FailureDelay {
        free_failures: 5,
        first_delay: Duration::from_secs(1),
        max_delay: Duration::from_mins(15),
        forget_after: Duration::from_hours(12),
    },
};

const SESSION_POLICY: SessionPolicy = SessionPolicy {
    lifetime: Duration::from_hours(24 * 7),
    max_idle_timeout: Duration::from_hours(12),
    touch_interval: Duration::from_mins(5),
    expired_sessions_per_delete_batch: 1000,
    recent_authentication_window: Duration::from_mins(15),
    max_sessions_per_user: 100,
};

const TERMINAL_IDLE_TIMEOUT: Duration = SESSION_POLICY.max_idle_timeout;

const PROTOCOL_LIVENESS: Liveness = Liveness::DEFAULT;

const DATABASE_CONNECTIONS: std::num::NonZeroUsize = std::num::NonZeroUsize::new(8).unwrap();

const GARBAGE_COLLECTION_INTERVAL: Duration = Duration::from_mins(1);

#[derive(Debug)]
pub struct Limits {
    pub tunnel: paron_tunnel_protocol::Limits,
    pub concurrent_password_hashes: std::num::NonZeroUsize,
    pub database_connections: std::num::NonZeroUsize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            tunnel: paron_tunnel_protocol::Limits::default(),
            concurrent_password_hashes: std::num::NonZeroUsize::MIN,
            database_connections: DATABASE_CONNECTIONS,
        }
    }
}

pub struct Paron {
    pub tunnel: Arc<Broker>,
    pub http: axum::Router,
    pub users: Arc<UserService<SqliteTx>>,
    pub account_recovery: Arc<AccountRecoveryService<SqliteTx>>,

    sessions: Arc<SessionService<SqliteTx>>,
    authentication: Arc<AuthenticationService<SqliteTx>>,
    totp_registration_repo: Arc<InMemoryRegistrationRepository<TotpRegistration>>,
    passkey_registration_repo: Arc<InMemoryRegistrationRepository<PasskeyRegistration>>,
    machine_connections: Arc<MachineConnectionService<SqliteTx>>,
    events: Arc<EventService>,
    tasks: Tasks,
    pool: sqlx::SqlitePool,
}

impl Paron {
    #[allow(clippy::too_many_lines)]
    pub async fn new(
        data_dir: data_dir::Contents,
        limits: Limits,
        public_urls: &[PublicUrl],
        listen_port: u16,
    ) -> Result<Self> {
        let public_urls: Vec<PublicUrl> = public_urls
            .iter()
            .cloned()
            .chain(
                std::iter::once(listen_port)
                    .chain(public_urls.iter().map(PublicUrl::port))
                    .collect::<BTreeSet<u16>>()
                    .into_iter()
                    .flat_map(|port| super::local_hosts().map(|host| PublicUrl::new(host, port))),
            )
            .collect();
        let canonical = &public_urls[0];

        let pool = open_database(&data_dir.database_path, limits.database_connections).await?;

        let tunnels = Arc::new(ConnectedAgents::new());

        let unit_of_work = Arc::new(SqliteUnitOfWork::new(pool.clone()));

        let event_stream = Arc::new(InMemoryEventStream::default());
        let events = Arc::new(EventService::new(
            Arc::clone(&event_stream) as Arc<dyn EventPublisher>,
            event_stream,
            Arc::new(SystemClock),
        ));
        let audit = Arc::new(AuditService::new(
            Arc::new(SqliteAuditRepository::new(pool.clone())),
            Arc::new(SystemClock),
        ));

        let cipher = Arc::new(XChaCha20Poly1305Cipher::new(
            &data_dir.keys.derive(DATABASE_ENCRYPTION_HKDF),
        ));
        let machine_repo = Arc::new(EncryptedSqliteMachineRepository::new(
            pool.clone(),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&cipher) as Arc<_>,
        ));
        let user_repo = Arc::new(SqliteUserRepository::new(
            pool.clone(),
            Arc::clone(&unit_of_work) as Arc<_>,
        ));
        let password_repo = Arc::new(EncryptedSqlitePasswordRepository::new(
            pool.clone(),
            Arc::clone(&cipher) as Arc<_>,
        ));
        let totp_repo = Arc::new(EncryptedSqliteTotpRepository::new(
            pool.clone(),
            Arc::clone(&cipher) as Arc<_>,
        ));
        let totp_registration_repo = Arc::new(
            InMemoryRegistrationRepository::<TotpRegistration>::new(Arc::new(SystemClock)),
        );
        let passkey_registration_repo = Arc::new(InMemoryRegistrationRepository::<
            PasskeyRegistration,
        >::new(Arc::new(SystemClock)));
        let session_repo = Arc::new(SqliteSessionRepository::new(pool.clone()));
        let password_hasher = Arc::new(BoundedArgon2PasswordHasher::new(
            limits.concurrent_password_hashes.get(),
        ));

        let session_service = Arc::new(SessionService::new(
            session_repo as Arc<_>,
            Arc::new(Hmac::new(data_dir.keys.derive(SESSION_TOKEN_MAC_HKDF))) as Arc<_>,
            Arc::new(SystemClock),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&events),
            SESSION_POLICY,
        ));
        let user_service = Arc::new(UserService::new(
            user_repo as Arc<_>,
            Arc::clone(&session_service),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&events),
            Arc::new(SystemClock),
        ));
        let password_service = Arc::new(PasswordService::new(
            password_repo,
            password_hasher,
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::new(SystemClock),
        ));
        let totp_service = Arc::new(TotpService::new(
            totp_repo,
            Arc::clone(&totp_registration_repo) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::new(SystemClock),
            TOTP_REGISTRATION_LIFETIME,
            canonical.host().clone(),
        ));
        let passkey_service = Arc::new(PasskeyService::new(
            Arc::new(SqlitePasskeyRepository::new(pool.clone())),
            Arc::clone(&passkey_registration_repo) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::new(SystemClock),
            PASSKEY_CEREMONY_LIFETIME,
        ));
        let recovery_code_service = Arc::new(RecoveryCodeService::new(
            Arc::new(SqliteRecoveryCodeRepository::new(pool.clone())),
            Arc::new(Hmac::new(data_dir.keys.derive(RECOVERY_CODE_MAC_HKDF))),
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::new(SystemClock),
            RECOVERY_CODE_COUNT,
        ));
        let credential_service = Arc::new(CredentialService::new(
            password_service,
            totp_service,
            passkey_service,
            recovery_code_service,
            Arc::clone(&user_service),
            Arc::clone(&session_service),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::new(SystemClock),
        ));
        let account_recovery_service = Arc::new(AccountRecoveryService::new(AccountRecoveryDeps {
            users: Arc::clone(&user_service),
            credentials: Arc::clone(&credential_service),
            repository: Arc::new(SqlitePendingAccountRecoveryRepository::new(pool.clone())),
            tokens: Arc::new(Hmac::new(
                data_dir.keys.derive(ACCOUNT_RECOVERY_TOKEN_MAC_HKDF),
            )),
            unit_of_work: Arc::clone(&unit_of_work) as Arc<_>,
            audit: Arc::clone(&audit),
            clock: Arc::new(SystemClock),
            token_lifetime: ACCOUNT_RECOVERY_LIFETIME,
        }));
        let authentication_service = Arc::new(AuthenticationService::new(AuthenticationDeps {
            users: Arc::clone(&user_service),
            sessions: Arc::clone(&session_service),
            credentials: Arc::clone(&credential_service),
            pending_sign_ins: Arc::new(SqlitePendingSignInRepository::new(pool.clone())),
            sign_in_tokens: Arc::new(Hmac::new(data_dir.keys.derive(SIGN_IN_TOKEN_MAC_HKDF))),
            device_tokens: Arc::new(Hmac::new(data_dir.keys.derive(DEVICE_TOKEN_MAC_HKDF))),
            failed_sign_in_attempts: Arc::new(SqliteFailedSignInAttemptRepository::new(
                pool.clone(),
            )),
            unit_of_work: Arc::clone(&unit_of_work) as Arc<_>,
            audit: Arc::clone(&audit),
            clock: Arc::new(SystemClock),
            policy: SIGN_IN_POLICY,
        }));

        let transport = Arc::new(Transport::new(Arc::clone(&tunnels), SSH_KEEPALIVE_INTERVAL));
        let keypairs = Arc::new(RusshKeypairs);
        let machine_service = Arc::new(MachineService::new(
            machine_repo,
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&keypairs) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&events),
        ));
        let machine_authentication_service = Arc::new(MachineAuthenticationService::new(
            Arc::clone(&machine_service),
            unit_of_work as Arc<_>,
            Arc::new(Hmac::new(data_dir.keys.derive(ENROLLMENT_TOKEN_MAC_HKDF))),
            Arc::new(SystemClock),
            ENROLLMENT_POLICY,
            data_dir.tunnel_identity.root_public_key().fingerprint(),
            BrokerAddress::try_from(canonical)?,
        ));

        let tasks = Tasks::default();

        let connection_service = Arc::new(MachineConnectionService::new(
            Arc::clone(&machine_service),
            transport,
            keypairs,
            Arc::clone(&audit),
            Arc::clone(&events),
            Arc::clone(&session_service) as Arc<_>,
            tasks.clone(),
            TERMINAL_IDLE_TIMEOUT,
        ));

        let http = http::router(Deps {
            machine_connections: Arc::clone(&connection_service) as Arc<_>,
            machines: machine_service,
            machine_enrollment: Arc::clone(&machine_authentication_service) as Arc<_>,
            authentication: Arc::clone(&authentication_service) as Arc<_>,
            credentials: Arc::clone(&credential_service) as Arc<_>,
            account_recovery: Arc::clone(&account_recovery_service) as Arc<_>,
            users: Arc::clone(&user_service) as Arc<_>,
            audit,
            public_origins: Arc::new(PublicOrigins::new(
                url::Origin::from(canonical),
                public_urls.iter().map(url::Origin::from),
            )),
            config: HTTP_CONFIG,
        });

        Ok(Self {
            http,
            users: user_service,
            account_recovery: account_recovery_service,
            sessions: session_service,
            authentication: authentication_service,
            totp_registration_repo,
            passkey_registration_repo,
            machine_connections: connection_service,
            events,
            tasks,
            pool,
            tunnel: Arc::new(Broker::new(
                Arc::new(MachinePolicy::new(
                    machine_authentication_service,
                    Arc::new(InMemorySequenceStore::new()),
                )),
                tunnels,
                data_dir.tunnel_identity,
                PROTOCOL_LIVENESS,
                limits.tunnel,
            )),
        })
    }

    pub async fn background_tasks(&self) -> Result<Vec<BoxFuture<'static, ()>>> {
        fn collect_garbage<F, Fut, E>(name: &'static str, mut collect: F) -> BoxFuture<'static, ()>
        where
            F: FnMut() -> Fut + Send + 'static,
            Fut: Future<Output = Result<(), E>> + Send,
            E: std::fmt::Display,
        {
            async move {
                loop {
                    tokio::time::sleep(GARBAGE_COLLECTION_INTERVAL).await;

                    if let Err(error) = collect().await {
                        tracing::warn!("could not cleanup {name}: {error:#}");
                    }
                }
            }
            .boxed()
        }

        let sessions = Arc::clone(&self.sessions);
        let expired_sessions = collect_garbage("expired sessions", move || {
            let sessions = Arc::clone(&sessions);
            async move { sessions.delete_expired(&mut TxContext::none()).await }
        });

        let authentication = Arc::clone(&self.authentication);
        let expired_sign_ins = collect_garbage("expired sign-ins", move || {
            let authentication = Arc::clone(&authentication);
            async move {
                authentication
                    .delete_expired_sign_ins(&mut TxContext::none())
                    .await
            }
        });

        let authentication = Arc::clone(&self.authentication);
        let expired_sign_in_attempts = collect_garbage("expired sign-in attempts", move || {
            let authentication = Arc::clone(&authentication);
            async move {
                authentication
                    .delete_expired_sign_in_attempts(&mut TxContext::none())
                    .await
            }
        });

        let totp_registration_repo = Arc::clone(&self.totp_registration_repo);
        let expired_totp_registrations = collect_garbage("expired TOTP registrations", move || {
            totp_registration_repo.remove_expired();
            std::future::ready(Ok::<(), Infallible>(()))
        });

        let passkey_registration_repo = Arc::clone(&self.passkey_registration_repo);
        let expired_passkey_registrations =
            collect_garbage("expired passkey registrations", move || {
                passkey_registration_repo.remove_expired();
                std::future::ready(Ok::<(), Infallible>(()))
            });

        let machine_connections = Arc::clone(&self.machine_connections);
        let mut machine_events = self
            .events
            .subscribe::<machine::event::EventKind>(SubscriptionScope::All)
            .await?;
        let withdrawn_agents = async move {
            loop {
                match machine_events.next().await {
                    Ok(event) => machine_connections.handle_machine_event(&event).await,
                    Err(error) => {
                        tracing::error!("could not read the machine events: {error}");

                        return;
                    }
                }
            }
        };

        Ok(vec![
            expired_sessions,
            expired_sign_ins,
            expired_sign_in_attempts,
            expired_totp_registrations,
            expired_passkey_registrations,
            withdrawn_agents.boxed(),
        ])
    }

    pub async fn close(&self) {
        self.tasks.stop().await;
        self.pool.close().await;
    }
}

async fn open_database(
    path: &std::path::Path,
    connections: std::num::NonZeroUsize,
) -> Result<sqlx::SqlitePool> {
    let (connect, pool) = sqlite::requirements();
    let pool = pool
        .max_connections(u32::try_from(connections.get()).unwrap_or(u32::MAX))
        .connect_with(
            connect
                .filename(path)
                .optimize_on_close(true, None)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Full),
        )
        .await?;

    match sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await?
        .parse()?
    {
        sqlx::sqlite::SqliteJournalMode::Wal => {}
        mode => tracing::warn!(
            "the database is in {mode:?} mode and not WAL: expect service degradation"
        ),
    }

    sqlite::migrate(&pool).await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::{BTreeSet, HashSet};
    use std::future::IntoFuture as _;
    use std::net::Ipv4Addr;

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use paron_core::application::identity::authentication::account_recovery::{
        RecoverAccountCommand, RecoveryAction,
    };
    use paron_core::application::identity::authentication::methods::password::Password;
    use paron_core::application::identity::authorization::actor::Actor;
    use paron_core::application::identity::users::CreateUserCommand;
    use paron_core::application::repository::TxContext;
    use paron_core::domain::machine::tunnel::token::Token;
    use paron_core::domain::user::{DisplayName, Role, Username};
    use paron_tunnel_protocol::quic::QuicAgentTransport;
    use paron_tunnel_protocol::tcp::{TcpAgentTransport, TcpBrokerTransport};
    use paron_tunnel_protocol::{
        Agent, AgentIdentity, Identity, InMemoryRollbackIndex, PinnedBroker, Selector, Selectors,
        Sequence, TunnelError,
    };
    use secrecy::{ExposeSecret as _, SecretString};
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tower::ServiceExt as _;

    use super::*;
    use crate::server::data_dir::keys;

    use crate::testing::TempDataDir;

    const WAIT_LIMIT: Duration = Duration::from_secs(10);
    const ADMINISTRATOR: &str = "administrator";
    const PASSWORD: &str = "a long enough password";
    const HOST_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAVHDVsuJdkBBIZuX0LN98XZ4nbPf4TFfUIpCakYOML5";

    /// One counter for every connection of one agent. The broker refuses a sequence that starts
    /// again.
    #[derive(Default)]
    struct IncrementingSequence(std::sync::atomic::AtomicU64);

    impl Sequence for IncrementingSequence {
        fn next(&self) -> std::result::Result<u64, TunnelError> {
            Ok(self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1)
        }
    }

    async fn send(
        http: &axum::Router,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Option<String>, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "localhost");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = http.clone().oneshot(request).await.unwrap();
        let status = response.status();
        // The cookies that the response sets, as a `Cookie` header. A cookie that it clears has no
        // value, and is left out.
        let set_cookies: Vec<&str> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|cookie| cookie.to_str().ok()?.split(';').next())
            .filter(|cookie| !cookie.ends_with('='))
            .collect();
        let set_cookie = (!set_cookies.is_empty()).then(|| set_cookies.join("; "));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        (
            status,
            set_cookie,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
        )
    }

    fn agent_for(
        token: &Token,
        identity: Identity,
        echo_port: u16,
        sequence: Arc<dyn Sequence>,
    ) -> Agent {
        Agent::new(
            vec![Box::new(QuicAgentTransport), Box::new(TcpAgentTransport)],
            PinnedBroker {
                address: token.server_endpoint.address.clone(),
                root: token.server_endpoint.fingerprint.clone(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id: token.server_endpoint.machine,
                keys: identity,
            },
            Arc::new(Selectors::new([Selector::tcp(echo_port)
                .expect("a bound port is a selector")
                .into()])),
            sequence,
            Liveness::default(),
        )
    }

    async fn start_echo_server() -> u16 {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut read_half, mut write_half) = socket.split();
                    let _ = tokio::io::copy(&mut read_half, &mut write_half).await;
                });
            }
        });

        port
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn enroll_then_unenroll_admits_the_agent_then_cuts_it_off() {
        let data_dir = TempDataDir::new("paron-enroll-unenroll");
        let contents = crate::server::data_dir::init(data_dir.path()).unwrap();
        let paron = Paron::new(contents, Limits::default(), &[], 443)
            .await
            .unwrap();
        for task in paron.background_tasks().await.unwrap() {
            tokio::spawn(task);
        }
        let bound_broker = paron
            .tunnel
            .bind(&TcpBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap();
        let broker_address = bound_broker.address();
        tokio::spawn(bound_broker.into_future());
        let echo_port = start_echo_server().await;
        let administrator = paron
            .users
            .create(
                &mut TxContext::none(),
                &Actor::System,
                CreateUserCommand {
                    username: Username::parse(ADMINISTRATOR).unwrap(),
                    display_name: DisplayName::parse(ADMINISTRATOR).unwrap(),
                    roles: BTreeSet::from([Role::Administrator]),
                },
            )
            .await
            .unwrap();
        let link = paron
            .account_recovery
            .start_recovery(&mut TxContext::none(), &Actor::System, administrator.id)
            .await
            .unwrap();
        paron
            .account_recovery
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token: link.token,
                    action: RecoveryAction::SetPassword(Password::parse(PASSWORD.into()).unwrap()),
                },
            )
            .await
            .unwrap();
        let (_, sign_in_token, _) =
            send(&paron.http, Method::POST, "/api/auth/sign-in", None, None).await;
        let (_, cookie, _) = send(
            &paron.http,
            Method::POST,
            "/api/auth/sign-in/verify",
            sign_in_token.as_deref(),
            Some(json!({ "kind": "password", "username": ADMINISTRATOR, "password": PASSWORD })),
        )
        .await;
        let cookie = cookie.expect("a sign-in sets a session cookie");
        let (status, _, machine) = send(
            &paron.http,
            Method::POST,
            "/api/machines",
            Some(&cookie),
            Some(json!({
                "name": "build",
                "connectivity": { "kind": "tunneled" },
                "protocols": {
                    "ssh": {
                        "user": "deploy",
                        "port": 22,
                        "authentication": {
                            "host_key": HOST_KEY,
                            "method": { "kind": "password", "password": "hunter2" },
                        },
                        "connect_timeout_seconds": 30,
                    },
                },
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{machine}");
        let enrollment_path = format!(
            "/api/machines/{}/enrollment",
            machine["id"].as_str().unwrap()
        );
        let (status, _, invitation) = send(
            &paron.http,
            Method::POST,
            &enrollment_path,
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{invitation}");
        let mut token: Token = invitation["token"].as_str().unwrap().parse().unwrap();
        token.server_endpoint.address = broker_address.to_string().parse().unwrap();
        let identity_der = Identity::generate().unwrap().private_key_der().clone();
        let identity = move || Identity::from_pkcs8_der(&identity_der).unwrap();
        let sequence: Arc<dyn Sequence> = Arc::new(IncrementingSequence::default());
        let agent = Arc::new(agent_for(
            &token,
            identity(),
            echo_port,
            Arc::clone(&sequence),
        ));

        assert!(
            agent.join(&SecretString::from("guessed")).await.is_err(),
            "a wrong secret must not enroll"
        );
        agent
            .join(&token.secret)
            .await
            .expect("the issued secret enrolls");
        let serve_task = tokio::spawn({
            let agent = Arc::clone(&agent);
            async move { agent.serve(std::future::pending()).await }
        });
        let tunnel = tokio::time::timeout(
            WAIT_LIMIT,
            paron
                .tunnel
                .connected_agents()
                .wait_for_tunnel_to(token.server_endpoint.machine),
        )
        .await
        .expect("the machine becomes reachable in time");
        let mut stream = tunnel
            .open(&Selector::tcp(echo_port).unwrap())
            .await
            .expect("an enrolled machine is reachable through its tunnel");
        stream.write_all(b"over-the-tunnel").await.unwrap();
        let mut echoed = [0u8; b"over-the-tunnel".len()];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"over-the-tunnel");

        let (status, _, _) = send(
            &paron.http,
            Method::DELETE,
            &enrollment_path,
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let served = tokio::time::timeout(WAIT_LIMIT, serve_task)
            .await
            .expect("the agent of an unenrolled machine is cut off in time")
            .unwrap();
        assert_matches!(served, Ok(_));
        let mut after_the_cut_off = [0u8; 1];
        let read = tokio::time::timeout(WAIT_LIMIT, stream.read(&mut after_the_cut_off))
            .await
            .expect("the open stream of the agent ends in time");
        assert_matches!(read, Ok(0) | Err(_));
        let refused = agent_for(&token, identity(), echo_port, sequence)
            .serve(std::future::ready(()))
            .await;
        assert_matches!(
            refused,
            Err(TunnelError::Refused),
            "the key of an unenrolled machine is no longer admitted"
        );
        paron.close().await;
    }

    #[test]
    fn every_service_gets_a_different_key() {
        let data_dir = TempDataDir::new("services-keys");
        let master_key = keys::mint(data_dir.path()).unwrap();

        let service_keys: HashSet<_> = [
            DATABASE_ENCRYPTION_HKDF,
            ENROLLMENT_TOKEN_MAC_HKDF,
            SESSION_TOKEN_MAC_HKDF,
        ]
        .map(|label| *master_key.derive(label).expose_secret())
        .into();

        assert_eq!(service_keys.len(), 3);
    }
}
