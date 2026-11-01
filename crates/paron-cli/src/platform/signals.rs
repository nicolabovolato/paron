#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Signal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
    #[cfg(windows)]
    Break,
    #[cfg(windows)]
    ConsoleClose,
    #[cfg(windows)]
    Shutdown,
    #[cfg(windows)]
    ServiceStop,
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            #[cfg(unix)]
            Self::Interrupt => "SIGINT",
            #[cfg(unix)]
            Self::Terminate => "SIGTERM",
            #[cfg(windows)]
            Self::Interrupt => "Ctrl+C",
            #[cfg(windows)]
            Self::Break => "Ctrl+Break",
            #[cfg(windows)]
            Self::ConsoleClose => "console close",
            #[cfg(windows)]
            Self::Shutdown => "system shutdown",
            #[cfg(windows)]
            Self::ServiceStop => "service stop",
        })
    }
}

/// Resolves on SIGINT or SIGTERM. On Windows, resolves on Ctrl+C, Ctrl+Break, the close of the
/// console, a shutdown, and a stop from the service control manager.
pub async fn stop_requested() -> std::io::Result<Signal> {
    tokio::select! {
        interrupted = tokio::signal::ctrl_c() => interrupted.map(|()| Signal::Interrupt),
        terminated = terminate() => terminated,
    }
}

async fn terminate() -> std::io::Result<Signal> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        signal(SignalKind::terminate())?.recv().await;

        Ok(Signal::Terminate)
    }

    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_close, ctrl_shutdown};

        let mut broken = ctrl_break()?;
        let mut closed = ctrl_close()?;
        let mut shut_down = ctrl_shutdown()?;

        Ok(tokio::select! {
            _ = broken.recv() => Signal::Break,
            _ = closed.recv() => Signal::ConsoleClose,
            _ = shut_down.recv() => Signal::Shutdown,
            () = SERVICE_STOPS.notified() => Signal::ServiceStop,
        })
    }
}

/// The service control manager sends no signal: it calls the handler of the service on a thread
/// of its own. A stop that comes before anyone waits is kept.
#[cfg(windows)]
static SERVICE_STOPS: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[cfg(windows)]
pub fn request_service_stop() {
    SERVICE_STOPS.notify_one();
}
