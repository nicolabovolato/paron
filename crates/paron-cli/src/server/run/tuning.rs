use std::num::NonZeroUsize;

const WANTED_OPEN_FILE_DESCRIPTORS: u64 = 65_536;

const MAX_DATABASE_CONNECTIONS: NonZeroUsize = NonZeroUsize::new(32).unwrap();

#[derive(Debug)]
pub struct Limits {
    pub paron: crate::server::paron::Limits,
    pub listener: crate::server::run::listener::Limits,
}

pub fn limits(overrides: &crate::server::cli::TuningArgs) -> Limits {
    let _open_fds = crate::platform::limits::raise_fd_limit_to(WANTED_OPEN_FILE_DESCRIPTORS);
    let _memory = detect_memory();
    let parallelism = detect_parallelism();

    let mut paron = crate::server::paron::Limits::default();
    paron.concurrent_password_hashes = parallelism;
    paron.database_connections =
        parallelism.clamp(paron.database_connections, MAX_DATABASE_CONNECTIONS);
    paron.tunnel.max_unadmitted = overrides
        .max_connecting_agents
        .unwrap_or(paron.tunnel.max_unadmitted);

    let limits = Limits {
        paron,
        listener: crate::server::run::listener::Limits::default(),
    };

    tracing::debug!("limits: {limits:?}");

    limits
}

/// The number of CPUs paron can use. A cgroup quota or an affinity mask makes it smaller. A core
/// that runs two threads counts as two CPUs.
fn detect_parallelism() -> NonZeroUsize {
    match std::thread::available_parallelism() {
        Ok(parallelism) => {
            tracing::debug!("detected parallelism of {parallelism}");

            parallelism
        }
        Err(error) => {
            tracing::warn!("could not detect available parallelism: {error}");

            NonZeroUsize::MIN
        }
    }
}

fn detect_memory() -> Option<u64> {
    const RECOMMENDED_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
    const MINIMUM_MEMORY_BYTES: u64 = 256 * 1024 * 1024;

    let total_machine_memory = crate::platform::limits::machine_memory_bytes();
    let max_process_memory = crate::platform::limits::process_memory_bytes()
        .filter(|limit| total_machine_memory.is_none_or(|machine| *limit < machine));

    let memory = max_process_memory.or(total_machine_memory);

    if let Some(limit) = max_process_memory {
        tracing::info!(
            "detected process memory limit: {} MiB",
            limit / (1024 * 1024)
        );
    } else if let Some(total) = total_machine_memory {
        tracing::debug!(
            "detected total machine memory: {} MiB",
            total / (1024 * 1024)
        );
    } else {
        tracing::warn!("could not detect available memory");
    }

    if let Some(memory) = memory {
        if memory < MINIMUM_MEMORY_BYTES {
            tracing::warn!(
                "Paron needs {}+ MiB of memory: expect service degradation",
                MINIMUM_MEMORY_BYTES / (1024 * 1024)
            );
        } else if memory < RECOMMENDED_MEMORY_BYTES {
            tracing::info!(
                "Paron works best with {}+ MiB of memory",
                RECOMMENDED_MEMORY_BYTES / (1024 * 1024)
            );
        }
    }

    memory
}

#[cfg(test)]
mod tests {
    // 1024 is the usual default soft limit for open files.
    #[test]
    fn every_default_together_fits_in_a_default_open_file_limit() {
        let server_limits = crate::server::paron::Limits::default();
        let listener_limits = crate::server::run::listener::Limits::default();

        let open_file_count = server_limits.tunnel.max_unadmitted
            + server_limits.database_connections.get()
            + listener_limits.web_connections
            + listener_limits.pending_connections;

        assert!(
            open_file_count <= 1024 - 128,
            "{open_file_count} leaves nothing for what Paron opens"
        );
    }
}
