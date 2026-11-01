//! What one tunnel costs, measured per operation (RFC §4.3, TRN-6).
//!
//! Both roles run in this process, so a figure here is the cost of a whole round trip through
//! the protocol rather than of one side; `benches/fleet.rs` separates the two and reports the
//! broker's own CPU and memory. The runtime is pinned to two worker threads so a wide machine
//! cannot hide a scheduling cost that a small broker would pay.
//!
//! `cargo bench -p paron-tunnel-protocol --bench tunnel`

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::duration_suboptimal_units,
    reason = "a measurement harness turns counters into rates for a human to read; the \
    precision a cast loses is far below what the numbers mean"
)]

#[path = "harness/mod.rs"]
mod harness;

use std::future::IntoFuture as _;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use harness::{Counting, GrantAll, agent_id, tcp_echo};
use paron_tunnel_protocol::quic::{QuicAgentTransport, QuicBrokerTransport};
use paron_tunnel_protocol::tcp::{TcpAgentTransport, TcpBrokerTransport};
use paron_tunnel_protocol::{
    Agent, AgentIdentity, AgentTransport, Broker, BrokerAddress, BrokerIdentity, ConnectedAgents,
    Identity, InMemoryRollbackIndex, Limits, Liveness, PinnedBroker, Selector, Selectors,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::task::AbortOnDropHandle;

/// Two workers, the shape of a small self-hosted broker.
const WORKERS: usize = 2;

/// Which of the two mappings of appendices A and B is under measurement.
#[derive(Clone, Copy)]
enum Mapping {
    Quic,
    Tcp,
}

impl Mapping {
    fn name(self) -> &'static str {
        match self {
            Self::Quic => "quic",
            Self::Tcp => "tcp",
        }
    }

    fn agent_transport(self) -> Box<dyn AgentTransport> {
        match self {
            Self::Quic => Box::new(QuicAgentTransport),
            Self::Tcp => Box::new(TcpAgentTransport),
        }
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .unwrap()
}

/// One agent, serving, with an echo behind it.
struct One {
    broker: Arc<Broker>,
    selector: Selector,
    _agent: Arc<Agent>,
    _accepting: AbortOnDropHandle<()>,
}

/// A UNIX-domain echo, for measurements that open and close without pause.
///
/// Not TCP: a loopback connection per open spends an ephemeral port and leaves it in `TIME_WAIT`,
/// so a few seconds of churn exhausts the machine's port range and the harness measures the
/// operating system's teardown rather than the protocol.
#[cfg(unix)]
fn unix_echo() -> std::path::PathBuf {
    use std::hash::{BuildHasher as _, RandomState};

    let path = std::env::temp_dir().join(format!(
        "paron-bench-{}-{:x}.sock",
        std::process::id(),
        RandomState::new().hash_one("paron")
    ));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();

    tokio::spawn(async move {
        while let Ok((mut connection, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = connection.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });

    path
}

/// `churns` picks the service behind the agent: a UNIX socket where the measurement opens and
/// closes without pause, TCP where it holds one stream and pushes bytes through it.
async fn one_agent(mapping: Mapping, churns: bool) -> One {
    #[cfg(unix)]
    let selector = if churns {
        Selector::unix(unix_echo()).unwrap()
    } else {
        Selector::tcp(tcp_echo().await).unwrap()
    };

    // An agent on Windows serves no UNIX sockets, so the churn goes through TCP there.
    #[cfg(not(unix))]
    let selector = {
        let _ = churns;

        Selector::tcp(tcp_echo().await).unwrap()
    };

    let broker = Arc::new(Broker::new(
        Arc::new(GrantAll),
        Arc::new(ConnectedAgents::new()),
        BrokerIdentity::generate().unwrap(),
        // Deployment liveness (§6.2): a benchmark probing at the floor would measure the probe
        // loop rather than the work.
        Liveness::default(),
        Limits::default(),
    ));
    let serving = match mapping {
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
    .unwrap();
    let address: BrokerAddress = format!("127.0.0.1:{}", serving.address().port())
        .parse()
        .unwrap();
    // Before the agent dials: the wait below is for it to register, which cannot happen while
    // nothing is accepting.
    let accepting = AbortOnDropHandle::new(tokio::spawn(serving.into_future()));

    let agent = Arc::new(Agent::new(
        vec![mapping.agent_transport()],
        PinnedBroker {
            address,
            root: broker.identity().fingerprint(),
            rollback: Arc::new(InMemoryRollbackIndex::default()),
        },
        AgentIdentity {
            id: agent_id(0),
            keys: Identity::generate().unwrap(),
        },
        Arc::new(Selectors::new([selector.clone().into()])),
        Arc::new(Counting::default()),
        Liveness::default(),
    ));
    tokio::spawn({
        let agent = Arc::clone(&agent);

        async move {
            let _ = agent.serve(std::future::pending()).await;
        }
    });

    tokio::time::timeout(
        Duration::from_secs(30),
        broker.connected_agents().wait_for_tunnel_to(agent_id(0)),
    )
    .await
    .expect("the agent under measurement never registered");

    One {
        broker,
        selector,
        _agent: agent,
        _accepting: accepting,
    }
}

/// Opening a tunnel stream and getting a byte back: the broker's open, the agent's accept, its
/// POL-8 answer and its connection to the service (§4.3, STM-20).
fn stream_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("stream_open");
    group.sample_size(50);

    for mapping in [Mapping::Tcp, Mapping::Quic] {
        let runtime = runtime();
        let one = runtime.block_on(one_agent(mapping, true));

        group.bench_function(mapping.name(), |b| {
            b.to_async(&runtime).iter_custom(|iterations| {
                let broker = Arc::clone(&one.broker);
                let selector = one.selector.clone();

                async move {
                    let started = Instant::now();

                    for _ in 0..iterations {
                        let mut stream = broker
                            .connected_agents()
                            .open(agent_id(0), &selector)
                            .await
                            .expect("a registered agent answers");
                        stream.write_all(b"x").await.unwrap();
                        let mut back = [0u8; 1];
                        stream.read_exact(&mut back).await.unwrap();
                    }

                    started.elapsed()
                }
            });
        });
    }

    group.finish();
}

/// Bytes through one open stream, 64 KiB at a time, there and back.
fn relay_throughput(c: &mut Criterion) {
    const CHUNK: usize = 64 * 1024;

    let mut group = c.benchmark_group("relay_throughput");
    group.sample_size(20);
    group.throughput(criterion::Throughput::Bytes(CHUNK as u64));

    for mapping in [Mapping::Tcp, Mapping::Quic] {
        let runtime = runtime();
        let one = runtime.block_on(one_agent(mapping, false));
        let stream = runtime.block_on(async {
            one.broker
                .connected_agents()
                .open(agent_id(0), &one.selector)
                .await
                .expect("a registered agent answers")
        });
        let stream = Arc::new(tokio::sync::Mutex::new(stream));

        group.bench_function(mapping.name(), |b| {
            b.to_async(&runtime).iter_custom(|iterations| {
                let stream = Arc::clone(&stream);

                async move {
                    let out = vec![0xA5u8; CHUNK];
                    let mut back = vec![0u8; CHUNK];
                    let mut stream = stream.lock().await;
                    let started = Instant::now();

                    for _ in 0..iterations {
                        stream.write_all(&out).await.unwrap();
                        stream.read_exact(&mut back).await.unwrap();
                    }

                    started.elapsed()
                }
            });
        });
    }

    group.finish();
}

/// Many forwards held on one connection at once: what a consumer with a lot of ports open
/// costs, up to each mapping's per-connection ceiling.
fn concurrent_forwards(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_forwards");
    group.sample_size(10);

    for mapping in [Mapping::Tcp, Mapping::Quic] {
        let runtime = runtime();
        let one = runtime.block_on(one_agent(mapping, true));

        for held in [16usize, 128] {
            group.bench_function(format!("{}/{held}", mapping.name()), |b| {
                b.to_async(&runtime).iter_custom(|iterations| {
                    let broker = Arc::clone(&one.broker);
                    let selector = one.selector.clone();

                    async move {
                        let started = Instant::now();

                        for _ in 0..iterations {
                            let mut open = Vec::with_capacity(held);

                            for _ in 0..held {
                                let mut stream = broker
                                    .connected_agents()
                                    .open(agent_id(0), &selector)
                                    .await
                                    .expect("under the per-connection ceiling");
                                stream.write_all(b"x").await.unwrap();
                                let mut back = [0u8; 1];
                                stream.read_exact(&mut back).await.unwrap();
                                open.push(stream);
                            }

                            drop(open);
                        }

                        started.elapsed()
                    }
                });
            });
        }
    }

    group.finish();
}

criterion_group!(benches, stream_open, relay_throughput, concurrent_forwards);
criterion_main!(benches);
