//! What the members of §11 are run against: a broker with records in a known state, agents that
//! reach it, echo services to reach through it, and a relay that can be cut.
//!
//! None of it is normative. It is here so `mod.rs` is the twenty-one members and nothing else.

use std::future::IntoFuture as _;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio_util::task::AbortOnDropHandle;

use super::records::{Counting, Records};
use super::{Mapping, Suite};
use crate::quic::QuicBrokerTransport;
use crate::tcp::TcpBrokerTransport;
use crate::transport::AgentConnection;
use crate::wire::{self, Access, Claim, Frame};
use crate::{
    Agent, AgentId, AgentIdentity, Broker, BrokerAddress, BrokerIdentity, ConnectedAgents,
    Identity, InMemoryRollbackIndex, Limits, Liveness, PinnedBroker, PublicKey, Selector,
    Selectors, Sequence, ServedSelectors,
};

/// K and M short enough that the timing members finish in seconds, which is what the conformance
/// mode of §6.2 exists for (TRN-8).
pub(super) fn short_probes() -> Liveness {
    Liveness::for_conformance(Duration::from_millis(150), 2)
}

/// Ports these agents serve are always ones just bound, so a port that is not a selector is a
/// fault in the test rather than an input to tolerate.
pub(super) fn tcp_ports(ports: impl IntoIterator<Item = u16>) -> Arc<Selectors> {
    Arc::new(Selectors::new(ports.into_iter().map(|port| {
        Selector::tcp(port)
            .expect("a bound port is a selector")
            .into()
    })))
}

/// How long to poll for an expected state before calling it absent.
pub(super) const TIMEOUT: Duration = Duration::from_secs(5);

impl Suite {
    pub(super) async fn broker(&self) -> TestBroker {
        let records = Records::new();
        let broker = Arc::new(Broker::new(
            Arc::clone(&records) as Arc<dyn crate::BrokerPolicy>,
            Arc::new(ConnectedAgents::new()),
            BrokerIdentity::generate().unwrap(),
            short_probes(),
            Limits::default(),
        ));
        let serving = match self.mapping {
            Mapping::Quic => {
                broker
                    .bind(&QuicBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
                    .await
            }
            Mapping::Tcp => {
                broker
                    .bind(&TcpBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
                    .await
            }
        }
        .expect("the broker must bind");
        let address = format!("127.0.0.1:{}", serving.address().port())
            .parse()
            .expect("a loopback address with a bound port");

        TestBroker {
            broker,
            records,
            address,
            _accepting: AbortOnDropHandle::new(tokio::spawn(serving.into_future())),
        }
    }

    pub(super) fn agent(
        &self,
        broker: &TestBroker,
        id: AgentId,
        target: Arc<dyn ServedSelectors>,
    ) -> (Arc<Agent>, PublicKey) {
        let identity = Identity::generate().unwrap();
        let public = identity.public_key();

        (
            Arc::new(Agent::new(
                vec![self.mapping.agent_transport()],
                PinnedBroker {
                    address: broker.address.clone(),
                    root: broker.broker.identity().fingerprint(),
                    rollback: Arc::new(InMemoryRollbackIndex::default()),
                },
                AgentIdentity { id, keys: identity },
                target,
                Counting::new(),
                short_probes(),
            )),
            public,
        )
    }

    /// A broker with one agent joined and serving `target`, settled to reachability.
    pub(super) async fn joined(&self, target: Arc<dyn ServedSelectors>) -> Joined {
        let broker = self.broker().await;

        let id = AgentId::new(uuid::Uuid::now_v7());
        let identity = Identity::generate().unwrap();
        let key_der = identity.private_key_der().clone();
        broker.records.joined(id, identity.public_key());

        let sequence = Counting::new();
        let agent = Arc::new(Agent::new(
            vec![self.mapping.agent_transport()],
            PinnedBroker {
                address: broker.address.clone(),
                root: broker.broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity { id, keys: identity },
            target,
            Arc::clone(&sequence) as Arc<dyn Sequence>,
            short_probes(),
        ));
        let serving = tokio::spawn({
            let agent = Arc::clone(&agent);

            async move {
                let _ = agent.serve(std::future::pending()).await;
            }
        });

        {
            let broker = Arc::clone(&broker.broker);

            wait_until(move || broker.connected_agents().tunnel_to(id).is_some()).await;
        }

        Joined {
            broker,
            id,
            key_der,
            sequence,
            serving,
            agent,
        }
    }

    /// A connection through the mapping's SPI, for members that must speak below the agent
    /// state machine: wrong versions, malformed frames, raw claims.
    pub(super) async fn raw_dial(
        &self,
        broker: &TestBroker,
        identity: &Identity,
    ) -> AgentConnection {
        let address = broker.address.resolve().await.unwrap()[0];

        self.mapping
            .agent_transport()
            .dial(
                &PinnedBroker {
                    address: broker.address.clone(),
                    root: broker.broker.identity().fingerprint(),
                    rollback: Arc::new(InMemoryRollbackIndex::default()),
                },
                address,
                identity,
            )
            .await
            .expect("the mapping must establish a connection")
    }

    /// One whole connection spent on one claim: dial, claim, read the verdict.
    pub(super) async fn claim_outcome(
        &self,
        broker: &TestBroker,
        identity: &Identity,
        id: AgentId,
        sequence: u64,
    ) -> Access {
        let mut connection = self.raw_dial(broker, identity).await;
        wire::write_version(&mut connection.control).await.unwrap();
        write_claim(&mut connection, id, sequence).await;

        match read_frame_within(&mut connection, TIMEOUT).await {
            Some(Frame::Access(access)) => access,
            other => panic!("a claim is answered by ACCESS, got {other:?}"),
        }
    }
}

pub(super) struct TestBroker {
    pub(super) broker: Arc<Broker>,
    pub(super) records: Arc<Records>,
    pub(super) address: BrokerAddress,
    _accepting: AbortOnDropHandle<()>,
}

impl TestBroker {
    pub(super) fn connected_agents(&self) -> &Arc<ConnectedAgents> {
        self.broker.connected_agents()
    }
}

pub(super) struct Joined {
    pub(super) broker: TestBroker,
    pub(super) id: AgentId,
    pub(super) key_der: secrecy::SecretSlice<u8>,
    pub(super) sequence: Arc<Counting>,
    pub(super) serving: tokio::task::JoinHandle<()>,
    pub(super) agent: Arc<Agent>,
}

impl Joined {
    pub(super) fn connected_agents(&self) -> &Arc<ConnectedAgents> {
        self.broker.connected_agents()
    }
}

impl Drop for Joined {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

pub(super) async fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;

    while !condition() {
        assert!(
            Instant::now() < deadline,
            "a state the test waited for never arrived"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn write_claim(connection: &mut AgentConnection, id: AgentId, sequence: u64) {
    wire::write_frame(
        &mut connection.control,
        &Frame::Claim(Claim::new(id, sequence).unwrap()),
    )
    .await
    .unwrap();
}

/// The next frame that is not a probe, or `None` once the connection has ended.
pub(super) async fn read_frame_within(
    connection: &mut AgentConnection,
    patience: Duration,
) -> Option<Frame> {
    let deadline = Instant::now() + patience;

    loop {
        let left = deadline.checked_duration_since(Instant::now())?;

        match tokio::time::timeout(left, wire::read_frame(&mut connection.control)).await {
            Err(_elapsed) => return None,
            Ok(Err(_)) => return None,
            Ok(Ok(Frame::Ping)) => {
                let _ = wire::write_frame(&mut connection.control, &Frame::Pong).await;
            }
            Ok(Ok(Frame::Pong)) => {}
            Ok(Ok(frame)) => return Some(frame),
        }
    }
}

/// Announces one selector and serves another, so that a broker treating the announced set as
/// permission, or as a promise, is distinguishable from a conforming one (§3.1).
pub(super) struct Disagreeing {
    pub(super) announced: Selector,
    pub(super) served: Selector,
}

impl ServedSelectors for Disagreeing {
    fn announced(&self) -> crate::AnnounceSet {
        crate::AnnounceSet::new([self.announced.clone().into()])
    }

    fn serves(&self, selector: &Selector) -> bool {
        *selector == self.served
    }
}

/// A local echo service: TCP, UDP or UNIX-domain, counting what it accepted.
pub(super) struct Echo {
    pub(super) port: u16,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) path: std::path::PathBuf,
    pub(super) accepted: Arc<AtomicUsize>,
    pub(super) _serving: tokio::task::JoinHandle<()>,
}

impl Echo {
    pub(super) async fn tcp() -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));

        let serving = tokio::spawn({
            let accepted = Arc::clone(&accepted);

            async move {
                loop {
                    let Ok((mut connection, _)) = listener.accept().await else {
                        return;
                    };
                    accepted.fetch_add(1, Ordering::Relaxed);

                    tokio::spawn(async move {
                        let (mut read, mut write) = connection.split();
                        let _ = tokio::io::copy(&mut read, &mut write).await;
                    });
                }
            }
        });

        Self {
            port,
            path: std::path::PathBuf::new(),
            accepted,
            _serving: serving,
        }
    }

    pub(super) async fn udp() -> Self {
        let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = socket.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));

        let serving = tokio::spawn(async move {
            let mut datagram = vec![0u8; usize::from(u16::MAX)];

            while let Ok((length, from)) = socket.recv_from(&mut datagram).await {
                let _ = socket.send_to(&datagram[..length], from).await;
            }
        });

        Self {
            port,
            path: std::path::PathBuf::new(),
            accepted,
            _serving: serving,
        }
    }

    #[cfg(unix)]
    pub(super) fn unix() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ptp-cnf18-{}-{}.sock",
            std::process::id(),
            uuid::Uuid::now_v7().simple()
        ));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));

        let serving = tokio::spawn({
            let accepted = Arc::clone(&accepted);

            async move {
                loop {
                    let Ok((mut connection, _)) = listener.accept().await else {
                        return;
                    };
                    accepted.fetch_add(1, Ordering::Relaxed);

                    tokio::spawn(async move {
                        let (mut read, mut write) = connection.split();
                        let _ = tokio::io::copy(&mut read, &mut write).await;
                    });
                }
            }
        });

        Self {
            port: 0,
            path,
            accepted,
            _serving: serving,
        }
    }

    pub(super) fn accepted(&self) -> usize {
        self.accepted.load(Ordering::Relaxed)
    }
}

/// A relay in front of the broker that can be severed without closing either side: no FIN, no
/// RST, no ICMP. What CNF-8 needs, in the packet flavour of the mapping under test.
pub(super) struct Relay {
    pub(super) address: BrokerAddress,
    pub(super) severed: tokio::sync::watch::Sender<bool>,
    pub(super) _forwarding: tokio::task::JoinHandle<()>,
}

fn relay_address(port: u16) -> BrokerAddress {
    format!("127.0.0.1:{port}")
        .parse()
        .expect("a loopback address with a bound port")
}

impl Relay {
    pub(super) async fn between(mapping: Mapping, broker: &str) -> Self {
        match mapping {
            Mapping::Tcp => Self::tcp(broker).await,
            Mapping::Quic => Self::udp(broker).await,
        }
    }

    pub(super) fn cut(&self) {
        let _ = self.severed.send(true);
    }

    async fn tcp(broker: &str) -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = relay_address(listener.local_addr().unwrap().port());
        let (severed, watch) = tokio::sync::watch::channel(false);
        let broker = broker.to_owned();

        let forwarding = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let Ok(upstream) = tokio::net::TcpStream::connect(&broker).await else {
                    return;
                };
                let mut watch = watch.clone();

                tokio::spawn(async move {
                    let (mut client_read, mut client_write) = client.into_split();
                    let (mut upstream_read, mut upstream_write) = upstream.into_split();

                    let cut = tokio::select! {
                        _ = tokio::io::copy(&mut client_read, &mut upstream_write) => false,
                        _ = tokio::io::copy(&mut upstream_read, &mut client_write) => false,
                        _ = watch.wait_for(|cut| *cut) => true,
                    };

                    if cut {
                        // Severed: both sockets are held open and silent until the test ends,
                        // which is what a host losing power looks like.
                        std::future::pending::<()>().await;
                    }
                });
            }
        });

        Self {
            address,
            severed,
            _forwarding: forwarding,
        }
    }

    async fn udp(broker: &str) -> Self {
        let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = relay_address(socket.local_addr().unwrap().port());
        let upstream: std::net::SocketAddr = broker.parse().unwrap();
        let (severed, watch) = tokio::sync::watch::channel(false);

        let forwarding = tokio::spawn(async move {
            let mut datagram = vec![0u8; usize::from(u16::MAX)];
            let mut client: Option<std::net::SocketAddr> = None;

            loop {
                let Ok((length, from)) = socket.recv_from(&mut datagram).await else {
                    return;
                };

                if *watch.borrow() {
                    // Severed: keep receiving so the socket stays open, forward nothing.
                    continue;
                }

                if from == upstream {
                    if let Some(client) = client {
                        let _ = socket.send_to(&datagram[..length], client).await;
                    }
                } else {
                    client = Some(from);
                    let _ = socket.send_to(&datagram[..length], upstream).await;
                }
            }
        });

        Self {
            address,
            severed,
            _forwarding: forwarding,
        }
    }
}
