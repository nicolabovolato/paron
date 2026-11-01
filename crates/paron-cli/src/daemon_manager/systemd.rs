use std::path::PathBuf;

use askama::Template;
use tracing_subscriber::Layer;

use super::{Daemon, DaemonManager, LogSubscriber, SupervisedProcess};

#[derive(Template)]
#[template(path = "daemon_manager/systemd.service", escape = "none")]
struct UnitFile {
    description: String,
    settings: Vec<String>,
    command: String,
    user: String,
    privileged_ports: bool,
    writable: String,
    stop_seconds: u64,
}

pub struct Systemd;

impl DaemonManager for Systemd {
    fn definition_path(&self, daemon: &Daemon) -> PathBuf {
        PathBuf::from("/etc/systemd/system").join(format!("{name}.service", name = daemon.name))
    }

    fn has_definition(&self, daemon: &Daemon) -> bool {
        self.definition_path(daemon).exists()
    }

    fn install(&self, daemon: &Daemon, user: &str) -> crate::Result<()> {
        crate::platform::unix::files::install_file(
            &self.definition_path(daemon),
            &Self::render(daemon, user),
            0o644,
        )
    }

    fn start(&self, daemon: &Daemon) -> crate::Result<()> {
        crate::platform::unix::privileges::run_privileged("systemctl", &["daemon-reload"])?;
        crate::platform::unix::privileges::run_privileged("systemctl", &["enable", daemon.name])?;
        crate::platform::unix::privileges::run_privileged("systemctl", &["restart", daemon.name])
    }

    fn logs_follow_command(&self, daemon: &Daemon) -> Vec<String> {
        ["journalctl", "-u", daemon.name, "-f"]
            .map(str::to_owned)
            .to_vec()
    }
}

impl Systemd {
    fn render(daemon: &Daemon, user: &str) -> String {
        let mut out = UnitFile {
            description: daemon.description.to_owned(),
            settings: daemon
                .environment
                .iter()
                .map(|(name, value)| quoted(&format!("{name}={value}")))
                .collect(),
            command: daemon
                .command
                .iter()
                .map(|word| quoted(word))
                .collect::<Vec<_>>()
                .join(" "),
            user: user.to_owned(),
            privileged_ports: daemon.privileged_ports,
            writable: quoted(&daemon.data_dir.to_string_lossy()),
            stop_seconds: crate::lifecycle::STOP_TIMEOUT.as_secs(),
        }
        .to_string();

        out.push('\n');

        out
    }
}

fn quoted(word: &str) -> String {
    format!(
        "\"{escaped}\"",
        escaped = word
            .replace('\\', r"\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

pub struct Unit;

impl Unit {
    pub fn this_process() -> Option<Self> {
        std::env::var_os("JOURNAL_STREAM")
            .is_some_and(|journal_stream| {
                rustix::fs::fstat(std::io::stderr()).is_ok_and(|stderr| {
                    journal_stream.to_str()
                        == Some(&format!(
                            "{device}:{inode}",
                            device = stderr.st_dev,
                            inode = stderr.st_ino
                        ))
                })
            })
            .then_some(Self)
    }
}

impl SupervisedProcess for Unit {
    fn log_layer(&self) -> Option<Box<dyn Layer<LogSubscriber> + Send + Sync>> {
        tracing_journald::layer()
            .ok()
            .map(|layer| Box::new(layer) as Box<dyn Layer<LogSubscriber> + Send + Sync>)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Systemd;
    use crate::daemon_manager::testing::server_daemon;

    #[test]
    fn render_names_binary_and_every_argument() {
        let unit_file = Systemd::render(&server_daemon(), "root");

        assert!(
            unit_file.contains(r#"ExecStart="/usr/local/bin/paron" "server" "run""#),
            "{unit_file}"
        );
    }

    #[test]
    fn render_makes_only_the_data_directory_writable() {
        let unit_file = Systemd::render(&server_daemon(), "root");

        assert!(unit_file.contains("ProtectSystem=strict"), "{unit_file}");
        assert!(
            unit_file.contains(r#"ReadWritePaths="/var/lib/paron""#),
            "{unit_file}"
        );
    }

    #[test]
    fn render_path_with_special_characters_stays_one_escaped_argument() {
        for (path, escaped) in [
            ("/opt/paron tools/paron", r#""/opt/paron tools/paron""#),
            ("/opt/paron 100%/paron", r#""/opt/paron 100%%/paron""#),
            (r#"/opt/pa"ron"#, r#""/opt/pa\"ron""#),
            (r"/opt/pa\ron", r#""/opt/pa\\ron""#),
        ] {
            let mut daemon = server_daemon();
            daemon.command[0] = path.to_owned();
            daemon.data_dir = PathBuf::from(path);

            let unit_file = Systemd::render(&daemon, "root");

            assert!(
                unit_file.contains(&format!("ExecStart={escaped} ")),
                "{unit_file}"
            );
            assert!(
                unit_file.contains(&format!("ReadWritePaths={escaped}\n")),
                "{unit_file}"
            );
        }
    }

    #[test]
    fn render_names_account_that_owns_data_directory() {
        let unit_file = Systemd::render(&server_daemon(), "paron");

        assert!(unit_file.contains("\nUser=paron\n"), "{unit_file}");
    }

    #[test]
    fn render_privileged_ports_grants_bind_capability() {
        let unit_file = Systemd::render(&server_daemon(), "root");

        assert!(
            unit_file.contains("\nAmbientCapabilities=CAP_NET_BIND_SERVICE\n"),
            "{unit_file}"
        );
    }

    #[test]
    fn render_unprivileged_ports_withholds_bind_capability() {
        let mut daemon = server_daemon();
        daemon.privileged_ports = false;

        let unit_file = Systemd::render(&daemon, "root");

        assert!(!unit_file.contains("AmbientCapabilities"), "{unit_file}");
    }
}
