/// Raises the open file limit as close to `wanted` as the machine allows, and returns the limit in force afterwards.
pub fn raise_fd_limit_to(wanted: u64) -> u64 {
    #[cfg(windows)]
    {
        wanted
    }

    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

        let limit = getrlimit(Resource::Nofile);

        // Already unlimited?
        let Some(current) = limit.current else {
            return wanted;
        };

        let wanted = wanted.min(limit.maximum.unwrap_or(u64::MAX));

        if current >= wanted {
            return current;
        }

        let raised = binary_search(current, wanted, |value| {
            setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: Some(value),
                    maximum: limit.maximum,
                },
            )
            .is_ok()
        });

        if raised > current {
            tracing::debug!("raised the open file limit from {current} to {raised}");
        } else {
            tracing::warn!("could not raise the open file limit from {current} to {wanted}");
        }

        raised
    }
}

#[cfg(any(unix, test))]
fn binary_search(mut low: u64, mut high: u64, predicate: impl Fn(u64) -> bool) -> u64 {
    if predicate(high) {
        return high;
    }

    while high - low > 1 {
        let mid = low + (high - low) / 2;

        if predicate(mid) {
            low = mid;
        } else {
            high = mid;
        }
    }

    low
}

/// Total physical machine memory in bytes
pub fn machine_memory_bytes() -> Option<u64> {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};

    let system = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    )
    .inspect_err(|reason| tracing::debug!("could not read the machine memory: {reason}"))
    .ok()?;

    // sysinfo reports 0 when it cannot read the total.
    Some(system.total_memory()).filter(|bytes| *bytes > 0)
}

/// Memory this process is allowed to use (e.g. cgroups)
pub fn process_memory_bytes() -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new()
        .inspect_err(|reason| tracing::debug!("could not read the process memory limit: {reason}"))
        .ok()?;
    let own = Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[own]),
        false,
        ProcessRefreshKind::nothing(),
    );

    system
        .process(own)?
        .cgroup_limits()
        .map(|limits| limits.total_memory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_search_returns_the_highest_value_that_passes() {
        for (cap, highest_passing) in [
            (0, 256),
            (257, 257),
            (10240, 10240),
            (65535, 65535),
            (u64::MAX, 65536),
        ] {
            assert_eq!(
                binary_search(256, 65536, |value| value <= cap),
                highest_passing,
                "cap {cap}"
            );
        }
    }

    #[test]
    fn machine_memory_bytes_returns_plausible_bytes() {
        let memory_bytes = machine_memory_bytes().unwrap();

        assert!(
            memory_bytes >= 128 * 1024 * 1024,
            "{memory_bytes} is not a plausible number of bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn raise_fd_limit_to_never_lowers_the_limit() {
        fn soft_fd_limit() -> u64 {
            rustix::process::getrlimit(rustix::process::Resource::Nofile)
                .current
                .unwrap_or(u64::MAX)
        }

        let limit_before = soft_fd_limit();

        raise_fd_limit_to(65_536);

        assert!(soft_fd_limit() >= limit_before);
    }
}
