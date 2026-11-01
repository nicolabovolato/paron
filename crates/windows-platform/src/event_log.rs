use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt as _;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
    RegisterEventSourceW, ReportEventW,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Error,
    Warning,
    Information,
}

/// An event source in the Application log. The registered message file decides how Event Viewer
/// shows the text: with `EventLogMessages.dll`, event 0 shows the text as it is.
pub struct EventLog(HANDLE);

// Windows allows the use of one event source handle from many threads.
unsafe impl Send for EventLog {}
unsafe impl Sync for EventLog {}

impl EventLog {
    pub fn new(source: &str) -> std::io::Result<Self> {
        let source = wide(source);
        let handle = unsafe { RegisterEventSourceW(std::ptr::null(), source.as_ptr()) };

        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self(handle))
    }

    pub fn write(&self, level: Level, text: &str) -> std::io::Result<()> {
        let event_type = match level {
            Level::Error => EVENTLOG_ERROR_TYPE,
            Level::Warning => EVENTLOG_WARNING_TYPE,
            Level::Information => EVENTLOG_INFORMATION_TYPE,
        };
        let text = wide(text);
        let strings = [text.as_ptr()];

        let written = unsafe {
            ReportEventW(
                self.0,
                event_type,
                0,
                0,
                std::ptr::null_mut(),
                1,
                0,
                strings.as_ptr(),
                std::ptr::null(),
            )
        };

        if written == 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        unsafe { DeregisterEventSource(self.0) };
    }
}

/// The text as a NUL-terminated wide string, without the NUL characters that it has itself
fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .filter(|unit| *unit != 0)
        .chain(std::iter::once(0))
        .collect()
}
