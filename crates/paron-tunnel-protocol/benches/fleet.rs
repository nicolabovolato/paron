//! What a broker costs to run a fleet, measured on the broker alone.
//!
//! Two processes: this binary runs itself once as the broker and once as the machines, so the
//! CPU and memory reported for the broker are its own and not an average of both ends. The
//! broker's runtime is pinned to a worker count, which is what stands in for a CPU bound; the
//! numbers are kernel-accounted (`getrusage`), not sampled.
//!
//! ```text
//! cargo bench -p paron-tunnel-protocol --bench fleet -- --agents 1000
//! cargo bench -p paron-tunnel-protocol --bench fleet -- --agents 200 --load --quic
//! ```

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
use std::io::BufRead as _;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use harness::{Counting, GrantAll, agent_id, tcp_echo};
use paron_tunnel_protocol::quic::{QuicAgentTransport, QuicBrokerTransport};
use paron_tunnel_protocol::tcp::{TcpAgentTransport, TcpBrokerTransport};
use paron_tunnel_protocol::{
    Agent, AgentIdentity, AgentTransport, Broker, BrokerAddress, BrokerIdentity, ConnectedAgents,
    Fingerprint, Identity, InMemoryRollbackIndex, Limits, Liveness, PinnedBroker, Selector,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// What one process has spent: CPU accounted by the kernel across every thread of the runtime,
/// and resident memory as the kernel reports it, not as the allocator believes.
struct Spent {
    cpu: Duration,
    rss_kb: u64,
}

fn spent(watching: &mut sysinfo::System) -> Spent {
    let mine = sysinfo::Pid::from_u32(std::process::id());
    watching.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[mine]), true);

    let process = watching.process(mine).expect("this process exists");

    Spent {
        cpu: Duration::from_millis(process.accumulated_cpu_time()),
        rss_kb: process.memory() / 1024,
    }
}

struct Args {
    role: Option<String>,
    agents: usize,
    workers: usize,
    seconds: u64,
    quic: bool,
    load: bool,
    address: String,
    key: String,
}

fn args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    let value = |name: &str, fallback: &str| -> String {
        raw.iter()
            .position(|a| a == name)
            .and_then(|at| raw.get(at + 1))
            .cloned()
            .unwrap_or_else(|| fallback.to_owned())
    };
    let flag = |name: &str| raw.iter().any(|a| a == name);

    Args {
        role: raw
            .iter()
            .position(|a| a == "--role")
            .and_then(|at| raw.get(at + 1))
            .cloned(),
        agents: value("--agents", "200").parse().unwrap(),
        workers: value("--workers", "2").parse().unwrap(),
        seconds: value("--seconds", "10").parse().unwrap(),
        quic: flag("--quic"),
        load: flag("--load"),
        address: value("--address", ""),
        key: value("--key", ""),
    }
}

fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
}

fn main() {
    let args = args();

    match args.role.as_deref() {
        Some("broker") => runtime(args.workers).block_on(broker(&args)),
        Some("machines") => runtime(num_cpus()).block_on(machines(&args)),
        _ => parent(&args),
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get)
}

/// Runs both halves and prints one report.
fn parent(args: &Args) {
    let binary = std::env::current_exe().unwrap();
    let mapping = if args.quic { "quic" } else { "tcp" };

    let mut broker = std::process::Command::new(&binary)
        .args(["--role", "broker"])
        .args(["--agents", &args.agents.to_string()])
        .args(["--workers", &args.workers.to_string()])
        .args(["--seconds", &args.seconds.to_string()])
        .args(if args.quic { vec!["--quic"] } else { vec![] })
        .args(if args.load { vec!["--load"] } else { vec![] })
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut out = std::io::BufReader::new(broker.stdout.take().unwrap());
    let mut line = String::new();
    let (mut address, mut key) = (String::new(), String::new());

    while out.read_line(&mut line).unwrap() > 0 {
        if let Some(rest) = line.strip_prefix("ADDRESS ") {
            rest.trim().clone_into(&mut address);
        } else if let Some(rest) = line.strip_prefix("KEY ") {
            rest.trim().clone_into(&mut key);
            break;
        }
        line.clear();
    }

    let mut machines = std::process::Command::new(&binary)
        .args(["--role", "machines"])
        .args(["--agents", &args.agents.to_string()])
        .args(["--seconds", &args.seconds.to_string()])
        .args(["--address", &address])
        .args(["--key", &key])
        .args(if args.quic { vec!["--quic"] } else { vec![] })
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut echo = String::new();
    std::io::BufReader::new(machines.stdout.take().unwrap())
        .read_line(&mut echo)
        .unwrap();
    std::io::Write::write_all(
        broker.stdin.as_mut().unwrap(),
        format!("{}\n", echo.trim()).as_bytes(),
    )
    .unwrap();

    println!(
        "\n== {} agents over {mapping}, broker pinned to {} worker{}, {}s{} ==",
        args.agents,
        args.workers,
        if args.workers == 1 { "" } else { "s" },
        args.seconds,
        if args.load { ", relaying" } else { ", idle" },
    );

    // The broker prints its own report on the remaining lines.
    line.clear();
    while out.read_line(&mut line).unwrap() > 0 {
        print!("{line}");
        line.clear();
    }

    let _ = broker.wait();
    let _ = machines.wait().map(|status| status.success());
}

#[allow(
    clippy::too_many_lines,
    reason = "a harness reads better as one sequence than as a call graph of one-use helpers"
)]
async fn broker(args: &Args) {
    let tunnels = Arc::new(ConnectedAgents::new());
    let broker = Arc::new(Broker::new(
        Arc::new(GrantAll),
        Arc::clone(&tunnels),
        BrokerIdentity::generate().unwrap(),
        Liveness::default(),
        Limits {
            max_unadmitted: (args.agents * 4).max(64),
            admission_timeout: Duration::from_secs(30),
        },
    ));

    let serving = if args.quic {
        broker
            .bind(&QuicBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap()
    } else {
        broker
            .bind(&TcpBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap()
    };

    println!("ADDRESS 127.0.0.1:{}", serving.address().port());
    println!("KEY {}", broker.identity().fingerprint());

    tokio::spawn(serving.into_future());

    // The machines bind the echo, so its cost is theirs; the port comes back over stdin. A
    // selector names a port and the agent reaches that port on its own loopback (§2.5), which
    // here is this same host.
    let echo_port: u16 = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        line.trim().parse().unwrap()
    })
    .await
    .unwrap();

    let mut watching = sysinfo::System::new().expect("sysinfo supports this platform");
    let before = spent(&mut watching);
    let started = Instant::now();

    // Wait for the fleet.
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut up = 0;
    while up < args.agents && Instant::now() < deadline {
        up = (0..args.agents)
            .filter(|n| tunnels.tunnel_to(agent_id(*n)).is_some())
            .count();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let connected_in = started.elapsed();
    let after_connect = spent(&mut watching);

    let relayed = Arc::new(AtomicU64::new(0));

    if args.load {
        const CHUNK: usize = 64 * 1024;
        let until = Instant::now() + Duration::from_secs(args.seconds);
        let mut relays = Vec::new();

        for n in 0..up {
            let tunnels = Arc::clone(&tunnels);
            let relayed = Arc::clone(&relayed);

            relays.push(tokio::spawn(async move {
                let Ok(mut stream) = tunnels
                    .open(agent_id(n), &Selector::tcp(echo_port).unwrap())
                    .await
                else {
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

        for relay in relays {
            let _ = relay.await;
        }
    } else {
        tokio::time::sleep(Duration::from_secs(args.seconds)).await;
    }

    let end = spent(&mut watching);
    let still_up = (0..args.agents)
        .filter(|n| tunnels.tunnel_to(agent_id(*n)).is_some())
        .count();

    let idle_cpu = after_connect.cpu.saturating_sub(before.cpu);
    let run_cpu = end.cpu.saturating_sub(after_connect.cpu);
    let moved = relayed.load(Ordering::Relaxed);

    println!(
        "  broker  registered {up}/{} in {connected_in:?}",
        args.agents
    );
    println!(
        "  broker  cpu: {:?} to admit the fleet, {:?} over {}s {} ({:.2}% of one core)",
        idle_cpu,
        run_cpu,
        args.seconds,
        if args.load { "relaying" } else { "idle" },
        run_cpu.as_secs_f64() / args.seconds as f64 * 100.0,
    );
    println!(
        "  broker  rss {} KB before the fleet -> {} KB, {:.1} KB per agent",
        before.rss_kb,
        end.rss_kb,
        end.rss_kb.saturating_sub(before.rss_kb) as f64 / args.agents as f64,
    );
    if args.load {
        println!(
            "  broker  relayed {:.1} MB ({:.1} MB/s)",
            moved as f64 / 1e6,
            moved as f64 / 1e6 / args.seconds as f64,
        );
    }
    println!(
        "  broker  {still_up}/{} still registered at the end",
        args.agents
    );
}

async fn machines(args: &Args) {
    let address: BrokerAddress = args.address.parse().unwrap();
    let key = Fingerprint::parse(&args.key).unwrap();
    let port = tcp_echo().await;
    println!("{port}");

    let mut held = Vec::with_capacity(args.agents);
    for n in 0..args.agents {
        let transport: Box<dyn AgentTransport> = if args.quic {
            Box::new(QuicAgentTransport)
        } else {
            Box::new(TcpAgentTransport)
        };
        let agent = Arc::new(Agent::new(
            vec![transport],
            PinnedBroker {
                address: address.clone(),
                root: key.clone(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id: agent_id(n),
                keys: Identity::generate().unwrap(),
            },
            Arc::new(Fixed(port)),
            Arc::new(Counting::default()),
            Liveness::default(),
        ));
        let serving = Arc::clone(&agent);
        tokio::spawn(async move {
            let _ = serving.serve(std::future::pending()).await;
        });
        held.push(agent);
    }

    tokio::time::sleep(Duration::from_secs(args.seconds + 8)).await;

    let mut watching = sysinfo::System::new().expect("sysinfo supports this platform");
    let end = spent(&mut watching);
    eprintln!(
        "  machines cpu {:?}, rss {} KB ({:.1} KB per agent)",
        end.cpu,
        end.rss_kb,
        end.rss_kb as f64 / args.agents as f64,
    );
}

/// Answers the one selector the broker asks for with the local echo, whatever port it names.
struct Fixed(u16);

impl paron_tunnel_protocol::ServedSelectors for Fixed {
    fn announced(&self) -> paron_tunnel_protocol::AnnounceSet {
        paron_tunnel_protocol::AnnounceSet::new([Selector::tcp(self.0).unwrap().into()])
    }

    fn serves(&self, _selector: &Selector) -> bool {
        true
    }
}
