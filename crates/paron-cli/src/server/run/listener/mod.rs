pub mod admission;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls_acme::is_tls_alpn_challenge;
use tokio::net::{TcpListener, TcpStream};

use super::http;
use super::proxy::{self, Proxies};
use super::tls::{self, Tls};
use crate::Result;
use admission::{
    AdmissionPermit, AdmissionRefused, AdmissionSemaphore, LoadSheddingSemaphore, TotalSemaphore,
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub web_connections: usize,
    pub web_connections_per_source: usize,
    pub pending_connections_per_source: usize,
    pub pending_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            web_connections: 512,
            web_connections_per_source: 64,
            pending_connections: 128,
            pending_connections_per_source: 32,
        }
    }
}

const MAX_PENDING_CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_ACCEPT_MAX_CONSECUTIVE_FAILURES: usize = 1024;

pub async fn serve(
    listener: TcpListener,
    tls: Tls,
    router: axum::Router,
    tunnel: paron_tunnel_protocol::tcp::TcpBroker,
    proxies: Proxies,
    limits: Limits,
    stop_signal: tokio_util::sync::CancellationToken,
) -> Result<()> {
    if proxies.l4.is_empty() && proxies.l7.is_empty() {
        tracing::debug!(
            "no trusted proxies configured, every connection is treated as coming from a client"
        );
    }

    if !proxies.l4.is_empty() {
        tracing::info!("trusted L4 proxies configured: {}", *proxies.l4);
    }

    if !proxies.l7.is_empty() {
        tracing::info!("trusted L7 proxies configured: {}", *proxies.l7);
    }

    let connection_state = Arc::new(ConnectionState {
        acme_challenges: tls.acme_challenges,
        tunnel,
        proxies,
        connections: AdmissionSemaphore::new(
            TotalSemaphore::Hard(Arc::new(tokio::sync::Semaphore::new(
                limits.web_connections,
            ))),
            limits.web_connections_per_source,
        ),
        tls: tls.config,
        router,
        stop_signal: stop_signal.clone(),
    });

    let mut acceptor = BackoffTcpAcceptor::new(listener, TCP_ACCEPT_MAX_CONSECUTIVE_FAILURES);
    let pending_connections = AdmissionSemaphore::new(
        TotalSemaphore::Shedding(LoadSheddingSemaphore::new(limits.pending_connections)),
        limits.pending_connections_per_source,
    );
    let connections = tokio_util::task::TaskTracker::new();

    let ended = loop {
        let (tcp, peer) = match acceptor.next(&stop_signal).await {
            Ok(Some(accepted)) => accepted,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        };

        if let Err(error) = tcp.set_nodelay(true) {
            tracing::debug!(%peer, "could not turn off Nagle's algorithm: {error}");
        }

        let source = (!connection_state.proxies.l4.contains(peer.ip())
            && !connection_state.proxies.l7.contains(peer.ip()))
        .then(|| peer.ip());

        let pending = match pending_connections.try_acquire_owned(source) {
            Ok(pending) => pending,
            Err(AdmissionRefused::Total) => {
                tracing::debug!(%peer, "connection refused: too many in flight");

                continue;
            }
            Err(AdmissionRefused::Source) => {
                tracing::debug!(%peer, "connection refused: too many in flight from this address");

                continue;
            }
        };

        let connection_state = Arc::clone(&connection_state);

        connections.spawn(async move {
            handle(&connection_state, tcp, peer, pending).await;
        });
    };

    // Stop accepting new connections
    drop(acceptor);

    connections.close();

    if ended.is_ok() {
        connections.wait().await;
    }

    ended
}

struct ConnectionState {
    acme_challenges: Vec<tls::acme::Certificate>,
    tunnel: paron_tunnel_protocol::tcp::TcpBroker,
    proxies: Proxies,
    connections: AdmissionSemaphore,
    tls: Arc<rustls::ServerConfig>,
    router: axum::Router,
    stop_signal: tokio_util::sync::CancellationToken,
}

async fn handle(
    state: &ConnectionState,
    mut tcp: TcpStream,
    peer: SocketAddr,
    pending_connection_guard: AdmissionPermit,
) {
    let timeout: tokio::time::Instant =
        tokio::time::Instant::now() + MAX_PENDING_CONNECTION_TIMEOUT;

    let peer = if state.proxies.l4.contains(peer.ip()) {
        let result = tokio::time::timeout_at(timeout, proxy::client_from_proxy_protocol(&mut tcp));

        match state.stop_signal.run_until_cancelled(result).await {
            None => return,
            Some(Err(_)) => {
                tracing::debug!(%peer, "trusted proxy sent nothing in time, connection dropped");
                return;
            }
            Some(Ok(Err(error))) => {
                tracing::debug!(%peer, "could not read the PROXY header: {error}");
                return;
            }
            Some(Ok(Ok(client))) => client.unwrap_or(peer),
        }
    } else {
        // without this, it would trigger for every l7 proxy that is not also doing l4
        if !state.proxies.l7.contains(peer.ip()) {
            proxy::report_untrusted_l4_peer(peer, &state.proxies.l4);
        }

        peer
    };

    let result = tokio::time::timeout_at(
        timeout,
        // Reads only ClientHello and returns the handle to start the tls handshake
        tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp),
    );

    let Some(result) = state.stop_signal.run_until_cancelled(result).await else {
        return;
    };

    let tls_handshake = match result {
        Ok(Ok(start)) => start,
        Ok(Err(error)) => {
            tracing::debug!(%peer, "not a TLS connection: {error}");
            return;
        }
        Err(_) => {
            tracing::debug!(%peer, "no ClientHello in time, connection dropped");
            return;
        }
    };

    match tls_handshake.client_hello() {
        hello if paron_tunnel_protocol::tcp::is_ptp1(&hello) => {
            // The tunnel protocol applies its own limits from here on
            drop(pending_connection_guard);

            state
                .tunnel
                .serve(tls_handshake, peer, &state.stop_signal)
                .await;
        }
        hello => {
            let source = (!state.proxies.l4.contains(peer.ip())
                && !state.proxies.l7.contains(peer.ip()))
            .then(|| peer.ip());

            let _admitted = match state.connections.try_acquire_owned(source) {
                Ok(admitted) => admitted,
                Err(AdmissionRefused::Total) => {
                    tracing::debug!(%peer, "connection refused: limit reached");

                    return;
                }
                Err(AdmissionRefused::Source) => {
                    tracing::debug!(%peer, "connection refused: per address limit reached");

                    return;
                }
            };

            match hello {
                hello if is_tls_alpn_challenge(&hello) => {
                    let handshake = tokio::time::timeout_at(
                        timeout,
                        tls::acme::handshake(tls_handshake, peer, &state.acme_challenges),
                    );

                    if let Some(Err(_)) = state.stop_signal.run_until_cancelled(handshake).await {
                        tracing::debug!(%peer, "ACME challenge handshake did not finish in time, connection dropped");
                    }

                    drop(pending_connection_guard);
                }
                // anything else will be be served (or rejected) by the http path
                _ => {
                    let handshake = tokio::time::timeout_at(
                        timeout,
                        tls_handshake.into_stream(Arc::clone(&state.tls)),
                    );

                    let tls = match state.stop_signal.run_until_cancelled(handshake).await {
                        Some(Ok(Ok(tls))) => tls,
                        Some(Ok(Err(error))) => {
                            tracing::debug!(%peer, "could not finish the TLS handshake: {error}");

                            return;
                        }
                        Some(Err(_)) => {
                            tracing::debug!(%peer, "HTTPS handshake did not finish in time, connection dropped");

                            return;
                        }
                        None => return,
                    };

                    // this path has its own connection limits
                    drop(pending_connection_guard);

                    http::connection::serve(&state.router, tls, peer, &state.stop_signal).await;
                }
            }
        }
    }
}

/// A spare descriptor, so a process that has run out can still accept a queued connection and
/// close it. Without one the client stays connected until its own timeout. libuv does the same.
struct OpenFileReserve(Option<std::fs::File>);

impl OpenFileReserve {
    fn new() -> Self {
        Self(Self::held())
    }

    fn held() -> Option<std::fs::File> {
        std::fs::File::open(crate::platform::files::NULL_DEVICE).ok()
    }

    /// Frees the spare descriptor, accepts and immediately closes everything already queued, then takes a
    /// spare back.
    fn refuse_queued(&mut self, listener: &TcpListener) {
        let Some(reserved) = self.0.take() else {
            // Descriptors are process wide, so another worker can take the one this frees.
            self.0 = Self::held();

            return;
        };

        drop(reserved);

        match crate::platform::net::close_queued_connections(listener) {
            0 => {}
            refused => tracing::debug!("refused {refused} connections: out of open files"),
        }

        self.0 = Self::held();
    }
}

/// Accepts TCP connections. Backs off after running out of open files, after an unknown error,
/// and after too many connection errors in a row.
///
/// Errors on accept failures caused by bad descriptor errors
struct BackoffTcpAcceptor {
    listener: TcpListener,
    backoff: Duration,
    failures: usize,
    max_consecutive_failures: usize,
    reserve: OpenFileReserve,
}

impl BackoffTcpAcceptor {
    const MIN_ACCEPT_BACKOFF: Duration = Duration::from_millis(5);
    const MAX_ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

    fn new(listener: TcpListener, max_consecutive_failures: usize) -> Self {
        Self {
            listener,
            backoff: Self::MIN_ACCEPT_BACKOFF,
            failures: 0,
            max_consecutive_failures,
            reserve: OpenFileReserve::new(),
        }
    }

    async fn next(
        &mut self,
        stop_signal: &tokio_util::sync::CancellationToken,
    ) -> Result<Option<(TcpStream, SocketAddr)>> {
        loop {
            let accepted = tokio::select! {
                accepted = self.listener.accept() => accepted,
                () = stop_signal.cancelled() => return Ok(None),
            };

            match accepted {
                Ok(accepted) => {
                    self.backoff = Self::MIN_ACCEPT_BACKOFF;
                    self.failures = 0;

                    return Ok(Some(accepted));
                }
                Err(error) => {
                    use crate::platform::net::AcceptFailure;

                    self.failures += 1;

                    match crate::platform::net::accept_failure(&error) {
                        AcceptFailure::BadListener => {
                            return Err(
                                format!("could not keep accepting connections: {error}").into()
                            );
                        }
                        AcceptFailure::BadConnection
                            if self.failures <= self.max_consecutive_failures =>
                        {
                            continue;
                        }
                        AcceptFailure::OutOfOpenFiles => {
                            self.reserve.refuse_queued(&self.listener);
                        }
                        AcceptFailure::BadConnection | AcceptFailure::Unknown => {}
                    }

                    tracing::warn!("could not accept a connection: {error}");

                    tokio::select! {
                        () = tokio::time::sleep(self.backoff) => {}
                        () = stop_signal.cancelled() => return Ok(None),
                    }

                    self.backoff = (self.backoff * 2).min(Self::MAX_ACCEPT_BACKOFF);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use paron_tunnel_protocol::tcp::TcpBroker;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio_rustls::client::TlsStream;
    use tokio_util::sync::CancellationToken;

    use super::proxy::{L4Proxies, L7Proxies};
    use super::*;
    use crate::server::cli::CertificateArgs;
    use crate::server::paron::Paron;
    use crate::server::run::background_tasks::BackgroundTasks;
    use crate::testing::TempDataDir;

    const ROOMY_LIMITS: Limits = Limits {
        web_connections: 100,
        web_connections_per_source: 100,
        pending_connections: 100,
        pending_connections_per_source: 100,
    };

    struct RunningListener {
        address: SocketAddr,
        serve_task: tokio::task::JoinHandle<Result<()>>,
        stop_signal: CancellationToken,
        paron: Paron,
        _data_dir: TempDataDir,
    }

    impl RunningListener {
        async fn stop(self) {
            self.stop_signal.cancel();
            self.serve_task.await.unwrap().unwrap();
            self.paron.close().await;
        }
    }

    async fn start_listener(
        data_dir_name: &str,
        limits: Limits,
        l4_proxies: L4Proxies,
        l7_proxies: L7Proxies,
    ) -> RunningListener {
        let data_dir = TempDataDir::new(data_dir_name);
        let contents = crate::server::data_dir::init(data_dir.path()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = Paron::new(
            contents,
            crate::server::paron::Limits::default(),
            &[],
            address.port(),
        )
        .await
        .unwrap();
        let (tls, self_signed) = tls::configure(
            data_dir.path(),
            &[],
            &CertificateArgs::default(),
            &mut BackgroundTasks::new(),
        )
        .unwrap();
        let router = http::build(
            &server,
            &l7_proxies,
            &mut BackgroundTasks::new(),
            self_signed,
            #[cfg(debug_assertions)]
            false,
        )
        .unwrap();
        let tunnel_broker = TcpBroker::new(Arc::clone(&server.tunnel)).unwrap();
        let stop_signal = CancellationToken::new();

        let serving = serve(
            listener,
            tls,
            router,
            tunnel_broker,
            Proxies {
                l4: l4_proxies,
                l7: l7_proxies,
            },
            limits,
            stop_signal.clone(),
        );

        RunningListener {
            address,
            serve_task: tokio::spawn(serving),
            stop_signal,
            paron: server,
            _data_dir: data_dir,
        }
    }

    fn loopback_if(is_listed: bool) -> Vec<proxy::NetworkAddress> {
        if is_listed {
            vec!["127.0.0.1".parse().unwrap()]
        } else {
            Vec::new()
        }
    }

    #[test]
    fn default_limits_give_one_source_less_than_the_whole() {
        let limits = Limits::default();

        assert!(limits.web_connections_per_source < limits.web_connections);
        assert!(limits.pending_connections_per_source < limits.pending_connections);
    }

    #[tokio::test]
    async fn refuse_queued_closes_the_queued_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let mut open_file_reserve = OpenFileReserve::new();

        open_file_reserve.refuse_queued(&listener);

        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut [0u8; 1]))
            .await
            .expect("the connection was left in the queue");
        assert_eq!(read.unwrap(), 0);
        assert!(
            open_file_reserve.0.is_some(),
            "the reserve was not taken back"
        );
    }

    #[derive(PartialEq, Eq, Debug)]
    enum ConnectionOutcome {
        Dropped,
        TreatedAsClient,
        TreatedAsTrustedProxy,
    }

    // The router keeps `x-request-id` from a trusted proxy and replaces it for a client.
    // The answer thus shows what the listener took the connection for.
    async fn connect_with_proxy_protocol_headers(
        address: SocketAddr,
        header_count: usize,
    ) -> ConnectionOutcome {
        let mut tcp = TcpStream::connect(address).await.unwrap();

        for _ in 0..header_count {
            tcp.write_all(b"PROXY TCP4 203.0.113.9 127.0.0.1 40000 443\r\n")
                .await
                .unwrap();
        }

        match tls_handshake(tcp, b"http/1.1").await {
            Ok(connection) => send_request_with_id(connection, "set-by-a-proxy").await,
            Err(_) => ConnectionOutcome::Dropped,
        }
    }

    async fn send_request_with_id(
        mut connection: TlsStream<TcpStream>,
        sent_request_id: &str,
    ) -> ConnectionOutcome {
        connection
            .write_all(
                format!(
                    "GET / HTTP/1.1\r\nHost: localhost\r\nx-request-id: {sent_request_id}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut response = Vec::new();
        connection.read_to_end(&mut response).await.unwrap();

        let response = String::from_utf8_lossy(&response);
        let Some((head, _)) = response.split_once("\r\n\r\n") else {
            return ConnectionOutcome::Dropped;
        };

        let returned_request_id = head
            .lines()
            .find_map(|line| line.strip_prefix("x-request-id: "))
            .expect("an answer carries a request id")
            .trim();

        if returned_request_id == sent_request_id {
            ConnectionOutcome::TreatedAsTrustedProxy
        } else {
            ConnectionOutcome::TreatedAsClient
        }
    }

    #[tokio::test]
    async fn serve_answers_every_pairing_of_the_two_lists() {
        for (name, proxy_protocol, forwarded_headers) in [
            ("neither", false, false),
            ("proxy-protocol", true, false),
            ("forwarded-headers", false, true),
            ("both", true, true),
        ] {
            let running = start_listener(
                &format!("listener-pairing-{name}"),
                ROOMY_LIMITS,
                L4Proxies::new(&loopback_if(proxy_protocol)),
                L7Proxies::new(&loopback_if(forwarded_headers)),
            )
            .await;

            let outcome_with_header = connect_with_proxy_protocol_headers(running.address, 1).await;
            let outcome_without_header =
                connect_with_proxy_protocol_headers(running.address, 0).await;

            // A PROXY header from an untrusted peer stays in the stream, and the TLS acceptor
            // reads it as a bad ClientHello.
            let expected_with_header = if proxy_protocol {
                ConnectionOutcome::TreatedAsClient
            } else {
                ConnectionOutcome::Dropped
            };
            let expected_without_header = if forwarded_headers {
                ConnectionOutcome::TreatedAsTrustedProxy
            } else {
                ConnectionOutcome::TreatedAsClient
            };
            assert_eq!(outcome_with_header, expected_with_header, "{name}");
            assert_eq!(outcome_without_header, expected_without_header, "{name}");
            running.stop().await;
        }
    }

    #[tokio::test]
    async fn serve_second_proxy_protocol_header_is_not_read() {
        let running = start_listener(
            "listener-two-headers",
            ROOMY_LIMITS,
            L4Proxies::new(&loopback_if(true)),
            L7Proxies::default(),
        )
        .await;

        let outcome = connect_with_proxy_protocol_headers(running.address, 2).await;

        assert_eq!(outcome, ConnectionOutcome::Dropped);
        running.stop().await;
    }

    #[tokio::test]
    async fn serve_listed_proxy_is_not_charged_a_source_share() {
        for (name, is_l4_proxy, is_l7_proxy) in [
            ("l4", true, false),
            ("l7", false, true),
            ("both", true, true),
        ] {
            let running = start_listener(
                &format!("listener-share-exempt-{name}"),
                Limits {
                    web_connections_per_source: 1,
                    ..ROOMY_LIMITS
                },
                L4Proxies::new(&loopback_if(is_l4_proxy)),
                L7Proxies::new(&loopback_if(is_l7_proxy)),
            )
            .await;
            let _first_connection = connect_tls(running.address).await.unwrap();

            let second_connection = connect_tls(running.address).await;

            assert!(second_connection.is_ok(), "{name}: {second_connection:?}");
            running.stop().await;
        }
    }

    #[tokio::test]
    async fn serve_stop_does_not_wait_for_a_connection_in_setup() {
        let running = start_listener(
            "listener-drain-setup",
            ROOMY_LIMITS,
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;
        let _silent_connection = TcpStream::connect(running.address).await.unwrap();
        // The listener accepts connections in order. When a later connection finishes its
        // handshake, the silent connection is in setup.
        drop(connect_tls(running.address).await.unwrap());

        tokio::time::timeout(Duration::from_secs(2), running.stop())
            .await
            .expect("the drain did not finish in time");
    }

    #[derive(Debug)]
    struct AcceptAnyServerCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
        fn verify_server_cert(
            &self,
            _: &rustls::pki_types::CertificateDer<'_>,
            _: &[rustls::pki_types::CertificateDer<'_>],
            _: &rustls::pki_types::ServerName<'_>,
            _: &[u8],
            _: rustls::pki_types::UnixTime,
        ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error>
        {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &rustls::pki_types::CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &rustls::pki_types::CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            crate::server::run::tls::provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    async fn connect_tls(address: SocketAddr) -> std::io::Result<TlsStream<TcpStream>> {
        connect_tls_with_alpn(address, b"http/1.1").await
    }

    async fn connect_tls_with_alpn(
        address: SocketAddr,
        alpn: &[u8],
    ) -> std::io::Result<TlsStream<TcpStream>> {
        tls_handshake(TcpStream::connect(address).await?, alpn).await
    }

    async fn tls_handshake(tcp: TcpStream, alpn: &[u8]) -> std::io::Result<TlsStream<TcpStream>> {
        let mut config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
            .with_no_client_auth();

        config.alpn_protocols = vec![alpn.to_vec()];

        tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect("localhost".try_into().unwrap(), tcp)
            .await
    }

    // Returns when the listener has closed its side, so the connection no longer holds a place.
    async fn close(mut connection: TlsStream<TcpStream>) {
        connection.shutdown().await.unwrap();
        connection.read_to_end(&mut Vec::new()).await.unwrap();
    }

    #[tokio::test]
    async fn serve_ptp1_is_not_answered_by_the_web_half() {
        let running = start_listener(
            "listener-ptp1",
            ROOMY_LIMITS,
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;

        let handshake = connect_tls_with_alpn(running.address, b"ptp1").await;

        let error = handshake.err().map(|error| error.to_string());
        assert!(
            !error
                .as_deref()
                .unwrap_or_default()
                .contains("NoApplicationProtocol"),
            "answered by the web half: {error:?}"
        );
        running.stop().await;
    }

    #[tokio::test]
    async fn serve_connection_past_the_total_limit_is_refused() {
        let running = start_listener(
            "listener-ceiling",
            Limits {
                web_connections: 2,
                ..ROOMY_LIMITS
            },
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;
        let _connections = [
            connect_tls(running.address).await.unwrap(),
            connect_tls(running.address).await.unwrap(),
        ];

        let third_connection = connect_tls(running.address).await;

        assert!(third_connection.is_err(), "{third_connection:?}");
        running.stop().await;
    }

    #[tokio::test]
    async fn serve_closed_connection_frees_its_place_in_the_total_limit() {
        let running = start_listener(
            "listener-ceiling-freed",
            Limits {
                web_connections: 1,
                ..ROOMY_LIMITS
            },
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;
        close(connect_tls(running.address).await.unwrap()).await;

        let next_connection = connect_tls(running.address).await;

        assert!(next_connection.is_ok(), "{next_connection:?}");
        running.stop().await;
    }

    #[tokio::test]
    async fn serve_connection_past_the_source_limit_is_refused() {
        let running = start_listener(
            "listener-share",
            Limits {
                web_connections_per_source: 1,
                ..ROOMY_LIMITS
            },
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;
        let _first_connection = connect_tls(running.address).await.unwrap();

        let second_connection = connect_tls(running.address).await;

        assert!(second_connection.is_err(), "{second_connection:?}");
        running.stop().await;
    }

    #[tokio::test]
    async fn serve_closed_connection_frees_its_source_share() {
        let running = start_listener(
            "listener-share-freed",
            Limits {
                web_connections_per_source: 1,
                ..ROOMY_LIMITS
            },
            L4Proxies::default(),
            L7Proxies::default(),
        )
        .await;
        close(connect_tls(running.address).await.unwrap()).await;

        let next_connection = connect_tls(running.address).await;

        assert!(next_connection.is_ok(), "{next_connection:?}");
        running.stop().await;
    }
}
