//! The QUIC transport mapping (RFC Appendix A).
//!
//! QUIC version 1 with TLS 1.3, ALPN "ptp1". The control stream is the first client-initiated
//! bidirectional stream; tunnel streams are server-initiated bidirectional streams. The broker
//! permits the agent one bidirectional stream and no unidirectional streams, the agent permits
//! the broker a bounded number, and the DATAGRAM extension is never negotiated (TRN-9,
//! §10.3). Address validation is QUIC Retry, used while the unadmitted-connection bound is
//! approached (TRN-10).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::broker::{Broker, DENIAL_DELAY, serve_connection};
use crate::error::TunnelError;
use crate::identity::{Identity, PublicKey};
use crate::tls;
use crate::transport::{
    AgentConnection, AgentTransport, BrokerConnection, BrokerTransport, Closer, IncomingStreams,
    Leased, PinnedBroker, Serving, Stream, StreamOpener, TUNNEL_STREAMS,
};

/// How long one connection attempt gets before the agent gives up on it.
///
/// Not an idle timeout, which TRN-8 holds at or above the role's detection bound: a handshake
/// is not a connection the detection bound speaks of.
///
/// It has to end the attempt with an error rather than wait. A port that refuses answers with
/// ICMP and fails at once, but a firewall that drops answers with nothing, and QUIC retransmits
/// its Initial for as long as it is allowed to. Without a bound the attempt never ends, so a race
/// in which no attempt can succeed would never fail and never reach STM-14's backoff.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long QUIC itself will tolerate silence on a connection.
///
/// This is a backstop, not the liveness mechanism — that is the protocol's own PING frames
/// (§6.2, TRN-8). What it catches is a peer that completes a handshake and then says nothing at
/// all, which the protocol cannot notice because the protocol has not started yet.
///
/// It has to sit above the two silences a healthy connection is allowed. The first is the gap
/// between probes, which is why the detection bound and one more probe interval go in. The second
/// is a connection waiting on a denial: TRN-10 makes a broker sit on an `ACCESS`(Denied) for a
/// second before sending it, and during that second the connection is silent through no fault of
/// its own. Twice the delay covers that second plus however long the policy took to produce the
/// answer, without this function having to know what a policy costs.
fn idle_timeout(broker: &Broker) -> Duration {
    let quiet_between_probes =
        broker.liveness().detection_bound() + broker.liveness().probe_interval();

    quiet_between_probes.max(DENIAL_DELAY * 2)
}

/// The one capability the broker exercises, as a QUIC stream limit (TRN-9).
pub(crate) fn broker_streams() -> u32 {
    u32::try_from(TUNNEL_STREAMS).unwrap_or(u32::MAX)
}

/// What one stream may have in flight toward this side, and so what a stream costs in buffers.
/// The same credit the TCP mapping starts its streams at, so a connection costs the same either
/// way (TRN-10).
const STREAM_WINDOW: u32 = 256 * 1024;

pub(crate) fn transport_config(
    peer_bidi_streams: u32,
    idle_timeout: Option<Duration>,
) -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();

    transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(peer_bidi_streams));
    transport.max_concurrent_uni_streams(quinn::VarInt::from_u32(0));
    // Pinned rather than inherited: quinn defaults to 1.25MB a stream and no connection bound at
    // all, so what a peer may make this side hold would move with the dependency. The connection
    // window is what the streams it may open add up to, which is the most it could ever have in
    // flight and so bounds nothing a conforming peer does.
    transport.stream_receive_window(quinn::VarInt::from_u32(STREAM_WINDOW));
    transport.receive_window(
        quinn::VarInt::from_u64(u64::from(peer_bidi_streams) * u64::from(STREAM_WINDOW))
            .unwrap_or(quinn::VarInt::MAX),
    );
    // TRN-9, §10.3: the DATAGRAM extension is not negotiated.
    transport.datagram_receive_buffer_size(None);
    // TRN-8: liveness is the protocol's own PING frames, so this is a backstop and never below
    // the detection bound, which TRN-8 forbids.
    transport
        .max_idle_timeout(idle_timeout.map(|d| d.try_into().unwrap_or(quinn::VarInt::MAX.into())));

    Arc::new(transport)
}

/// Appendix A, agent side.
#[derive(Debug, Default)]
pub struct QuicAgentTransport;

#[async_trait]
impl AgentTransport for QuicAgentTransport {
    async fn dial(
        &self,
        broker: &PinnedBroker,
        address: SocketAddr,
        identity: &Identity,
    ) -> Result<AgentConnection, TunnelError> {
        let crypto = QuicClientConfig::try_from(tls::agent_config(
            identity,
            &broker.root,
            Arc::clone(&broker.rollback),
        )?)
        .map_err(TunnelError::unexpected)?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        // The agent reaps a dead broker through its own probe loop; no transport idle timeout is
        // needed on this side.
        config.transport_config(transport_config(broker_streams(), None));

        let (endpoint, connection) = connect(address, broker.address.host(), config).await?;

        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(TunnelError::unexpected)?;

        Ok(AgentConnection::new(
            Box::new(BiStream { send, recv }),
            Box::new(Inbound {
                connection: connection.clone(),
            }),
            Box::new(CloseConnection {
                connection,
                endpoint: Some(endpoint),
            }),
        ))
    }
}

/// Appendix A, broker side.
#[derive(Debug, Default)]
pub struct QuicBrokerTransport;

#[async_trait]
impl BrokerTransport for QuicBrokerTransport {
    async fn bind(&self, address: SocketAddr, broker: Arc<Broker>) -> Result<Serving, TunnelError> {
        let crypto = QuicServerConfig::try_from(tls::broker_config(broker.keys())?)
            .map_err(TunnelError::unexpected)?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        // The agent gets the control stream and nothing further (TRN-9). The idle timeout sits
        // above the detection bound so a peer that completes the handshake and never speaks is
        // reaped (TRN-10, and see `transport_config`).
        config.transport_config(transport_config(1, Some(idle_timeout(&broker))));

        let endpoint = quinn::Endpoint::server(config, address).map_err(TunnelError::unexpected)?;
        let bound = endpoint.local_addr().map_err(TunnelError::unexpected)?;

        let stopping = tokio_util::sync::CancellationToken::new();

        Ok(Serving::new(
            bound,
            stopping.clone(),
            accept(endpoint, broker, stopping),
        ))
    }
}

async fn accept(
    endpoint: quinn::Endpoint,
    broker: Arc<Broker>,
    stopping: tokio_util::sync::CancellationToken,
) {
    let sessions = tokio_util::task::TaskTracker::new();

    while let Some(Some(incoming)) = stopping.run_until_cancelled(endpoint.accept()).await {
        // TRN-10: Retry is the address validation this transport supplies, used while the
        // unadmitted-connection bound is approached.
        if broker.unadmitted().past_half_capacity() && !incoming.remote_address_validated() {
            let _ = incoming.retry();

            continue;
        }

        let Some(slot) = broker
            .unadmitted()
            .take_slot(incoming.remote_address().ip())
        else {
            incoming.refuse();

            continue;
        };
        let broker = Arc::clone(&broker);
        let stopping = stopping.clone();

        sessions.spawn(async move {
            let Ok(connecting) = incoming.accept() else {
                return;
            };
            let Ok(connection) = connecting.await else {
                return;
            };
            let Some(agent_public_key) = agent_key(&connection) else {
                // TRN-3 requires a client certificate, so a handshake without one has already
                // failed: reaching here is a transport bug rather than a peer's doing.
                close(&connection);

                return;
            };
            let agent_address = connection.remote_address();

            // The control stream is the first client-initiated bidirectional stream. The wait
            // is bounded so a peer that completes the handshake and opens nothing cannot park
            // this task (TRN-10).
            let opened =
                tokio::time::timeout(broker.limits().admission_timeout, connection.accept_bi())
                    .await;
            let Ok(Ok((send, recv))) = opened else {
                close(&connection);

                return;
            };

            let closing = connection.clone();

            let mut serving = std::pin::pin!(serve_connection(
                broker,
                // `slot` goes with it: `serve_connection` gives it back at admission (TRN-10).
                BrokerConnection::new(
                    agent_address,
                    agent_public_key,
                    Box::new(BiStream { send, recv }),
                    Arc::new(Opener::over(connection.clone())),
                    Arc::new(CloseConnection {
                        connection,
                        endpoint: None,
                    }),
                ),
                slot,
            ));

            // Closed rather than dropped: the peer is told, and the session is then awaited so it
            // ends on its own terms.
            tokio::select! {
                () = &mut serving => {}
                () = stopping.cancelled() => {
                    close(&closing);

                    serving.await;
                }
            }
        });
    }

    sessions.close();
    sessions.wait().await;
}

/// One QUIC handshake with the broker at `address`. The endpoint comes back with the connection
/// because it owns the socket underneath it.
///
/// One endpoint per attempt: the socket a QUIC endpoint binds is of one address family, so
/// reaching an IPv6 address and an IPv4 one takes two sockets.
async fn connect(
    address: SocketAddr,
    host: &str,
    config: quinn::ClientConfig,
) -> Result<(quinn::Endpoint, quinn::Connection), TunnelError> {
    let unspecified: SocketAddr = match address {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };

    let attempt = async {
        let mut endpoint = quinn::Endpoint::client(unspecified).map_err(TunnelError::unexpected)?;
        endpoint.set_default_client_config(config);

        let connection = endpoint
            .connect(address, host)
            .map_err(TunnelError::unexpected)?
            .await
            .map_err(TunnelError::unexpected)?;

        Ok((endpoint, connection))
    };

    tokio::time::timeout(HANDSHAKE_TIMEOUT, attempt)
        .await
        .map_err(TunnelError::unexpected)?
}

/// Close a connection deliberately, which Appendix A says is an application `CONNECTION_CLOSE`.
fn close(connection: &quinn::Connection) {
    connection.close(quinn::VarInt::from_u32(0), b"");
}

/// The key the agent proved possession of in the handshake (TRN-3).
fn agent_key(connection: &quinn::Connection) -> Option<PublicKey> {
    let identity = connection.peer_identity()?;
    let certificates = identity
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok()?;
    let leaf = certificates.first()?;

    PublicKey::of_certificate(leaf).ok()
}

/// One QUIC bidirectional stream as one byte pipe.
struct BiStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl AsyncRead for BiStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for BiStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut self.send), cx)
    }
}

/// The agent's accept side: the broker's tunnel streams, as QUIC delivers them.
struct Inbound {
    connection: quinn::Connection,
}

#[async_trait]
impl IncomingStreams for Inbound {
    async fn next(&mut self) -> Option<Box<dyn Stream>> {
        let (send, recv) = self.connection.accept_bi().await.ok()?;

        Some(Box::new(BiStream { send, recv }))
    }
}

/// The broker's open side, counting the tunnel streams the agent admits.
///
/// Counted here rather than left to QUIC because an open past the agent's limit would wait for a
/// place another consumer may never give up, and §6.1 answers a consumer instead (TRN-6).
struct Opener {
    connection: quinn::Connection,
    stream_slots: Arc<tokio::sync::Semaphore>,
}

impl Opener {
    fn over(connection: quinn::Connection) -> Self {
        Self {
            connection,
            stream_slots: Arc::new(tokio::sync::Semaphore::new(TUNNEL_STREAMS)),
        }
    }
}

#[async_trait]
impl StreamOpener for Opener {
    async fn open(&self) -> Result<Box<dyn Stream>, TunnelError> {
        let slot = Arc::clone(&self.stream_slots)
            .try_acquire_owned()
            .map_err(|_| TunnelError::Refused)?;

        let (send, recv) = self
            .connection
            .open_bi()
            .await
            .map_err(TunnelError::unexpected)?;

        Ok(Box::new(Leased::new(BiStream { send, recv }, slot)))
    }
}

/// How long to wait for a deliberate close to reach the peer before letting go of the socket that
/// carries it. Appendix A asks for the close but sets no time limit, so this is ours.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Ends the connection (STM-5, Appendix A: an application `CONNECTION_CLOSE`).
///
/// An agent holds its endpoint here, because the endpoint is the socket the close frame goes out
/// on: dropping it the moment `close` returns would end the connection with nothing sent. A
/// broker's endpoint belongs to its listener and outlives every connection on it, so there is
/// nothing for a broker to hold or to wait for.
struct CloseConnection {
    connection: quinn::Connection,
    endpoint: Option<quinn::Endpoint>,
}

#[async_trait]
impl Closer for CloseConnection {
    async fn close(&self) {
        close(&self.connection);

        if let Some(endpoint) = &self.endpoint {
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, endpoint.wait_idle()).await;
        }
    }
}
