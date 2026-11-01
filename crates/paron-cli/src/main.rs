use std::io::IsTerminal as _;
use std::process::ExitCode;

use clap::Parser as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use paron_cli::Result;
use paron_cli::cli::{self, Cli, Command, LogLevel};
use paron_cli::daemon_manager::{self, SupervisedProcess};
use paron_cli::lifecycle::Stopped;

#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    #[cfg(unix)]
    paron_cli::platform::unix::files::restrict_new_files_to_owner();

    #[expect(
        clippy::expect_used,
        reason = "no code installs a provider before main"
    )]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("could not install crypto provider");

    // Print the banner if no command was given.
    if std::env::args_os().nth(1).is_none() {
        paron_cli::ui::print_banner();
    }

    let cli = Cli::parse();
    let is_user_driven = cli.command.is_user_driven();

    let supervised_process = match daemon_manager::supervised_process() {
        Ok(supervised_process) => supervised_process,
        Err(reason) => {
            eprintln!("error: {reason}");

            return ExitCode::FAILURE;
        }
    };

    setup_logging(cli.log_level, supervised_process.as_deref());

    if !is_user_driven {
        std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));
    }

    let exit_code = match run(cli.command) {
        Ok(()) => 0,
        Err(reason) => match reason.downcast_ref::<Stopped>() {
            Some(Stopped::Interrupted) => 20,
            Some(Stopped::TimedOut) => 30,
            None => {
                if is_user_driven {
                    eprintln!("error: {reason}");
                } else {
                    tracing::error!("{reason}");
                }

                1
            }
        },
    };

    if let Some(supervised_process) = supervised_process {
        supervised_process.stopped(exit_code);
    }

    ExitCode::from(exit_code)
}

fn setup_logging(log_level: LogLevel, supervised_process: Option<&dyn SupervisedProcess>) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_level.to_tracing_filter().into()),
        )
        .with(
            supervised_process
                .and_then(SupervisedProcess::log_layer)
                .unwrap_or_else(|| {
                    Box::new(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(std::io::stderr().is_terminal())
                            .with_writer(std::io::stderr),
                    )
                }),
        )
        .init();
}

fn run(command: Command) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let outcome = runtime.block_on(cli::run(command));

    runtime.shutdown_background();

    outcome
}
