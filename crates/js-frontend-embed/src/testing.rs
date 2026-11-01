#![cfg(any(feature = "build", feature = "serve"))]

use std::path::{Path, PathBuf};

#[cfg(all(feature = "serve", debug_assertions))]
const FIRST_PORT: u16 = 20_000;

#[cfg(all(feature = "serve", debug_assertions))]
const PORT_COUNT: u16 = 12_000;

// The kernel gives out ephemeral ports from 32768 on Linux and from 49152 on macOS and Windows.
// The test ports stay below the lower limit, so the kernel cannot give one of them to a socket.
#[cfg(all(feature = "serve", debug_assertions))]
const _: () = assert!(FIRST_PORT + PORT_COUNT <= 32_768);

/// A free port that no other test process or thread can lease until this lease is dropped.
#[cfg(all(feature = "serve", debug_assertions))]
#[derive(Debug)]
pub(crate) struct PortLease {
    port: u16,
    _lock_file: std::fs::File,
}

#[cfg(all(feature = "serve", debug_assertions))]
impl PortLease {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

#[cfg(all(feature = "serve", debug_assertions))]
pub(crate) fn lease_port() -> PortLease {
    let lock_directory = std::env::temp_dir().join("js-frontend-embed-port-locks");
    std::fs::create_dir_all(&lock_directory).unwrap();

    for port in FIRST_PORT..FIRST_PORT + PORT_COUNT {
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(lock_directory.join(port.to_string()))
            .unwrap();

        match lock_file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(error)) => panic!("lock of port {port}: {error}"),
        }

        let addresses = crate::dev::addresses("localhost", port).unwrap();

        if !crate::dev::taken(&addresses) {
            return PortLease {
                port,
                _lock_file: lock_file,
            };
        }
    }

    panic!("no free port below the ephemeral range");
}

#[cfg(all(feature = "serve", debug_assertions))]
pub(crate) fn bun_frontend(package_json: &str) -> TempDir {
    let frontend = temp_dir();
    frontend.write_file("bun.lock", "");
    frontend.write_file("package.json", package_json);

    frontend
}

#[cfg(all(feature = "serve", debug_assertions, unix))]
pub(crate) fn node_dev_server_package_json(script: &str) -> String {
    format!(
        r#"{{"name":"t","scripts":{{"{script}":"node -e \"require('http').createServer((_,r)=>r.end(process.env.SAYS)).listen(process.env.JS_FRONTEND_EMBED_PORT)\""}}}}"#
    )
}

pub(crate) fn temp_dir() -> TempDir {
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_TEMP_DIR_NUMBER: AtomicU32 = AtomicU32::new(0);

    let path = std::env::temp_dir().join(format!(
        "js-frontend-embed-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIR_NUMBER.fetch_add(1, Ordering::Relaxed)
    ));

    std::fs::create_dir_all(&path).unwrap();

    TempDir(path)
}

#[derive(Debug)]
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn write_file(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();

        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[cfg(feature = "build")]
pub(crate) fn files_under(directory: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];

    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();

            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(directory).unwrap();

                files.push(
                    relative
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }

    files.sort();
    files
}
