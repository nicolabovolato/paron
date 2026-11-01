use std::path::PathBuf;

use askama::Template;
use tracing_subscriber::Layer;

use super::{Daemon, DaemonManager, LogSubscriber, SupervisedProcess};

#[derive(Template)]
#[template(path = "daemon_manager/launchd.plist", escape = "html")]
struct Plist {
    label: String,
    user: String,
    program_arguments: Vec<String>,
    settings: Vec<(String, String)>,
    stop_seconds: u64,
}

pub struct Launchd;

impl DaemonManager for Launchd {
    fn definition_path(&self, daemon: &Daemon) -> PathBuf {
        PathBuf::from("/Library/LaunchDaemons").join(format!("{label}.plist", label = daemon.name))
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
        let job = format!("system/{label}", label = daemon.name);

        let is_loaded = || {
            std::process::Command::new("launchctl")
                .args(["print", &job])
                .output()
                .is_ok_and(|ran| ran.status.success())
        };

        if is_loaded() {
            crate::platform::unix::privileges::run_privileged("launchctl", &["bootout", &job])?;

            // `bootout` returns before the job has stopped, and a `bootstrap` before that fails.
            // launchd kills the job when the `ExitTimeOut` of the loaded plist ends.
            while is_loaded() {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }

        crate::platform::unix::privileges::run_privileged(
            "launchctl",
            &[
                "bootstrap",
                "system",
                &self.definition_path(daemon).to_string_lossy(),
            ],
        )
    }

    fn logs_follow_command(&self, daemon: &Daemon) -> Vec<String> {
        vec![
            "log".to_owned(),
            "stream".to_owned(),
            "--predicate".to_owned(),
            format!(r#"subsystem == "{label}""#, label = daemon.name),
        ]
    }
}

impl Launchd {
    fn render(daemon: &Daemon, user: &str) -> String {
        let mut out = Plist {
            label: daemon.name.to_owned(),
            user: user.to_owned(),
            program_arguments: daemon.command.clone(),
            settings: daemon.environment.clone(),
            stop_seconds: crate::lifecycle::STOP_TIMEOUT.as_secs(),
        }
        .to_string();

        out.push('\n');

        out
    }
}

pub struct Job {
    label: String,
}

impl Job {
    /// launchd sets `$XPC_SERVICE_NAME` to the label of the job, and gives `/dev/null` as stderr to
    /// a job with no `StandardErrorPath`. A shell in Terminal has `$XPC_SERVICE_NAME` set to `0`.
    pub fn this_process() -> Option<Self> {
        let stderr = rustix::fs::fstat(std::io::stderr()).ok()?;
        let dev_null = rustix::fs::stat("/dev/null").ok()?;

        ((stderr.st_dev, stderr.st_ino) == (dev_null.st_dev, dev_null.st_ino))
            .then(|| std::env::var("XPC_SERVICE_NAME").ok())
            .flatten()
            .filter(|label| label != "0")
            .map(|label| Self { label })
    }
}

impl SupervisedProcess for Job {
    fn log_layer(&self) -> Option<Box<dyn Layer<LogSubscriber> + Send + Sync>> {
        Some(Box::new(tracing_oslog::OsLogger::new(&self.label, "")))
    }
}

#[cfg(test)]
mod tests {
    use super::Launchd;
    use crate::daemon_manager::testing::server_daemon;

    #[test]
    fn render_names_binary_and_every_argument_in_order() {
        let plist = Launchd::render(&server_daemon(), "root");
        let array = plist
            .split_once("<key>ProgramArguments</key>")
            .and_then(|(_, rest)| rest.split_once("</array>"))
            .expect("a ProgramArguments array")
            .0;

        let arguments: Vec<_> = array
            .lines()
            .filter_map(|line| line.trim().strip_prefix("<string>"))
            .filter_map(|line| line.strip_suffix("</string>"))
            .collect();

        assert_eq!(arguments, ["/usr/local/bin/paron", "server", "run"]);
    }

    #[test]
    fn render_carries_settings_inline() {
        let plist = Launchd::render(&server_daemon(), "root");

        assert!(plist.contains("<key>EnvironmentVariables</key>"), "{plist}");
        assert!(plist.contains("<key>PARON_LISTEN</key>"), "{plist}");
        assert!(plist.contains("<string>127.0.0.1:8443</string>"), "{plist}");
    }

    #[test]
    fn render_escapes_what_xml_reserves() {
        let mut daemon = server_daemon();
        daemon.command.push("a&b<c>d\"e'f".to_owned());

        let plist = Launchd::render(&daemon, "root");

        assert!(plist.contains("a&#38;b&#60;c&#62;d&#34;e&#39;f"), "{plist}");
        assert!(!plist.contains("a&b"), "{plist}");
    }

    #[test]
    fn render_is_accepted_by_plutil() {
        let data_dir = crate::testing::TempDataDir::new("plist");
        let plist_path = data_dir.join("definition.plist");
        std::fs::write(&plist_path, Launchd::render(&server_daemon(), "root")).unwrap();

        let plutil_output = std::process::Command::new("plutil")
            .arg("-lint")
            .arg(&plist_path)
            .output()
            .expect("plutil is part of macOS");

        assert!(plutil_output.status.success(), "{plutil_output:?}");
    }
}
