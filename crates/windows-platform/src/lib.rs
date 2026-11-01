#![cfg(windows)]

pub mod event_log;
pub mod security;
pub mod services;

use std::ffi::{OsStr, OsString};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::{define_windows_service, service_dispatcher};

/// What the dispatcher returns when the service control manager did not start the process
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;

struct Stop {
    within: Duration,
    requested: Box<dyn Fn() + Send + Sync>,
}

static STARTED: OnceLock<mpsc::SyncSender<io::Result<Option<Service>>>> = OnceLock::new();
static STOP: OnceLock<Stop> = OnceLock::new();
static STATUS: OnceLock<ServiceStatusHandle> = OnceLock::new();
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

define_windows_service!(ffi_service_main, service_main);

/// The connection to the service control manager
pub struct Service {
    name: OsString,
    status: ServiceStatusHandle,
}

/// Connects to the service control manager, and returns when it has started the service.
///
/// `Ok(None)` when the service control manager did not start this process. An error when it did,
/// but the connection failed: the service control manager then cannot stop the process.
///
/// The service control manager stops a service that does not connect within 30 seconds, so call
/// this first, and once. On a stop or a shutdown, `stop_requested` runs, and the service reports
/// that it stops within `stop_within`.
pub fn connect(
    stop_within: Duration,
    stop_requested: impl Fn() + Send + Sync + 'static,
) -> io::Result<Option<Service>> {
    let (started, connected) = mpsc::sync_channel(1);

    let stop = Stop {
        within: stop_within,
        requested: Box::new(stop_requested),
    };

    if STARTED.set(started).is_err() || STOP.set(stop).is_err() {
        return Err(io::Error::other(
            "already connected to the service control manager",
        ));
    }

    std::thread::spawn(|| {
        // A service in its own process can have an empty name here. This returns at once when the
        // service control manager did not start the process, and else when the service stops.
        match service_dispatcher::start("", ffi_service_main) {
            Ok(()) => {}
            Err(windows_service::Error::Winapi(error))
                if error.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
            {
                send(Ok(None));
            }
            Err(error) => send(Err(io::Error::other(error))),
        }
    });

    connected
        .recv()
        .map_err(|_| io::Error::other("the service dispatcher stopped before it started"))?
}

impl Service {
    /// The service control manager gives the name of the service as the first argument.
    pub fn name(&self) -> &OsStr {
        &self.name
    }

    /// Reports the service as stopped. A non-zero exit code is a failure, for which the service
    /// control manager runs the recovery actions of the service.
    ///
    /// After a stop that the service control manager asked for, the report is always a success:
    /// the recovery actions would otherwise start the service that it stopped.
    pub fn stopped(self, exit_code: u32) {
        let exit_code = if exit_code == 0 || STOP_REQUESTED.load(Ordering::Relaxed) {
            ServiceExitCode::NO_ERROR
        } else {
            ServiceExitCode::ServiceSpecific(exit_code)
        };

        report(
            self.status,
            ServiceState::Stopped,
            exit_code,
            Duration::ZERO,
        );
    }
}

fn service_main(arguments: Vec<OsString>) {
    send(started(arguments).map(Some));
}

fn started(arguments: Vec<OsString>) -> io::Result<Service> {
    let name = arguments
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("the service control manager gave no service name"))?;

    let status = service_control_handler::register(&name, |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            STOP_REQUESTED.store(true, Ordering::Relaxed);

            if let (Some(stop), Some(status)) = (STOP.get(), STATUS.get()) {
                report(
                    *status,
                    ServiceState::StopPending,
                    ServiceExitCode::NO_ERROR,
                    stop.within,
                );
                (stop.requested)();
            }

            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(io::Error::other)?;

    STATUS
        .set(status)
        .map_err(|_| io::Error::other("the service started twice"))?;

    report(
        status,
        ServiceState::Running,
        ServiceExitCode::NO_ERROR,
        Duration::ZERO,
    );

    Ok(Service { name, status })
}

fn send(started: io::Result<Option<Service>>) {
    if let Some(sender) = STARTED.get() {
        let _ = sender.send(started);
    }
}

fn report(
    status: ServiceStatusHandle,
    state: ServiceState,
    exit_code: ServiceExitCode,
    wait_hint: Duration,
) {
    let controls_accepted = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };

    let _ = status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    });
}
