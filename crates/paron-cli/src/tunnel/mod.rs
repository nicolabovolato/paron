use std::sync::Arc;

use paron_core::domain::machine::tunnel::token::ServerEndpoint;
use paron_tunnel_protocol::quic::QuicAgentTransport;
use paron_tunnel_protocol::tcp::TcpAgentTransport;
use paron_tunnel_protocol::{
    Agent, AgentIdentity, Announced, Identity, Liveness, PinnedBroker, PortRange, RollbackIndex,
    Selector, Selectors, Sequence,
};

use crate::tunnel::cli::{ServedArgs, TunnelRunArgs};
use crate::tunnel::data_dir::rollback::OnDiskRollbackIndex;
use crate::tunnel::data_dir::sequence::OnDiskSequence;
use crate::{Result, platform};

mod bootstrap;
pub mod cli;
mod data_dir;
mod enrol;

async fn run(args: TunnelRunArgs) -> Result<()> {
    tracing::info!("Paron tunnel {}", env!("CARGO_PKG_VERSION"));
    if platform::privileges::is_user_root() {
        tracing::warn!("running as root");
    }

    let _guard = platform::files::lock_directory_as_owner(&args.data.data_dir)?;
    let contents = data_dir::open(&args.data.data_dir)?;

    tracing::info!("connecting to {}", contents.server_endpoint.address);

    let tunnel = agent(
        &contents.server_endpoint,
        contents.agent_identity,
        Arc::new(OnDiskSequence::new(&args.data.data_dir)),
        Arc::new(OnDiskRollbackIndex::new(&args.data.data_dir)),
        announced_selectors(&args.served)?,
    );

    let stop_signal = tokio_util::sync::CancellationToken::new();

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn({
        let stop_signal = stop_signal.clone();
        async move {
            tunnel
                .serve_continuously(stop_signal.cancelled_owned())
                .await;

            Ok(())
        }
    });

    crate::lifecycle::run(tasks, stop_signal, std::future::ready(())).await
}

fn agent(
    server_endpoint: &ServerEndpoint,
    identity: Identity,
    sequence: Arc<dyn Sequence>,
    rollback: Arc<dyn RollbackIndex>,
    announced: Vec<Announced>,
) -> Arc<Agent> {
    Arc::new(Agent::new(
        vec![Box::new(QuicAgentTransport), Box::new(TcpAgentTransport)],
        PinnedBroker {
            address: server_endpoint.address.clone(),
            root: server_endpoint.fingerprint.clone(),
            rollback,
        },
        AgentIdentity {
            id: server_endpoint.machine,
            keys: identity,
        },
        Arc::new(Selectors::new(announced)),
        sequence,
        Liveness::default(),
    ))
}

fn announced_selectors(args: &ServedArgs) -> Result<Vec<Announced>> {
    let mut members = Vec::new();

    if args.all_ports {
        members.push(Announced::TcpRange(PortRange::every()));
        members.push(Announced::UdpRange(PortRange::every()));
    } else {
        for &port in args.ports.as_deref().unwrap_or_default() {
            let (Some(tcp), Some(udp)) = (Selector::tcp(port), Selector::udp(port)) else {
                return Err(format!("{port} is not a port").into());
            };

            members.push(tcp.into());
            members.push(udp.into());
        }
    }

    #[cfg(unix)]
    if args.all_sockets {
        members.push(Announced::UnixAny);
    } else {
        for path in args.sockets.as_deref().unwrap_or_default() {
            let selector = Selector::unix(path)
                .ok_or_else(|| format!("{} is not a UNIX socket path", path.display()))?;

            members.push(selector.into());
        }
    }

    Ok(members)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use paron_tunnel_protocol::{Selectors, ServedSelectors as _};

    use super::*;

    #[cfg_attr(windows, allow(unused_variables, clippy::needless_pass_by_value))]
    fn served_args(ports: Option<Vec<u16>>, sockets: Option<Vec<PathBuf>>) -> ServedArgs {
        ServedArgs {
            ports,
            all_ports: false,
            #[cfg(unix)]
            sockets,
            #[cfg(unix)]
            all_sockets: false,
        }
    }

    fn served_selectors(args: &ServedArgs) -> Selectors {
        Selectors::new(announced_selectors(args).unwrap())
    }

    // The count is a requirement. One `ANNOUNCE` payload cannot hold 65535 port selectors.
    #[test]
    fn announced_selectors_all_ports_returns_two_selectors_that_serve_tcp_and_udp() {
        let selectors = served_selectors(&ServedArgs {
            all_ports: true,
            ..served_args(None, None)
        });

        assert_eq!(selectors.announced().len(), 2);

        for port in [1, 22, 65535] {
            assert!(
                selectors.serves(&Selector::tcp(port).unwrap()),
                "tcp {port}"
            );
            assert!(
                selectors.serves(&Selector::udp(port).unwrap()),
                "udp {port}"
            );
        }

        assert!(
            !selectors.serves(&Selector::unix("/run/docker.sock").unwrap()),
            "the port answer must not serve a socket"
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_socket_argument_on_windows_returns_unknown_argument() {
        use clap::Parser as _;

        let parsed = crate::cli::Cli::try_parse_from([
            "paron",
            "tunnel",
            "run",
            "--data",
            r"C:\ProgramData\paron-tunnel",
            "--sockets",
            r"C:\x.sock",
        ]);

        assert_eq!(
            parsed.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::UnknownArgument)
        );
    }

    #[cfg(unix)]
    #[test]
    fn announced_selectors_all_sockets_serves_no_port() {
        let selectors = served_selectors(&ServedArgs {
            all_sockets: true,
            ..served_args(None, None)
        });

        assert_eq!(selectors.announced().len(), 1);
        assert!(selectors.serves(&Selector::unix("/run/docker.sock").unwrap()));
        assert!(!selectors.serves(&Selector::tcp(22).unwrap()));
    }

    #[test]
    fn announced_selectors_named_port_serves_tcp_and_udp_and_no_other_port() {
        let selectors = served_selectors(&served_args(Some(vec![22]), None));

        assert!(selectors.serves(&Selector::tcp(22).unwrap()));
        assert!(selectors.serves(&Selector::udp(22).unwrap()));
        assert!(!selectors.serves(&Selector::tcp(23).unwrap()));
    }

    #[test]
    fn announced_selectors_port_zero_returns_error() {
        let Err(error) = announced_selectors(&served_args(Some(vec![0]), None)) else {
            panic!("0 is not a port");
        };

        assert!(error.to_string().contains("is not a port"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn announced_selectors_path_it_cannot_encode_returns_error() {
        let Err(error) = announced_selectors(&served_args(None, Some(vec![PathBuf::new()]))) else {
            panic!("an empty path is not one PTP/1 can name");
        };

        assert!(
            error.to_string().contains("is not a UNIX socket path"),
            "{error}"
        );
    }
}
