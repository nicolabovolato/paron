#[cfg(unix)]
use std::os::fd::AsFd as Socket;
#[cfg(windows)]
use std::os::windows::io::AsSocket as Socket;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcceptFailure {
    /// The connection failed
    BadConnection,

    /// No room to accept the connection
    OutOfOpenFiles,

    /// Fatal and unrecoverable
    BadListener,

    /// Unrecognised error, back off before retrying
    Unknown,
}

/// Accepts and immediately closes every connection already queued on `listener`, and returns how
/// many there were.
pub fn close_queued_connections(listener: impl Socket) -> usize {
    let mut closed = 0;

    // Connections arriving during the drain queue up behind the ones being closed already.
    // Dropping a connection costs orders of magnitude less than initiating a TCP handshake.
    while let Ok(refused) = rustix::net::accept(&listener) {
        drop(refused);

        closed += 1;
    }

    closed
}

pub fn accept_failure(error: &std::io::Error) -> AcceptFailure {
    use rustix::io::Errno;

    let Some(errno) = Errno::from_io_error(error) else {
        return AcceptFailure::Unknown;
    };

    #[cfg(target_os = "linux")]
    if errno == Errno::NONET {
        return AcceptFailure::BadConnection;
    }

    #[cfg(unix)]
    match errno {
        Errno::PROTO => return AcceptFailure::BadConnection,
        Errno::NFILE => return AcceptFailure::OutOfOpenFiles,
        _ => {}
    }

    #[cfg(windows)]
    match errno {
        Errno::CONNRESET => return AcceptFailure::BadConnection,
        Errno::NOBUFS => return AcceptFailure::OutOfOpenFiles,
        _ => {}
    }

    match errno {
        Errno::INTR
        | Errno::CONNABORTED
        | Errno::NETDOWN
        | Errno::NETUNREACH
        | Errno::NOPROTOOPT
        | Errno::HOSTDOWN
        | Errno::HOSTUNREACH
        | Errno::OPNOTSUPP => AcceptFailure::BadConnection,
        Errno::MFILE => AcceptFailure::OutOfOpenFiles,
        Errno::BADF | Errno::INVAL | Errno::NOTSOCK => AcceptFailure::BadListener,
        _ => AcceptFailure::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use rustix::io::Errno;

    use super::*;

    fn io_error(errno: Errno) -> std::io::Error {
        std::io::Error::from_raw_os_error(errno.raw_os_error())
    }

    #[test]
    fn accept_failure_separates_what_the_listener_acts_on() {
        #[cfg(unix)]
        let (platform_bad_connection, platform_out_of_files) = (Errno::PROTO, Errno::NFILE);
        #[cfg(windows)]
        let (platform_bad_connection, platform_out_of_files) = (Errno::CONNRESET, Errno::NOBUFS);

        for bad_connection_errno in [
            Errno::CONNABORTED,
            Errno::INTR,
            Errno::HOSTUNREACH,
            platform_bad_connection,
        ] {
            assert_eq!(
                accept_failure(&io_error(bad_connection_errno)),
                AcceptFailure::BadConnection
            );
        }

        for out_of_files_errno in [Errno::MFILE, platform_out_of_files] {
            assert_eq!(
                accept_failure(&io_error(out_of_files_errno)),
                AcceptFailure::OutOfOpenFiles
            );
        }

        for bad_listener_errno in [Errno::BADF, Errno::INVAL, Errno::NOTSOCK] {
            assert_eq!(
                accept_failure(&io_error(bad_listener_errno)),
                AcceptFailure::BadListener
            );
        }
    }

    #[test]
    fn accept_failure_without_a_code_returns_unknown() {
        let error_without_errno = std::io::Error::other("no errno behind this one");

        assert_eq!(accept_failure(&error_without_errno), AcceptFailure::Unknown);
    }
}
