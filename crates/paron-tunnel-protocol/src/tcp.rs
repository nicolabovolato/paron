//! The TLS-over-TCP transport mapping (RFC Appendix B).
//!
//! TLS 1.3 over TCP, ALPN "ptp1", multiplexed by yamux: the agent is the yamux client, the
//! broker the yamux server. The control stream is the first agent-opened stream; tunnel
//! streams are broker-opened. The transport supplies no address validation, so the occupancy
//! bounds of TRN-10 carry the whole of the per-source requirement.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{FuturesAsyncReadCompatExt as _, TokioAsyncReadCompatExt as _};
use tokio_util::sync::CancellationToken;

use crate::broker::Broker;
use crate::broker::serve_connection;
use crate::error::TunnelError;
use crate::identity::{Identity, PublicKey};
use crate::tls;
use crate::transport::{
    AgentConnection, AgentTransport, BrokerAddress, BrokerConnection, BrokerTransport, Closer,
    IncomingStreams, Leased, PinnedBroker, Serving, Stream, StreamOpener, TUNNEL_STREAMS,
};

/// How long a TCP peer gets to finish the TLS handshake before the socket is dropped. Below
/// the protocol entirely: a peer that has not finished a handshake has not begun a connection
/// in the sense TRN-10 counts.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Appendix B, agent side.
#[derive(Debug, Default)]
pub struct TcpAgentTransport;

#[async_trait]
impl AgentTransport for TcpAgentTransport {
    async fn dial(
        &self,
        broker: &PinnedBroker,
        address: SocketAddr,
        identity: &Identity,
    ) -> Result<AgentConnection, TunnelError> {
        let connector = tokio_rustls::TlsConnector::from(tls::agent_config(
            identity,
            &broker.root,
            Arc::clone(&broker.rollback),
        )?);
        let tls = connect(address, &broker.address, &connector).await?;

        if tls.get_ref().1.alpn_protocol() != Some(tls::ALPN) {
            return Err(TunnelError::fault("the peer did not negotiate ptp1"));
        }

        let connection =
            yamux::Connection::new(tls.compat(), session_config(), yamux::Mode::Client);
        let (session, inbound) = YamuxSession::start(connection);

        // The control stream is the first agent-opened stream (Appendix B).
        let control = session.request_stream().await?;

        Ok(AgentConnection::new(
            Box::new(control.compat()),
            Box::new(Inbound { inbound }),
            Box::new(session),
        ))
    }
}

/// One TCP connection and TLS handshake with the broker at `address`.
async fn connect(
    address: SocketAddr,
    broker_address: &BrokerAddress,
    connector: &tokio_rustls::TlsConnector,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, TunnelError> {
    let attempt = async {
        let tcp = TcpStream::connect(address)
            .await
            .map_err(TunnelError::unexpected)?;
        tcp.set_nodelay(true).map_err(TunnelError::unexpected)?;

        connector
            .connect(broker_address.server_name(), tcp)
            .await
            .map_err(TunnelError::unexpected)
    };

    tokio::time::timeout(HANDSHAKE_TIMEOUT, attempt)
        .await
        .map_err(TunnelError::unexpected)?
}

/// Appendix B, broker side.
#[derive(Debug, Default)]
pub struct TcpBrokerTransport;

#[async_trait]
impl BrokerTransport for TcpBrokerTransport {
    async fn bind(&self, address: SocketAddr, broker: Arc<Broker>) -> Result<Serving, TunnelError> {
        let listener = TcpListener::bind(address)
            .await
            .map_err(TunnelError::unexpected)?;
        let bound = listener.local_addr().map_err(TunnelError::unexpected)?;
        let port = TcpBroker::new(broker)?;

        let stopping = CancellationToken::new();

        Ok(Serving::new(
            bound,
            stopping.clone(),
            accept(listener, port, stopping),
        ))
    }
}

/// Whether a ClientHello is asking for this mapping (TRN-12).
///
/// The question a sharer has to answer, answered here so that it is asked the same way
/// everywhere: a connection that does not offer `ptp1` is not a PTP connection, and one that
/// does is not to be handed anything else — the two configurations differ in whether a client
/// certificate is required at all, and picking the wrong one silently drops TRN-3's proof.
pub fn is_ptp1(hello: &rustls::server::ClientHello<'_>) -> bool {
    hello
        .alpn()
        .is_some_and(|mut offered| offered.any(|protocol| protocol == tls::ALPN))
}

/// Appendix B on a port this mapping does not own.
///
/// A broker that shares its port answers the ClientHello itself — the mapping cannot, because
/// deciding whether a connection is PTP/1 at all is what the ClientHello is being read for — and
/// hands over the ones that asked for [`tls::ALPN`]. Everything from the handshake on is the
/// mapping's, exactly as it is behind [`listen`](BrokerTransport::listen): the same TLS
/// configuration, the same client certificate requirement (TRN-3), the same bound.
///
/// What the sharer owes in return is a bound on the part it kept. Accepting a connection and
/// reading its ClientHello happens before anyone knows whose connection it is, so TRN-10's bound
/// here cannot cover it — a peer that opens connections and dribbles ClientHellos into them is
/// the sharer's to refuse, and is not seen here at all.
pub struct TcpBroker {
    broker: Arc<Broker>,

    /// Built once. Building it parses a certificate and a private key, which is not a thing to
    /// do per connection.
    config: Arc<rustls::ServerConfig>,
}

impl std::fmt::Debug for TcpBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TcpBroker").finish_non_exhaustive()
    }
}

impl TcpBroker {
    pub fn new(broker: Arc<Broker>) -> Result<Self, TunnelError> {
        Ok(Self {
            config: tls::broker_config(broker.keys())?,
            broker,
        })
    }

    /// Serve one connection whose ClientHello asked for PTP/1.
    ///
    /// The permit is taken before the handshake is completed, not after, so TRN-10 bounds the
    /// same span it does behind a listener of this mapping's own. A connection turned away here
    /// is dropped where it stands: the peer sees a TCP close with no TLS answer, which is what a
    /// full listener would have given it.
    pub async fn serve(
        &self,
        start: tokio_rustls::server::StartHandshake<TcpStream>,
        agent_address: SocketAddr,
        stopping: &CancellationToken,
    ) {
        // TCP supplies no address validation of its own (Appendix B), so the occupancy bound
        // carries the whole of TRN-10's per-source requirement here. Taken before the handshake
        // is completed, not after, so it bounds the same span a listener of this mapping's own
        // would. A connection turned away is dropped where it stands: the peer sees a TCP close
        // with no TLS answer, which is what a full listener would have given it.
        let Some(slot) = self.broker.unadmitted().take_slot(agent_address.ip()) else {
            tracing::debug!(%agent_address, "agent connection refused: too many in flight");

            return;
        };

        let Ok(Ok(tls)) = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            start.into_stream(Arc::clone(&self.config)),
        )
        .await
        else {
            return;
        };

        serve_accepted(tls, agent_address, Arc::clone(&self.broker), slot, stopping).await;
    }
}

async fn accept(listener: TcpListener, port: TcpBroker, stopping: CancellationToken) {
    let port = Arc::new(port);
    let sessions = tokio_util::task::TaskTracker::new();

    loop {
        let Some(accepted) = stopping.run_until_cancelled(listener.accept()).await else {
            break;
        };

        let Ok((tcp, agent_address)) = accepted else {
            continue;
        };
        let _ = tcp.set_nodelay(true);

        let port = Arc::clone(&port);
        let stopping = stopping.clone();

        sessions.spawn(async move {
            // Owning the port, this is the whole of the ClientHello bound: nothing else is
            // competing for the accept loop, and a connection that never sends one is holding a
            // task and a socket and no permit. The handshake timeout below covers it, because
            // reading the ClientHello is part of the handshake here.
            let acceptor =
                tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp);

            let Ok(Ok(start)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor).await else {
                return;
            };

            port.serve(start, agent_address, &stopping).await;
        });
    }

    sessions.close();
    sessions.wait().await;
}

/// Everything after the handshake, which is the same whether the port is this mapping's or
/// shared: one implementation, so a change to admission cannot reach one path and miss the other.
///
/// `slot` is TRN-10's pre-admission place, already taken. It travels with the connection and is
/// given back at admission, inside [`serve_connection`].
async fn serve_accepted(
    tls: tokio_rustls::server::TlsStream<TcpStream>,
    agent_address: SocketAddr,
    broker: Arc<Broker>,
    slot: crate::broker::Slot,
    stopping: &CancellationToken,
) {
    // Asked again rather than trusted from the ClientHello: what a peer offered is not what was
    // negotiated, and a connection that did not negotiate ptp1 is not a PTP connection.
    if tls.get_ref().1.alpn_protocol() != Some(tls::ALPN) {
        return;
    }

    // TRN-3: the client certificate the handshake required and proved.
    let Some(agent_public_key) = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .and_then(|leaf| PublicKey::of_certificate(leaf).ok())
    else {
        return;
    };

    let connection = yamux::Connection::new(tls.compat(), session_config(), yamux::Mode::Server);
    let (session, mut inbound) = YamuxSession::start(connection);

    // The control stream is the first agent-opened stream; waiting for it is bounded like
    // everything pre-admission (TRN-10).
    let Ok(Some(control)) =
        tokio::time::timeout(broker.limits().admission_timeout, inbound.recv()).await
    else {
        session.close().await;

        return;
    };

    let session = Arc::new(session);

    // TRN-9: the agent gets the control stream and nothing further; a second agent-opened
    // stream ends the connection.
    tokio::spawn({
        let session = Arc::clone(&session);

        async move {
            if inbound.recv().await.is_some() {
                tracing::debug!("agent connection closed: too many streams");
                session.close().await;
            }
        }
    });

    let closing = Arc::clone(&session);

    let mut serving = std::pin::pin!(serve_connection(
        broker,
        BrokerConnection::new(
            agent_address,
            agent_public_key,
            Box::new(control.compat()),
            Arc::clone(&session) as Arc<dyn StreamOpener>,
            session as Arc<dyn Closer>,
        ),
        slot,
    ));

    // Closed rather than dropped: dropping the session mid-frame is a cut wire, so the transport
    // is closed first and the connection is then awaited, to end on its own terms.
    tokio::select! {
        () = &mut serving => {}
        () = stopping.cancelled() => {
            closing.close().await;

            serving.await;
        }
    }
}

fn session_config() -> yamux::Config {
    let mut config = yamux::Config::default();
    config.set_max_num_streams(SESSION_STREAMS);
    // Pinned rather than inherited: yamux defaults to a gigabyte per connection and autotunes
    // each stream's window upward toward it, which its own documentation names as a way to make
    // a peer hold that gigabyte. This is the floor yamux allows for this many streams, so every
    // stream keeps its initial credit and none grows past it (TRN-10).
    config.set_max_connection_receive_window(Some(SESSION_STREAMS * STREAM_WINDOW));

    config
}

/// What one stream may have in flight toward this side, and so what a stream costs in buffers.
/// yamux's own per-stream credit, which is what a session starts every stream at.
const STREAM_WINDOW: usize = yamux::DEFAULT_CREDIT as usize;

/// yamux counts every stream in the session against one budget, in both directions, so the
/// session's limit is the tunnel streams plus the control stream.
const SESSION_STREAMS: usize = TUNNEL_STREAMS + 1;

/// The session already carries as many streams as it may. It is a refusal, not the connection
/// going away, and STM-8 wants those two told apart.
struct AtStreamLimit;

type OpenedStream = Result<yamux::Stream, AtStreamLimit>;

/// How long to spend shutting a session down cleanly before giving up and dropping it where it
/// stands. A peer that has stopped reading must not be able to keep the task alive. Appendix B
/// asks for the clean shutdown but sets no time limit on it, so this is ours.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// A yamux session: a handle onto the one task that owns the connection.
///
/// yamux is poll-based and makes progress only while polled, so every open and every inbound
/// stream funnels through that task.
struct YamuxSession {
    /// Requests for a new outbound stream, answered by the task.
    opens: mpsc::Sender<oneshot::Sender<OpenedStream>>,
    /// One permit per tunnel stream this session may carry, so an open past the limit is refused
    /// here rather than blowing up the session (TRN-6).
    stream_slots: Arc<tokio::sync::Semaphore>,
    /// Asks the task to shut the session down cleanly. Whoever asks first takes the sender; the
    /// `None` it leaves behind is how [`Drop`] knows the task is already winding down and must not
    /// be killed mid-shutdown.
    shutdown: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    task: tokio::task::AbortHandle,
}

impl YamuxSession {
    fn start<T>(connection: yamux::Connection<T>) -> (Self, mpsc::Receiver<yamux::Stream>)
    where
        T: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send + 'static,
    {
        let (inbound_tx, inbound_rx) = mpsc::channel(64);
        let (opens_tx, opens_rx) = mpsc::channel(16);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task =
            tokio::spawn(run_session(connection, inbound_tx, opens_rx, shutdown_rx)).abort_handle();

        (
            Self {
                opens: opens_tx,
                stream_slots: Arc::new(tokio::sync::Semaphore::new(TUNNEL_STREAMS)),
                shutdown: parking_lot::Mutex::new(Some(shutdown_tx)),
                task,
            },
            inbound_rx,
        )
    }

    /// Ask the task that owns the session to open a stream, and wait for what it opened.
    async fn request_stream(&self) -> Result<yamux::Stream, TunnelError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.opens
            .send(reply_tx)
            .await
            .map_err(|_| TunnelError::NotConnected)?;

        reply_rx
            .await
            // The reply was dropped, so the session is gone.
            .map_err(|_| TunnelError::NotConnected)?
            // The agent is still there, it just has no room for another stream.
            .map_err(|AtStreamLimit| TunnelError::Refused)
    }
}

impl Drop for YamuxSession {
    fn drop(&mut self) {
        // Only where nobody asked for a goodbye: a session that is saying one is finishing under
        // its own bound, and aborting it here would cut off the `GoAway` it exists to send.
        if self.shutdown.lock().is_some() {
            self.task.abort();
        }
    }
}

#[async_trait]
impl StreamOpener for YamuxSession {
    async fn open(&self) -> Result<Box<dyn Stream>, TunnelError> {
        let slot = Arc::clone(&self.stream_slots)
            .try_acquire_owned()
            .map_err(|_| TunnelError::Refused)?;
        let stream = self.request_stream().await?;

        Ok(Box::new(Leased::new(stream.compat(), slot)))
    }
}

#[async_trait]
impl Closer for YamuxSession {
    /// Appendix B: a deliberate end is a yamux `GoAway` followed by TLS `close_notify`. Both are
    /// the task's to send, so this asks and returns — a broker replacing a registration (STM-11)
    /// is not made to wait on the connection it displaced.
    async fn close(&self) {
        let shutdown = self.shutdown.lock().take();

        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }
    }
}

/// The one task that polls a session: it opens what [`YamuxSession::request_stream`] asks for, hands
/// every inbound stream to whoever is receiving them, and says the goodbye Appendix B asks for
/// when the session is ended deliberately.
async fn run_session<T>(
    mut connection: yamux::Connection<T>,
    inbound: mpsc::Sender<yamux::Stream>,
    mut opens: mpsc::Receiver<oneshot::Sender<OpenedStream>>,
    mut shutdown: oneshot::Receiver<()>,
) where
    T: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send + 'static,
{
    let mut pending: VecDeque<oneshot::Sender<OpenedStream>> = VecDeque::new();

    let asked = std::future::poll_fn(|cx| {
        // Whether it was sent or the last handle dropped, there is nobody left to serve.
        if std::pin::Pin::new(&mut shutdown).poll(cx).is_ready() {
            return Poll::Ready(true);
        }

        while let Poll::Ready(request) = opens.poll_recv(cx) {
            match request {
                Some(reply) => pending.push_back(reply),
                // Openers gone; inbound streams may still matter, so the session lives on.
                None => break,
            }
        }

        while !pending.is_empty() {
            match connection.poll_new_outbound(cx) {
                Poll::Ready(Ok(stream)) => {
                    if let Some(reply) = pending.pop_front() {
                        let _ = reply.send(Ok(stream));
                    }
                }
                Poll::Ready(Err(error)) => {
                    // Every waiting open is refused either way; what differs is whether the
                    // session survives. A stream ceiling is a refusal and nothing more — the
                    // agent is still there and its other streams still relay — so ending the
                    // session here would let one consumer holding every stream take an agent
                    // off the network until it redialled (CNF-21).
                    let ceiling = matches!(error, yamux::ConnectionError::TooManyStreams);

                    for reply in pending.drain(..) {
                        let _ = reply.send(Err(AtStreamLimit));
                    }

                    if ceiling {
                        break;
                    }

                    return Poll::Ready(false);
                }
                Poll::Pending => break,
            }
        }

        loop {
            match connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(stream))) => {
                    // A receiver that stalls sheds streams rather than the whole session; the
                    // agent side takes them all, and the broker side ends the connection on
                    // the second anyway.
                    let _ = inbound.try_send(stream);
                }
                Poll::Ready(Some(Err(_)) | None) => return Poll::Ready(false),
                Poll::Pending => return Poll::Pending,
            }
        }
    })
    .await;

    // Appendix B: a deliberate end is a yamux GoAway, and the close_notify that closing the
    // stream underneath it sends. A session that ended on its own has nothing left to say it
    // with, and a peer that has stopped reading must not hold this task open.
    if asked {
        let closing = std::future::poll_fn(|cx| connection.poll_close(cx));

        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, closing).await;
    }
}

struct Inbound {
    inbound: mpsc::Receiver<yamux::Stream>,
}

#[async_trait]
impl IncomingStreams for Inbound {
    async fn next(&mut self) -> Option<Box<dyn Stream>> {
        let stream = self.inbound.recv().await?;

        Some(Box::new(stream.compat()))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use tokio::io::DuplexStream;
    use tokio_rustls::StartHandshake;

    use super::*;
    use crate::identity::InMemoryRollbackIndex;

    // The handshake never completes. The ALPN offer is in the ClientHello, which is enough.
    async fn client_hello_offering(alpn: Vec<Vec<u8>>) -> StartHandshake<DuplexStream> {
        let (client, server) = tokio::io::duplex(4096);
        let broker = Identity::generate().unwrap();
        let agent = Identity::generate().unwrap();
        let mut config = (*tls::agent_config(
            &agent,
            &broker.public_key().fingerprint(),
            Arc::new(InMemoryRollbackIndex::default()),
        )
        .unwrap())
        .clone();
        config.alpn_protocols = alpn;

        tokio::spawn(async move {
            let _ = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(tls::server_name("localhost").unwrap(), client)
                .await;
        });

        tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), server)
            .await
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn dial_peer_that_stays_silent_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _silent_peer = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let broker = Identity::generate().unwrap();
        let pinned_broker = PinnedBroker {
            address: address.to_string().parse().unwrap(),
            root: broker.public_key().fingerprint(),
            rollback: Arc::new(InMemoryRollbackIndex::default()),
        };

        let dialed = TcpAgentTransport
            .dial(&pinned_broker, address, &Identity::generate().unwrap())
            .await;

        assert_matches!(
            dialed.err(),
            Some(TunnelError::Unexpected(source))
                if source.is::<tokio::time::error::Elapsed>()
        );
    }

    #[tokio::test]
    async fn is_ptp1_with_only_ptp1_offered_returns_true() {
        let handshake = client_hello_offering(vec![tls::ALPN.to_vec()]).await;

        let ptp1 = is_ptp1(&handshake.client_hello());

        assert!(ptp1);
    }

    #[tokio::test]
    async fn is_ptp1_with_ptp1_among_others_returns_true() {
        let handshake = client_hello_offering(vec![b"h2".to_vec(), tls::ALPN.to_vec()]).await;

        let ptp1 = is_ptp1(&handshake.client_hello());

        assert!(ptp1);
    }

    #[tokio::test]
    async fn is_ptp1_with_other_protocols_only_returns_false() {
        let handshake = client_hello_offering(vec![b"h2".to_vec(), b"http/1.1".to_vec()]).await;

        let ptp1 = is_ptp1(&handshake.client_hello());

        assert!(!ptp1);
    }

    #[tokio::test]
    async fn is_ptp1_without_alpn_returns_false() {
        let handshake = client_hello_offering(Vec::new()).await;

        let ptp1 = is_ptp1(&handshake.client_hello());

        assert!(!ptp1);
    }
}
