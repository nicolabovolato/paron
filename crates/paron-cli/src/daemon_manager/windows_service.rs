use tracing_subscriber::Layer;

use super::{Daemon, LogSubscriber, SupervisedProcess};

pub fn account(daemon: &Daemon) -> String {
    format!(r"NT SERVICE\{name}", name = daemon.name)
}

pub fn has_definition(daemon: &Daemon) -> bool {
    windows_platform::services::is_installed(daemon.name)
}

pub fn install(daemon: &Daemon) -> crate::Result<()> {
    let user = &account(daemon);
    let (executable, arguments) = daemon
        .command
        .split_first()
        .ok_or("the daemon has no command")?;

    windows_platform::services::install(&windows_platform::services::Definition {
        name: daemon.name,
        display_name: daemon.description,
        executable: std::path::Path::new(executable),
        arguments,
        account: user,
        environment: &daemon.environment,
    })
    .map_err(|reason| {
        format!(
            "could not install the service {name}: {reason}",
            name = daemon.name
        )
    })?;

    crate::platform::windows::files::grant_full_access(&daemon.data_dir, user)?;

    register_event_source(daemon.name)
}

/// Starts the service now, and with each start of the system. A service that runs restarts with
/// the definition that `install` wrote.
pub fn start(daemon: &Daemon) -> crate::Result<()> {
    windows_platform::services::restart(daemon.name)
        .map_err(|reason| format!("could not start {name}: {reason}", name = daemon.name).into())
}

/// The event source has the name of the service. `New-EventLog` registers it with the message
/// file of .NET Framework 4 for the machine, which shows the text of each event as it is.
fn register_event_source(name: &str) -> crate::Result<()> {
    let ran = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "if (-not [System.Diagnostics.EventLog]::SourceExists('{name}')) \
                 {{ New-EventLog -LogName Application -Source '{name}' }}"
            ),
        ])
        .output()
        .map_err(|reason| format!("powershell.exe could not be run: {reason}"))?;

    if !ran.status.success() {
        return Err(format!(
            "could not register the event source {name}: {stderr}",
            stderr = String::from_utf8_lossy(&ran.stderr).trim()
        )
        .into());
    }

    Ok(())
}

pub fn logs_command(daemon: &Daemon) -> Vec<String> {
    [
        "Get-WinEvent",
        "-ProviderName",
        daemon.name,
        "-MaxEvents",
        "50",
    ]
    .map(str::to_owned)
    .to_vec()
}

pub struct Service(windows_platform::Service);

impl Service {
    pub fn connect() -> crate::Result<Option<Self>> {
        windows_platform::connect(
            crate::lifecycle::STOP_TIMEOUT,
            crate::platform::signals::request_service_stop,
        )
        .map(|service| service.map(Self))
        .map_err(|reason| {
            format!("could not connect to the service control manager: {reason}").into()
        })
    }
}

impl SupervisedProcess for Service {
    fn log_layer(&self) -> Option<Box<dyn Layer<LogSubscriber> + Send + Sync>> {
        let source = self.0.name().to_string_lossy();
        let event_log = windows_platform::event_log::EventLog::new(&source).ok()?;

        Some(Box::new(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_level(false)
                .with_ansi(false)
                .with_writer(EventLogWriter(event_log)),
        ))
    }

    fn stopped(self: Box<Self>, exit_code: u8) {
        self.0.stopped(u32::from(exit_code));
    }
}

struct EventLogWriter(windows_platform::event_log::EventLog);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for EventLogWriter {
    type Writer = EventLogEntry<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        EventLogEntry {
            event_log: &self.0,
            level: windows_platform::event_log::Level::Information,
            text: Vec::new(),
        }
    }

    fn make_writer_for(&'a self, metadata: &tracing::Metadata<'_>) -> Self::Writer {
        use windows_platform::event_log::Level;

        let level = match *metadata.level() {
            tracing::Level::ERROR => Level::Error,
            tracing::Level::WARN => Level::Warning,
            _ => Level::Information,
        };

        EventLogEntry {
            event_log: &self.0,
            level,
            text: Vec::new(),
        }
    }
}

struct EventLogEntry<'a> {
    event_log: &'a windows_platform::event_log::EventLog,
    level: windows_platform::event_log::Level,
    text: Vec<u8>,
}

impl std::io::Write for EventLogEntry<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.text.extend_from_slice(bytes);

        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventLogEntry<'_> {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.text);

        // Nothing can report a failure of the log itself.
        let _ = self.event_log.write(self.level, text.trim_end());
    }
}
