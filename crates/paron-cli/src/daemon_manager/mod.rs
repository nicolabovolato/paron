use std::path::PathBuf;

use tracing_subscriber::layer::Layered;
use tracing_subscriber::{EnvFilter, Layer, Registry};

#[cfg(target_os = "macos")]
pub mod launchd;
#[cfg(target_os = "linux")]
pub mod systemd;
#[cfg(windows)]
pub mod windows_service;

pub struct Daemon {
    pub name: &'static str,
    pub description: &'static str,

    pub command: Vec<String>,
    pub environment: Vec<(String, String)>,

    pub privileged_ports: bool,

    pub data_dir: PathBuf,
}

#[cfg(unix)]
pub trait DaemonManager {
    /// The file or directory that holds the definition
    fn definition_path(&self, daemon: &Daemon) -> PathBuf;

    /// True when a definition exists, which `install` replaces
    fn has_definition(&self, daemon: &Daemon) -> bool;

    /// Writes the definition of the daemon, which runs as `user`
    fn install(&self, daemon: &Daemon, user: &str) -> crate::Result<()>;

    /// Starts the daemon now, and with each start of the system. A daemon that runs restarts with
    /// the definition that `install` wrote.
    fn start(&self, daemon: &Daemon) -> crate::Result<()>;

    fn logs_follow_command(&self, daemon: &Daemon) -> Vec<String>;
}

pub trait SupervisedProcess {
    /// The layer that sends the log to the daemon manager.
    fn log_layer(&self) -> Option<Box<dyn Layer<LogSubscriber> + Send + Sync>>;

    /// Reports that Paron stopped
    fn stopped(self: Box<Self>, _exit_code: u8) {}
}

/// The subscriber that `main` puts the log layer on: the registry with the level filter
pub type LogSubscriber = Layered<EnvFilter, Registry>;

pub fn supervised_process() -> crate::Result<Option<Box<dyn SupervisedProcess>>> {
    cfg_select! {
        target_os = "linux" => { Ok(systemd::Unit::this_process().map(|unit| Box::new(unit) as _)) }
        target_os = "macos" => { Ok(launchd::Job::this_process().map(|job| Box::new(job) as _)) }
        windows => { Ok(windows_service::Service::connect()?.map(|service| Box::new(service) as _)) }
        _ => { Ok(None) }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod testing {
    use super::{Daemon, PathBuf};

    pub fn server_daemon() -> Daemon {
        Daemon {
            name: "paron-server",
            description: "Paron server",
            command: ["/usr/local/bin/paron", "server", "run"]
                .map(str::to_owned)
                .to_vec(),
            environment: vec![("PARON_LISTEN".to_owned(), "127.0.0.1:8443".to_owned())],
            privileged_ports: true,
            data_dir: PathBuf::from("/var/lib/paron"),
        }
    }
}
