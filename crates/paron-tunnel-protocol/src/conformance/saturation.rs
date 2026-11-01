//! A whole fleet relaying at once, on a runtime narrow enough that the control plane has to
//! compete for it.
//!
//! The failure this exists to catch is a scheduling one: relays are the only work here without a
//! bound, and if they can starve the probe loop then a busy broker deregisters connections that
//! are perfectly healthy (§6.2). Not a member of §11 — a conforming implementation could be slow
//! — but the same fixture, driven until it hurts.

use std::future::IntoFuture as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::task::AbortOnDropHandle;

use super::records::{Counting, Records};
use crate::quic::{QuicAgentTransport, QuicBrokerTransport};
use crate::tcp::{TcpAgentTransport, TcpBrokerTransport};
use crate::transport::AgentTransport;
use crate::{
    Agent, AgentId, AgentIdentity, Broker, BrokerAddress, BrokerIdentity, BrokerPolicy, Identity,
    InMemoryRollbackIndex, Limits, Liveness, PinnedBroker, Selector, Selectors,
};

/// Small enough for a test run, wide enough that every worker is busy.
const AGENTS: usize = 50;
const WORKERS: usize = 2;
const RELAY_FOR: Duration = Duration::from_secs(2);
const CHUNK: usize = 64 * 1024;

/// Nothing here may outlast this. A stall is the failure being looked for, so every wait is
/// bounded and the whole run is bounded again around them.
const BEFORE_STALLED: Duration = Duration::from_mins(1);

#[derive(Clone, Copy)]
enum Mapping {
    Quic,
    Tcp,
}

impl Mapping {
    fn agent_transport(self) -> Box<dyn AgentTransport> {
        match self {
            Self::Quic => Box::new(QuicAgentTransport),
            Self::Tcp => Box::new(TcpAgentTransport),
        }
    }
}

#[test]
fn tcp_carries_every_agent_at_once_without_stalling() {
    carries_every_agent(Mapping::Tcp);
}

#[test]
fn quic_carries_every_agent_at_once_without_stalling() {
    carries_every_agent(Mapping::Quic);
}

fn carries_every_agent(mapping: Mapping) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async move {
        tokio::time::timeout(BEFORE_STALLED, relay_every_agent(mapping))
            .await
            .expect("the run stalled: no branch of this test may wait unbounded");
    });
}

async fn relay_every_agent(mapping: Mapping) {
    let port = echo().await;
    let selector = Selector::tcp(port).unwrap();
    let records = Records::new();
    let broker = Arc::new(Broker::new(
        Arc::clone(&records) as Arc<dyn BrokerPolicy>,
        Arc::new(crate::ConnectedAgents::new()),
        BrokerIdentity::generate().unwrap(),
        // Deployment liveness, not the kit's: what is under test is whether probes keep up while
        // the runtime is busy, and at the kit's 150 ms they would not be expected to.
        Liveness::default(),
        Limits {
            max_unadmitted: AGENTS * 4,
            admission_timeout: Duration::from_secs(30),
        },
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
    let bound = serving.address();
    let _accepting = AbortOnDropHandle::new(tokio::spawn(serving.into_future()));

    let agents = connect(&broker, &records, bound, mapping, &selector);

    wait_for_fleet(&broker).await;

    let relayed = Arc::new(AtomicU64::new(0));
    let until = Instant::now() + RELAY_FOR;
    let mut relays = Vec::with_capacity(AGENTS);

    for n in 0..AGENTS {
        let tunnels = Arc::clone(broker.connected_agents());
        let relayed = Arc::clone(&relayed);
        let selector = selector.clone();

        relays.push(tokio::spawn(async move {
            let Ok(mut stream) = tunnels.open(agent_id(n), &selector).await else {
                return;
            };
            let out = vec![0xA5u8; CHUNK];
            let mut back = vec![0u8; CHUNK];

            while Instant::now() < until {
                if stream.write_all(&out).await.is_err()
                    || stream.read_exact(&mut back).await.is_err()
                {
                    return;
                }
                relayed.fetch_add(CHUNK as u64, Ordering::Relaxed);
            }
        }));
    }

    // Under way, and now the control plane has to work through it.
    tokio::time::sleep(RELAY_FOR / 2).await;

    let opened = tokio::time::timeout(
        Duration::from_secs(20),
        broker.connected_agents().open(agent_id(0), &selector),
    )
    .await
    .expect("a stream open never answered while the fleet relayed")
    .expect("an agent that is registered answers");
    drop(opened);

    for relay in relays {
        let _ = relay.await;
    }

    assert!(
        relayed.load(Ordering::Relaxed) > 0,
        "nothing relayed, so this measured nothing"
    );

    let still_up = (0..AGENTS)
        .filter(|n| broker.connected_agents().tunnel_to(agent_id(*n)).is_some())
        .count();
    assert_eq!(
        still_up, AGENTS,
        "the control plane starved: probes went unanswered past the detection bound while data \
         flowed"
    );

    drop(agents);
}

fn agent_id(n: usize) -> AgentId {
    AgentId::parse(&format!("00000000-0000-7000-8000-{n:012x}")).unwrap()
}

/// Every agent joined before it dials, so a claim is granted the moment one arrives.
fn connect(
    broker: &Arc<Broker>,
    records: &Records,
    bound: SocketAddr,
    mapping: Mapping,
    selector: &Selector,
) -> Vec<Arc<Agent>> {
    let address: BrokerAddress = format!("127.0.0.1:{}", bound.port()).parse().unwrap();
    let mut held = Vec::with_capacity(AGENTS);

    for n in 0..AGENTS {
        let identity = Identity::generate().unwrap();
        records.joined(agent_id(n), identity.public_key());

        let agent = Arc::new(Agent::new(
            vec![mapping.agent_transport()],
            PinnedBroker {
                address: address.clone(),
                root: broker.identity().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id: agent_id(n),
                keys: identity,
            },
            Arc::new(Selectors::new([selector.clone().into()])),
            Counting::new(),
            Liveness::default(),
        ));
        tokio::spawn({
            let agent = Arc::clone(&agent);

            async move {
                let _ = agent.serve(std::future::pending()).await;
            }
        });
        held.push(agent);
    }

    held
}

async fn wait_for_fleet(broker: &Broker) {
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        let up = (0..AGENTS)
            .filter(|n| broker.connected_agents().tunnel_to(agent_id(*n)).is_some())
            .count();

        assert!(Instant::now() < deadline, "the fleet never came up: {up}");

        if up == AGENTS {
            return;
        }

        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        while let Ok((mut connection, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = connection.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });

    port
}
