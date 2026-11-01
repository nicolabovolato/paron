use crate::Result;
use crate::{server, tunnel};

pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Server(command) => server::cli::run(command).await,
        Command::Tunnel(command) => tunnel::cli::run(command).await,
    }
}

#[derive(clap::Parser)]
#[command(name = "paron", version = env!("PARON_VERSION_STR"))]
pub struct Cli {
    /// Set the log level.
    #[arg(
        long,
        env = "PARON_LOG_LEVEL",
        global = true,
        default_value = "info",
        value_name = "LEVEL"
    )]
    pub log_level: LogLevel,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Start the Paron server
    #[command(subcommand, arg_required_else_help = true)]
    Server(server::cli::ServerCommand),

    /// Connect a machine to Paron
    #[command(subcommand, arg_required_else_help = true)]
    Tunnel(tunnel::cli::TunnelCommand),
}

impl Command {
    pub fn is_user_driven(&self) -> bool {
        !matches!(
            self,
            Self::Server(server::cli::ServerCommand::Run(_))
                | Self::Tunnel(tunnel::cli::TunnelCommand::Run(_))
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_tracing_filter(self) -> String {
        let level = match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        };

        let others = match self {
            Self::Trace => "trace",
            _ => "error",
        };

        format!("{others},paron={level},tower_http={level}")
    }
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct PromptArgs {
    /// Disable interactive prompts.
    #[arg(
        long,
        env = "PARON_NO_PROMPT",
        default_value_t = !std::io::IsTerminal::is_terminal(&std::io::stdin())
    )]
    pub no_prompt: bool,
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct ConfirmArgs {
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Copy, clap::Args, clap_extra::ArgNames)]
pub struct InstallArgs {
    /// Install a system daemon, which runs as a dedicated system account. Needs root access.
    #[arg(long = "with-system-daemon", env = "PARON_WITH_SYSTEM_DAEMON")]
    pub daemon: bool,
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    use clap::{Args as _, CommandFactory as _, Parser as _};
    use clap_extra::ArgName;

    use super::*;
    use crate::server::cli::{
        ServerBootstrapArgs, ServerDataArgs, ServerLockdownArgs, ServerRunArgs, ServerUserArgs,
    };
    #[cfg(unix)]
    use crate::tunnel::cli::{ServedArgs, TunnelDataArgs};
    use crate::tunnel::cli::{TunnelBootstrapArgs, TunnelEnrolArgs, TunnelRunArgs};

    #[cfg(unix)]
    fn tunnel_run_args() -> TunnelRunArgs {
        TunnelRunArgs {
            data: TunnelDataArgs {
                data_dir: PathBuf::from("/var/lib/paron-tunnel"),
            },
            served: ServedArgs {
                ports: Some(vec![22, 5432]),
                all_ports: false,
                #[cfg(unix)]
                sockets: None,
                #[cfg(unix)]
                all_sockets: true,
            },
        }
    }

    #[test]
    fn command_passes_clap_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn arg_names_match_what_clap_builds() {
        fn assert_names_match_arguments(command: &clap::Command, arg_names: &[ArgName]) {
            for name in arg_names {
                let argument = command.get_arguments().find(|argument| {
                    argument
                        .get_long()
                        .is_some_and(|long_name| format!("--{long_name}") == name.flag)
                });

                let Some(argument) = argument else {
                    panic!("`{}` has no {}", command.get_name(), name.flag);
                };

                assert_eq!(
                    argument.get_env().and_then(|env| env.to_str()),
                    name.env,
                    "{}",
                    name.flag
                );
            }
        }

        assert_names_match_arguments(
            &ServerRunArgs::augment_args(clap::Command::new("run")),
            &[
                ServerRunArgs::NAMES.data.data_dir,
                ServerRunArgs::NAMES.listen,
                ServerRunArgs::NAMES.tuning.max_connecting_agents,
                ServerRunArgs::NAMES.public.urls,
                ServerRunArgs::NAMES.certificate.letsencrypt,
                ServerRunArgs::NAMES.certificate.acme_directory,
                ServerRunArgs::NAMES.certificate.acme_contact,
                ServerRunArgs::NAMES.proxy.trusted_proxies,
                ServerRunArgs::NAMES.proxy.trusted_l7_proxies,
            ],
        );

        assert_names_match_arguments(
            &ServerBootstrapArgs::augment_args(clap::Command::new("bootstrap")),
            &[
                ServerBootstrapArgs::NAMES.data_dir,
                ServerBootstrapArgs::NAMES.listen,
                ServerBootstrapArgs::NAMES.public.urls,
                ServerBootstrapArgs::NAMES.install.daemon,
                ServerBootstrapArgs::NAMES.prompt.no_prompt,
            ],
        );

        assert_names_match_arguments(
            &ServerDataArgs::augment_args(clap::Command::new("list")),
            &[ServerDataArgs::NAMES.data_dir],
        );

        assert_names_match_arguments(
            &ServerLockdownArgs::augment_args(clap::Command::new("lockdown")),
            &[ServerLockdownArgs::NAMES.data.data_dir],
        );

        assert_names_match_arguments(
            &ServerUserArgs::augment_args(clap::Command::new("reset")),
            &[
                ServerUserArgs::NAMES.data.data_dir,
                ServerUserArgs::NAMES.password_stdin,
            ],
        );

        assert_names_match_arguments(
            &TunnelRunArgs::augment_args(clap::Command::new("run")),
            &[
                TunnelRunArgs::NAMES.data.data_dir,
                TunnelRunArgs::NAMES.served.ports,
                TunnelRunArgs::NAMES.served.all_ports,
                #[cfg(unix)]
                TunnelRunArgs::NAMES.served.sockets,
                #[cfg(unix)]
                TunnelRunArgs::NAMES.served.all_sockets,
            ],
        );

        assert_names_match_arguments(
            &TunnelBootstrapArgs::augment_args(clap::Command::new("bootstrap")),
            &[
                TunnelBootstrapArgs::NAMES.data_dir,
                TunnelBootstrapArgs::NAMES.token,
            ],
        );

        assert_names_match_arguments(
            &TunnelEnrolArgs::augment_args(clap::Command::new("enrol")),
            &[
                TunnelEnrolArgs::NAMES.data.data_dir,
                TunnelEnrolArgs::NAMES.token,
            ],
        );
    }

    #[cfg(unix)]
    #[test]
    fn flags_lists_only_the_set_arguments() {
        let run_args = tunnel_run_args();

        let flags = run_args.flags();

        assert_eq!(
            flags,
            [
                "--data",
                "/var/lib/paron-tunnel",
                "--ports",
                "22,5432",
                "--all-sockets"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn environment_lists_only_the_set_arguments() {
        let run_args = tunnel_run_args();

        let environment = run_args.environment();

        assert_eq!(
            environment,
            [
                (
                    "PARON_TUNNEL_DATA".to_owned(),
                    "/var/lib/paron-tunnel".to_owned()
                ),
                ("PARON_TUNNEL_PORTS".to_owned(), "22,5432".to_owned()),
                ("PARON_TUNNEL_ALL_SOCKETS".to_owned(), "true".to_owned()),
            ]
        );
    }

    #[test]
    fn every_argument_reads_an_environment_variable() {
        fn collect_arguments_without_env(
            command: &clap::Command,
            path: &str,
            arguments_without_env: &mut Vec<String>,
        ) {
            arguments_without_env.extend(
                command
                    .get_arguments()
                    .filter(|argument| argument.get_env().is_none())
                    .map(clap::Arg::get_id)
                    // `no_frontend_dev_server` exists only in a debug build. A service definition
                    // must not be able to set it. `yes` confirms one run, so it must be on the
                    // command line, not in an environment that every run shares.
                    .filter(|id| {
                        !matches!(
                            id.as_str(),
                            "help" | "version" | "no_frontend_dev_server" | "yes"
                        )
                    })
                    .map(|id| format!("{path} {id}")),
            );

            for subcommand in command.get_subcommands() {
                collect_arguments_without_env(
                    subcommand,
                    &format!("{path} {}", subcommand.get_name()),
                    arguments_without_env,
                );
            }
        }

        let mut arguments_without_env = Vec::new();
        collect_arguments_without_env(&Cli::command(), "paron", &mut arguments_without_env);

        assert!(
            arguments_without_env.is_empty(),
            "no environment variable for: {arguments_without_env:?}"
        );
    }

    #[test]
    fn parse_run_without_data_directory_returns_error() {
        for group in ["server", "tunnel"] {
            let Err(error) = Cli::try_parse_from(["paron", group, "run"]) else {
                panic!("{group} run: --data has no default");
            };

            assert!(error.to_string().contains("--data"), "{error}");
        }
    }

    #[test]
    fn parse_bootstrap_without_arguments_succeeds() {
        for group in ["server", "tunnel"] {
            if let Err(error) = Cli::try_parse_from(["paron", group, "bootstrap"]) {
                panic!("{group} bootstrap: {error}");
            }
        }
    }

    // tracing matches a directive against the start of the log target.
    #[test]
    fn to_tracing_filter_names_a_prefix_of_this_crate() {
        let tracing_filter = LogLevel::Info.to_tracing_filter();
        let directive = tracing_filter
            .split(',')
            .find_map(|directive| directive.split_once('='))
            .map(|(target, _)| target)
            .expect("the filter names at least one target");

        assert!(
            env!("CARGO_CRATE_NAME").starts_with(directive),
            "{} does not start with {directive}, so this crate is outside its own filter",
            env!("CARGO_CRATE_NAME")
        );
    }

    #[test]
    fn parse_log_level_on_either_side_of_subcommand_succeeds() {
        for arguments in [
            ["paron", "--log-level", "debug", "server", "bootstrap"],
            ["paron", "server", "bootstrap", "--log-level", "debug"],
        ] {
            if let Err(error) = Cli::try_parse_from(arguments) {
                panic!("{arguments:?}: {error}");
            }
        }
    }
}
