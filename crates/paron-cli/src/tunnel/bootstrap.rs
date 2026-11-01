use std::path::{Path, PathBuf};
use std::sync::Arc;

use paron_core::domain::machine::tunnel::token::Token;

use crate::bootstrap::daemon::{self, SystemDaemon};
use crate::daemon_manager::Daemon;
use crate::tunnel::cli::{ServedArgs, TunnelBootstrapArgs, TunnelDataArgs, TunnelRunArgs};
use crate::tunnel::data_dir;
use crate::tunnel::enrol;
use crate::ui::wizard::Wizard;
use crate::{Result, bootstrap, platform, ui};

#[cfg(target_os = "macos")]
const DAEMON_NAME: &str = "dev.bovolato.paron.tunnel";
#[cfg(not(target_os = "macos"))]
const DAEMON_NAME: &str = "paron-tunnel";
const DAEMON_DESCRIPTION: &str = "Paron tunnel";

const DEFAULT_INSTALL_DIRECTORY: &str = "paron-tunnel";

pub async fn run(args: TunnelBootstrapArgs) -> Result<()> {
    ui::print_banner();

    if let Some(dir) = &args.data_dir
        && data_dir::installed(dir)?
    {
        println!(
            "{} is already a Paron installation, skipping bootstrap",
            dir.display()
        );

        return Ok(());
    }

    let mut wizard = Wizard::new(args.prompt.no_prompt, "tunnel bootstrap");

    let setup = Setup::prepare(args, &mut wizard)?;

    if !wizard.yes_no("These answers are final. Install Paron?", true)? {
        wizard.cancel("cancelled");

        return Ok(());
    }

    setup.install(&mut wizard).await
}

struct Setup {
    data_dir: PathBuf,
    token: Token,

    run_cmd: Vec<String>,
    run_args: TunnelRunArgs,

    system_daemon: Option<SystemDaemon>,
}

impl Setup {
    #[allow(clippy::too_many_lines)]
    fn prepare(args: TunnelBootstrapArgs, wizard: &mut Wizard) -> Result<Self> {
        let data_dir = wizard.input(
            "Tunnel data directory",
            Some(&args.data_dir.map_or_else(
                || {
                    Path::new(platform::files::SERVICE_DATA_ROOT)
                        .join(DEFAULT_INSTALL_DIRECTORY)
                        .display()
                        .to_string()
                },
                |data| data.display().to_string(),
            )),
            None,
            bootstrap::parse_data_dir,
        )?;

        let token = match &args.token {
            Some(token) => {
                wizard.log_input("Enrollment token", TunnelBootstrapArgs::NAMES.token.flag);

                token.parse::<Token>()?
            }
            None => wizard
                .input(
                    "Enrollment token",
                    None,
                    Some("paron://host:port/machine?fingerprint=sha256:<hex>&secret=<hex>"),
                    |typed: &str| Ok(typed.parse::<Token>()?),
                )
                .map_err(ui::not_interactive_needs_flag(
                    TunnelBootstrapArgs::NAMES.token.flag,
                ))?,
        };

        let ports = wizard.input(
            "Ports to serve through the tunnel",
            Some(&Ports::from(&args.served).to_string()),
            Some(&format!(
                "22, 5432, or '{}' (comma separated), leave empty to disable",
                Ports::ALL_STR
            )),
            |typed: &str| typed.parse::<Ports>(),
        )?;

        #[cfg(unix)]
        let sockets = wizard.input(
            "UNIX sockets to serve through the tunnel",
            Some(&Sockets::from(&args.served).to_string()),
            Some(&format!(
                "/run/docker.sock, or '{}' (comma separated), leave empty to disable",
                Sockets::ALL_STR
            )),
            |typed: &str| {
                let sockets = typed.parse::<Sockets>()?;

                if let Sockets::Values(paths) = &sockets {
                    for path in paths {
                        if !platform::unix::files::is_unix_socket(path) {
                            return Err(format!("{} is not a socket", path.display()).into());
                        }
                    }
                }

                Ok(sockets)
            },
        )?;

        let run_args = TunnelRunArgs {
            data: TunnelDataArgs {
                data_dir: data_dir.clone(),
            },
            served: ServedArgs {
                ports: match &ports {
                    Ports::Values(ports) if !ports.is_empty() => Some(ports.clone()),
                    _ => None,
                },
                all_ports: matches!(ports, Ports::All),
                #[cfg(unix)]
                sockets: match &sockets {
                    Sockets::Values(sockets) if !sockets.is_empty() => Some(sockets.clone()),
                    _ => None,
                },
                #[cfg(unix)]
                all_sockets: matches!(sockets, Sockets::All),
            },
        };

        let run_cmd = vec![
            std::env::current_exe()?.display().to_string(),
            "tunnel".to_owned(),
            "run".to_owned(),
        ];

        let system_daemon = daemon::ask(
            Daemon {
                name: DAEMON_NAME,
                description: DAEMON_DESCRIPTION,
                command: run_cmd.clone(),
                environment: run_args.environment(),
                privileged_ports: false,
                data_dir: data_dir.clone(),
            },
            args.install.daemon,
            #[cfg(unix)]
            match &sockets {
                Sockets::Values(paths) => paths.as_slice(),
                Sockets::All => &[],
            },
            wizard,
        )?;

        Ok(Self {
            data_dir,
            token,
            run_cmd,
            run_args,
            system_daemon,
        })
    }

    async fn install(self, wizard: &mut Wizard) -> Result<()> {
        bootstrap::ask_for_root(&self.data_dir, self.system_daemon.as_ref())?;

        let step = wizard.install_step(&format!("creating {}", self.data_dir.display()));
        bootstrap::create_data_dir(&self.data_dir)?;
        let data_dir = data_dir::init(&self.data_dir, &self.token)?;
        step.finish();

        let step = wizard.install_step(&format!(
            "enrolling to {}",
            self.token.server_endpoint.address
        ));
        enrol::enrol(
            &self.token,
            data_dir.agent_identity,
            Arc::new(data_dir::sequence::OnDiskSequence::new(&self.data_dir)),
            Arc::new(data_dir::rollback::OnDiskRollbackIndex::new(&self.data_dir)),
        )
        .await?;
        step.finish();

        if let Some(daemon) = &self.system_daemon {
            daemon.install(wizard)?;
        }

        wizard.done("completed");

        println!();

        if let Some(daemon) = &self.system_daemon {
            println!("follow the tunnel logs with:");
            println!("\t{}", platform::shell::join(daemon.logs_follow_command()));
        } else {
            println!("start the tunnel with:");
            println!(
                "\t{}",
                platform::shell::join(self.run_cmd.iter().chain(&self.run_args.flags()))
            );
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ports {
    All,
    Values(Vec<u16>),
}

impl Ports {
    const ALL_STR: &str = "all";
}

impl From<&ServedArgs> for Ports {
    fn from(args: &ServedArgs) -> Self {
        match (args.all_ports, &args.ports) {
            (true, _) => Self::All,
            (false, ports) => Self::Values(ports.clone().unwrap_or_default()),
        }
    }
}

impl std::fmt::Display for Ports {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => formatter.write_str(Self::ALL_STR),
            Self::Values(ports) => formatter.write_str(
                &ports
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }
}

impl std::str::FromStr for Ports {
    type Err = crate::Error;

    fn from_str(typed: &str) -> Result<Self> {
        if typed.trim().to_lowercase() == Self::ALL_STR {
            return Ok(Self::All);
        }

        typed
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|port| {
                port.parse::<u16>()
                    .map_err(|_| crate::Error::from(format!("{port} is not a port")))
            })
            .collect::<Result<Vec<u16>>>()
            .map(Self::Values)
    }
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub enum Sockets {
    All,
    Values(Vec<PathBuf>),
}

#[cfg(unix)]
impl Sockets {
    const ALL_STR: &str = "all";
}

#[cfg(unix)]
impl From<&ServedArgs> for Sockets {
    fn from(args: &ServedArgs) -> Self {
        match (args.all_sockets, &args.sockets) {
            (true, _) => Self::All,
            (false, sockets) => Self::Values(sockets.clone().unwrap_or_default()),
        }
    }
}

#[cfg(unix)]
impl std::fmt::Display for Sockets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => formatter.write_str(Self::ALL_STR),
            Self::Values(sockets) => formatter.write_str(
                &sockets
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }
}

#[cfg(unix)]
impl std::str::FromStr for Sockets {
    type Err = crate::Error;

    fn from_str(typed: &str) -> Result<Self> {
        if typed.trim().to_lowercase() == Self::ALL_STR {
            return Ok(Self::All);
        }

        let paths: Vec<PathBuf> = typed
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();

        for path in &paths {
            if !path.is_absolute() {
                return Err(format!("{} is not an absolute path", path.display()).into());
            }

            if paron_tunnel_protocol::Selector::unix(path).is_none() {
                return Err(format!("{} is too long", path.display()).into());
            }
        }

        Ok(Self::Values(paths))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_ports_returns_the_same_answer() {
        for ports in [
            Ports::All,
            Ports::Values(vec![22, 5432]),
            Ports::Values(Vec::new()),
        ] {
            assert_eq!(ports.to_string().parse::<Ports>().unwrap(), ports);
        }
    }

    #[cfg(unix)]
    #[test]
    fn round_trip_sockets_returns_the_same_answer() {
        for sockets in [
            Sockets::All,
            Sockets::Values(vec![PathBuf::from("/run/docker.sock")]),
            Sockets::Values(Vec::new()),
        ] {
            assert_eq!(sockets.to_string().parse::<Sockets>().unwrap(), sockets);
        }
    }

    #[cfg(unix)]
    #[test]
    fn parse_sockets_relative_path_returns_error() {
        let parsed = "run/docker.sock".parse::<Sockets>();

        assert_eq!(
            parsed.unwrap_err().to_string(),
            "run/docker.sock is not an absolute path"
        );
    }
}
