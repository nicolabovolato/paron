use std::ffi::{OsStr, OsString};
use std::io;
use std::iter::once;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
    ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_MULTI_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegSetValueExW,
};
use windows_sys::Win32::System::Services::{
    QueryServiceStatusEx, SC_STATUS_PROCESS_INFO, SERVICE_STATUS_PROCESS,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: i32 = 1061;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;

/// What the service runs, as whom, and with which environment
pub struct Definition<'a> {
    pub name: &'a str,
    pub display_name: &'a str,
    pub executable: &'a Path,
    pub arguments: &'a [String],
    pub account: &'a str,
    pub environment: &'a [(String, String)],
}

pub fn is_installed(name: &str) -> bool {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|manager| manager.open_service(name, ServiceAccess::QUERY_STATUS))
        .is_ok()
}

/// Creates the service, or changes an existing one to the definition. It starts with the system,
/// and restarts 5 seconds after a failure.
pub fn install(definition: &Definition) -> io::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(io_error)?;

    let info = ServiceInfo {
        name: definition.name.into(),
        display_name: definition.display_name.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: definition.executable.to_path_buf(),
        launch_arguments: definition.arguments.iter().map(OsString::from).collect(),
        dependencies: Vec::new(),
        account_name: Some(definition.account.into()),
        // A virtual account has no password.
        account_password: None,
    };
    // A restart action needs the right to start the service.
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG | ServiceAccess::START;

    let service = match manager.open_service(definition.name, access) {
        Ok(service) => {
            service.change_config(&info).map_err(io_error)?;

            service
        }
        Err(_) => manager.create_service(&info, access).map_err(io_error)?,
    };

    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(5),
                };
                3
            ]),
        })
        .map_err(io_error)?;

    // A failure that the service reports as a stop with an exit code, and not only a crash, runs
    // the actions.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(io_error)?;

    set_environment(definition.name, definition.environment)
}

/// Starts the service. A service that runs is stopped first, so that it starts with its current
/// definition. Its process is killed when it does not stop within the wait hint that it reports.
pub fn restart(name: &str) -> io::Result<()> {
    let service = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|manager| {
            manager.open_service(
                name,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START,
            )
        })
        .map_err(io_error)?;

    let mut stopping = Instant::now();
    let mut killed = false;

    loop {
        let status = service.query_status().map_err(io_error)?;

        if status.current_state == ServiceState::Stopped {
            break;
        }

        // A service that is starting cannot take a stop, so it gets one when it runs.
        if status.current_state == ServiceState::Running && stop(&service)? {
            stopping = Instant::now();

            continue;
        }

        if !killed && stopping.elapsed() > status.wait_hint {
            let process_id = process_id(&service)?;

            // A service that is starting has no process id yet.
            if process_id != 0 {
                kill(process_id)?;
                killed = true;
            }
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    service.start(&[] as &[&OsStr]).map_err(io_error)
}

/// False when the service cannot take the stop now.
fn stop(service: &windows_service::service::Service) -> io::Result<bool> {
    match service.stop() {
        Ok(_) => Ok(true),
        Err(windows_service::Error::Winapi(error))
            if matches!(
                error.raw_os_error(),
                Some(ERROR_SERVICE_CANNOT_ACCEPT_CTRL | ERROR_SERVICE_NOT_ACTIVE)
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(io_error(error)),
    }
}

/// `windows-service` gives the process id only for a running service.
fn process_id(service: &windows_service::service::Service) -> io::Result<u32> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let size = u32::try_from(size_of::<SERVICE_STATUS_PROCESS>()).map_err(io::Error::other)?;
    let mut needed = 0;

    let queried = unsafe {
        QueryServiceStatusEx(
            service.raw_handle(),
            SC_STATUS_PROCESS_INFO,
            (&raw mut status).cast(),
            size,
            &raw mut needed,
        )
    };

    if queried == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(status.dwProcessId)
}

fn kill(process_id: u32) -> io::Result<()> {
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, process_id) };

    if process.is_null() {
        let error = io::Error::last_os_error();

        // No process has that id: it exited after the status was read.
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
            return Ok(());
        }

        return Err(error);
    }

    let terminated = unsafe { TerminateProcess(process, 1) };
    let error = io::Error::last_os_error();

    unsafe { CloseHandle(process) };

    if terminated == 0 {
        return Err(error);
    }

    Ok(())
}

/// The service control manager gives each process of the service the strings of the
/// `Environment` value of its key, one `NAME=value` in each.
fn set_environment(name: &str, environment: &[(String, String)]) -> io::Result<()> {
    if environment
        .iter()
        .any(|(variable, value)| variable.contains('\0') || value.contains('\0'))
    {
        return Err(io::Error::other("an environment variable contains a NUL"));
    }

    let path = wide(&format!(r"SYSTEM\CurrentControlSet\Services\{name}"));
    let value_name = wide("Environment");
    let mut key: HKEY = std::ptr::null_mut();

    win32(unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_SET_VALUE,
            &raw mut key,
        )
    })?;

    let written = if environment.is_empty() {
        match unsafe { RegDeleteValueW(key, value_name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            status => status,
        }
    } else {
        // Each string ends with a NUL, and an empty string ends the list.
        let strings: Vec<u16> = environment
            .iter()
            .flat_map(|(variable, value)| {
                format!("{variable}={value}")
                    .encode_utf16()
                    .chain(once(0))
                    .collect::<Vec<_>>()
            })
            .chain(once(0))
            .collect();
        let size = u32::try_from(strings.len() * 2).map_err(io::Error::other)?;

        unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_MULTI_SZ,
                strings.as_ptr().cast(),
                size,
            )
        }
    };

    unsafe { RegCloseKey(key) };

    win32(written)
}

/// The error of Windows itself, which the error of `windows-service` shows only as its source
fn io_error(error: windows_service::Error) -> io::Error {
    match error {
        windows_service::Error::Winapi(error) => error,
        error => io::Error::other(error),
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(once(0)).collect()
}

fn win32(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        return Ok(());
    }

    Err(io::Error::from_raw_os_error(status.cast_signed()))
}
