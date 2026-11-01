#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;

const AVAHI_SOCKET: &str = "/run/avahi-daemon/socket";
const HOSTS_FILE: &str = "/etc/hosts";

#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    Address(Ipv4Addr),
    NotFound,
    System,
}

#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Address(Ipv4Addr),
    NotFound,
    Unavailable,
}

// What `hosts: files mdns4_minimal [NOTFOUND=return] dns` does with glibc and libnss-mdns.
pub fn resolve(host: &str, has_local_soa: impl FnOnce() -> bool) -> Resolution {
    let hosts = std::fs::read_to_string(HOSTS_FILE).unwrap_or_default();

    if !is_mdns_name(host) || names_host(&hosts, host) {
        return Resolution::System;
    }

    match answer_from_avahi(host) {
        Answer::Address(address) => Resolution::Address(address),
        // With an SOA record for `local`, the DNS has a zone of that name, and it gets its turn.
        Answer::NotFound if !has_local_soa() => Resolution::NotFound,
        Answer::NotFound | Answer::Unavailable => Resolution::System,
    }
}

fn is_mdns_name(host: &str) -> bool {
    let labels = host.strip_suffix('.').unwrap_or(host).matches('.').count() + 1;
    let lowercase = host.to_ascii_lowercase();

    labels == 2
        && (lowercase.ends_with(".local") || lowercase.ends_with(".local."))
        // The request to Avahi is one line.
        && !host
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

fn names_host(hosts: &str, host: &str) -> bool {
    hosts.lines().any(|line| {
        line.split('#')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .skip(1)
            .any(|name| name.eq_ignore_ascii_case(host))
    })
}

fn answer_from_avahi(host: &str) -> Answer {
    let Ok(mut avahi) = UnixStream::connect(AVAHI_SOCKET) else {
        return Answer::Unavailable;
    };

    if writeln!(avahi, "RESOLVE-HOSTNAME-IPV4 {host}").is_err() {
        return Answer::Unavailable;
    }

    let mut answer = String::new();

    match BufReader::new(avahi).read_line(&mut answer) {
        Ok(length) if length > 0 => answer_in(&answer),
        _ => Answer::Unavailable,
    }
}

// A found answer is `+ <interface> <protocol> <host> <address>`.
fn answer_in(line: &str) -> Answer {
    let Some(found) = line.strip_prefix('+') else {
        return Answer::NotFound;
    };

    found
        .split_whitespace()
        .nth(3)
        .and_then(|address| address.parse().ok())
        .map_or(Answer::Unavailable, Answer::Address)
}

// The binary links with `--wrap=getaddrinfo`, so each call to `getaddrinfo` comes here and
// `__real_getaddrinfo` is the one of libc.
#[cfg(target_env = "musl")]
mod wrapped {
    use std::ffi::{CStr, CString, c_char, c_int};

    use libc::addrinfo;

    use super::Resolution;

    const CLASS_IN: c_int = 1;
    const TYPE_SOA: c_int = 6;
    const SOA_ANSWER_BYTES: c_int = 512;

    unsafe extern "C" {
        fn __real_getaddrinfo(
            node: *const c_char,
            service: *const c_char,
            hints: *const addrinfo,
            result: *mut *mut addrinfo,
        ) -> c_int;

        fn res_query(
            name: *const c_char,
            class: c_int,
            record_type: c_int,
            answer: *mut u8,
            answer_bytes: c_int,
        ) -> c_int;
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn __wrap_getaddrinfo(
        node: *const c_char,
        service: *const c_char,
        hints: *const addrinfo,
        result: *mut *mut addrinfo,
    ) -> c_int {
        // SAFETY: a caller of getaddrinfo gives valid hints or null.
        let (flags, family) = unsafe { hints.as_ref() }.map_or((0, libc::AF_UNSPEC), |hints| {
            (hints.ai_flags, hints.ai_family)
        });

        let asks_for_a_name_over_ipv4 = !node.is_null()
            && flags & libc::AI_NUMERICHOST == 0
            && matches!(family, libc::AF_UNSPEC | libc::AF_INET);

        let host = asks_for_a_name_over_ipv4
            // SAFETY: a caller of getaddrinfo gives a C string or null, and null is excluded.
            .then(|| unsafe { CStr::from_ptr(node) })
            .and_then(|host| host.to_str().ok());

        let address = match host.map(|host| super::resolve(host, has_local_soa)) {
            Some(Resolution::Address(address)) => CString::new(address.to_string()).ok(),
            Some(Resolution::NotFound) => return libc::EAI_NONAME,
            Some(Resolution::System) | None => None,
        };

        let node = address.as_ref().map_or(node, |address| address.as_ptr());

        // SAFETY: the arguments are those of the caller, or an address that lives until the return.
        unsafe { __real_getaddrinfo(node, service, hints, result) }
    }

    fn has_local_soa() -> bool {
        let mut answer = [0; SOA_ANSWER_BYTES as usize];

        // SAFETY: the name is a C string, and the buffer has the length that is given.
        let answer_bytes = unsafe {
            res_query(
                c"local".as_ptr(),
                CLASS_IN,
                TYPE_SOA,
                answer.as_mut_ptr(),
                SOA_ANSWER_BYTES,
            )
        };

        answer_bytes > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTS: &str =
        "127.0.0.1 localhost\n# 10.0.0.1 ghost.local\n10.0.0.5 nas Paron.local # the server\n";

    #[test]
    fn is_mdns_name_of_one_label_under_local_returns_true() {
        let hosts = ["paron.local", "Paron-2.LOCAL", "paron.local."];

        let refused: Vec<_> = hosts
            .into_iter()
            .filter(|host| !is_mdns_name(host))
            .collect();

        assert_eq!(refused, Vec::<&str>::new());
    }

    #[test]
    fn is_mdns_name_of_other_names_returns_false() {
        let hosts = [
            "paron.example.com",
            "db.corp.local",
            "local",
            "pa ron.local",
            "10.0.0.5",
        ];

        let accepted: Vec<_> = hosts
            .into_iter()
            .filter(|host| is_mdns_name(host))
            .collect();

        assert_eq!(accepted, Vec::<&str>::new());
    }

    #[test]
    fn names_host_of_listed_name_in_other_case_returns_true() {
        let named = names_host(HOSTS, "paron.local");

        assert!(named);
    }

    #[test]
    fn names_host_of_name_in_comment_returns_false() {
        let named = names_host(HOSTS, "ghost.local");

        assert!(!named);
    }

    #[test]
    fn names_host_of_address_returns_false() {
        let named = names_host(HOSTS, "10.0.0.5");

        assert!(!named);
    }

    #[test]
    fn answer_in_found_line_returns_the_address() {
        let answer = answer_in("+ 11 0 paron.local 172.30.0.2\n");

        assert_eq!(answer, Answer::Address(Ipv4Addr::new(172, 30, 0, 2)));
    }

    #[test]
    fn answer_in_timeout_line_returns_not_found() {
        let answer = answer_in("-15 Timeout reached\n");

        assert_eq!(answer, Answer::NotFound);
    }

    #[test]
    fn answer_in_found_line_without_address_returns_unavailable() {
        let answer = answer_in("+ 11 0 paron.local\n");

        assert_eq!(answer, Answer::Unavailable);
    }

    #[test]
    fn resolve_name_outside_local_returns_system() {
        let resolution = resolve("paron.example.com", || unreachable!());

        assert_eq!(resolution, Resolution::System);
    }
}
