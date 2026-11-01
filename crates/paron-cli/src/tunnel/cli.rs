use std::path::PathBuf;

use crate::Result;
use crate::cli::{
    ConfirmArgs, ConfirmArgsNames, InstallArgs, InstallArgsNames, PromptArgs, PromptArgsNames,
};
use crate::tunnel::{bootstrap, enrol};

#[derive(clap::Subcommand)]
pub enum TunnelCommand {
    /// Connect to Paron server
    Run(TunnelRunArgs),

    /// Set up a fresh installation
    Bootstrap(TunnelBootstrapArgs),

    /// Enrol to Paron server
    Enrol(TunnelEnrolArgs),
}

pub async fn run(command: TunnelCommand) -> Result<()> {
    match command {
        TunnelCommand::Run(args) => super::run(args).await,
        TunnelCommand::Bootstrap(args) => bootstrap::run(args).await,
        TunnelCommand::Enrol(args) => enrol::run(args).await,
    }
}

#[derive(clap::Args, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct TunnelRunArgs {
    #[command(flatten)]
    pub data: TunnelDataArgs,

    #[command(flatten)]
    pub served: ServedArgs,
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct TunnelBootstrapArgs {
    /// Tunnel data directory.
    ///
    /// Example: --data /var/lib/paron-tunnel
    #[arg(long = "data", env = "PARON_TUNNEL_DATA", value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// The enrollment token.
    ///
    /// Example: --token paron://paron.example.com:8443/<machine>?fingerprint=sha256:<hex>&secret=<hex>
    #[arg(long, env = "PARON_TUNNEL_TOKEN", hide_env_values = true)]
    pub token: Option<String>,

    #[command(flatten)]
    pub served: ServedArgs,

    #[command(flatten)]
    pub install: InstallArgs,

    #[command(flatten)]
    pub prompt: PromptArgs,
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct TunnelEnrolArgs {
    #[command(flatten)]
    pub data: TunnelDataArgs,

    /// The enrollment token.
    ///
    /// Example: --token paron://paron.example.com:8443/<machine>?fingerprint=sha256:<hex>&secret=<hex>
    #[arg(long, env = "PARON_TUNNEL_TOKEN", hide_env_values = true)]
    pub token: String,

    #[command(flatten)]
    pub confirm: ConfirmArgs,
}

#[derive(clap::Args, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct ServedArgs {
    /// Allow Paron server to reach these ports, comma separated.
    ///
    /// Example: --ports 22,5432
    #[arg(
        long,
        env = "PARON_TUNNEL_PORTS",
        value_delimiter = ',',
        conflicts_with = "all_ports"
    )]
    pub ports: Option<Vec<u16>>,

    /// Allow Paron server to reach every TCP and UDP port on this machine.
    #[arg(long, env = "PARON_TUNNEL_ALL_PORTS")]
    pub all_ports: bool,

    /// Allow Paron server to reach UNIX-domain sockets on this machine, comma separated.
    ///
    /// Example: --sockets /run/docker.sock,/var/run/postgresql/.s.PGSQL.5432
    #[cfg(unix)]
    #[arg(
        long,
        env = "PARON_TUNNEL_SOCKETS",
        value_delimiter = ',',
        conflicts_with = "all_sockets"
    )]
    pub sockets: Option<Vec<PathBuf>>,

    /// Allow Paron server to reach every UNIX-domain socket on this machine.
    ///
    /// The account on which Paron tunnel is running must have access to the sockets.
    #[cfg(unix)]
    #[arg(long, env = "PARON_TUNNEL_ALL_SOCKETS")]
    pub all_sockets: bool,
}

#[derive(clap::Args, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct TunnelDataArgs {
    /// Tunnel data directory.
    ///
    /// Example: --data /var/lib/paron-tunnel
    #[arg(long = "data", env = "PARON_TUNNEL_DATA", value_name = "DIR")]
    pub data_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::Cli;

    #[test]
    #[cfg_attr(windows, allow(clippy::single_element_loop))]
    fn parse_list_and_wildcard_for_one_kind_returns_argument_conflict() {
        for served_arguments in [
            ["--ports", "22", "--all-ports"],
            #[cfg(unix)]
            ["--sockets", "/run/x.sock", "--all-sockets"],
        ] {
            let parsed = Cli::try_parse_from(
                ["paron", "tunnel", "run", "--data", "/var/lib/paron-tunnel"]
                    .into_iter()
                    .chain(served_arguments),
            );

            assert_eq!(
                parsed.err().map(|error| error.kind()),
                Some(clap::error::ErrorKind::ArgumentConflict),
                "{served_arguments:?}"
            );
        }
    }
}
