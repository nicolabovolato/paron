use std::path::Path;

use crate::{Error, PackageManager};

/// The same as [`Dev::new`], spelled the way a call site reads better.
// Taken by value and never read: the frontend is the type, and `dev(Bundle)` reads better than
// `dev::<Bundle>()`. A unit struct costs nothing to pass.
pub fn dev<F: crate::Frontend>(_frontend: F) -> Dev {
    let mut dev = Dev::new(Path::new(F::DIRECTORY));

    // The same environment the build had, so a bundler config reads one variable whether it is
    // building or serving. Anything else is `Dev::env` on top of this.
    for (name, value) in crate::declared(F::ENVIRONMENT) {
        dev = dev.env(name, value);
    }

    dev
}

/// ```no_run
/// # fn main() -> Result<(), js_frontend_embed::Error> {
/// let _running = js_frontend_embed::Dev::new("frontend").port(5173).spawn()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Dev {
    inner: Inner,
}

#[derive(Debug, Clone)]
struct Inner {
    directory: std::path::PathBuf,
    package_manager: Option<PackageManager>,
    script: String,
    host: String,
    port: u16,
    environment: Vec<(String, String)>,
}

impl Dev {
    /// The frontend at `directory`, started with its `dev` script on port 5173.
    pub fn new(directory: impl AsRef<Path>) -> Self {
        Self {
            inner: Inner {
                directory: directory.as_ref().to_owned(),
                package_manager: None,
                script: "dev".to_owned(),
                host: "localhost".to_owned(),
                port: 5173,
                environment: Vec::new(),
            },
        }
    }

    /// Override the manager the lockfile implies.
    #[must_use]
    pub fn package_manager(mut self, manager: PackageManager) -> Self {
        self.inner.package_manager = Some(manager);
        self
    }

    /// The `package.json` script that starts the dev server. `dev` by convention.
    #[must_use]
    pub fn script(mut self, script: &str) -> Self {
        script.clone_into(&mut self.inner.script);
        self
    }

    /// Published to the dev server's process as [`crate::PORT`]. [`Dev::spawn`] refuses a port
    /// that is already taken, and refuses 0: the number has to be known before the bundler is
    /// started, because the bundler is what gets told it.
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.inner.port = port;
        self
    }

    /// The host half of [`DevServer::origin`], `localhost` by default.
    ///
    /// The scheme is not a choice: this hop speaks plain HTTP, so leave the bundler's own TLS
    /// setting alone — `server.https` in Vite — or every answer will be unreadable.
    #[must_use]
    pub fn host(mut self, host: &str) -> Self {
        host.clone_into(&mut self.inner.host);
        self
    }

    /// A variable for the dev server's process, on top of what this one inherited.
    #[must_use]
    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.inner.environment.push((key.to_owned(), value.into()));
        self
    }

    /// Starts it, and keeps it running until the returned [`DevServer`] is dropped.
    ///
    /// Starts one whenever it is called, so the caller is what decides there should be one. There
    /// is no way to tell a development run from `cargo test` by looking at the environment — same
    /// profile, same cargo, same sources on disk — so anything guessed here is wrong for one of
    /// them, and the guess this used to make was wrong for both: it started a bundler under
    /// `cargo test`, and refused one for a binary built by `cargo build` beside its own sources.
    ///
    /// Blocks until the bundler's
    /// port accepts a connection, so nothing forwarded afterwards comes back as a gateway error.
    /// It is not a promise that the frontend is ready to draw: a cold bundler optimises
    /// dependencies when the first request arrives.
    ///
    /// This process being killed during the wait leaves the dev server behind — the child is in
    /// a process group of its own, which is what lets shutdown stop what it started and what
    /// keeps a terminal interrupt from reaching it. The next run finds the port taken and says so.
    pub fn spawn(self) -> Result<DevServer, Error> {
        use std::process::Command;

        let inner = self.inner;
        let manager = match inner.package_manager {
            Some(manager) => manager,
            None => PackageManager::detect(&inner.directory)?,
        };

        // Binding port 0 succeeds, so the `taken` check below would wave it through and
        // everything afterwards would dial a port the bundler never listened on.
        if inner.port == 0 {
            return Err(Error::PortNotChosen);
        }

        // A host that names nothing is invisible to the wait below: the dev server binds what
        // its own config says and runs fine, so nothing dies and nothing answers.
        let Some(addresses) = addresses(&inner.host, inner.port) else {
            return Err(Error::UnknownHost { host: inner.host });
        };

        if taken(&addresses) {
            return Err(Error::PortTaken {
                host: inner.host.clone(),
                port: inner.port,
            });
        }

        let mut command = Command::new(manager.program());
        command
            .args(["run", &inner.script])
            .current_dir(&inner.directory)
            .env(crate::PORT, inner.port.to_string());

        for (key, value) in &inner.environment {
            command.env(key, value);
        }

        // Its own process group, so that stopping it stops everything it started. See `stop`.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;

            command.process_group(0);
        }

        let child = command.spawn().map_err(|error| match error.kind() {
            // `spawn` reports a missing working directory the same way it reports a program that
            // is not installed.
            std::io::ErrorKind::NotFound if inner.directory.is_dir() => Error::NotInstalled {
                program: manager.program().to_owned(),
            },
            _ => Error::io(&inner.directory, error),
        })?;

        // Before the wait, so a dev server that never answers is still stopped on the way out.
        let mut server = DevServer {
            child,
            origin: format!("http://{}:{}", inner.host, inner.port),
        };

        server.wait_until_listening(&addresses, &inner.script, GIVE_UP_STARTING_AFTER)?;

        Ok(server)
    }
}

/// A dev server that is running and answering. Holding one keeps it alive; dropping it stops the
/// dev server and everything the dev server started.
#[derive(Debug)]
pub struct DevServer {
    child: std::process::Child,
    origin: String,
}

impl DevServer {
    /// Where it is answering, for [`proxying`](crate::proxying) to forward to. Not somewhere to
    /// send a browser.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Blocks until the dev server answers on its port, or explains why it never will. The child
    /// is watched alongside the port, because a bundler that dies on a bad config dies in about
    /// the time it would otherwise have taken to start.
    fn wait_until_listening(
        &mut self,
        addresses: &[std::net::SocketAddr],
        script: &str,
        patience: std::time::Duration,
    ) -> Result<(), Error> {
        let deadline = std::time::Instant::now() + patience;

        loop {
            // Each attempt is bounded: a refused connection answers at once, but a filtered port
            // answers with silence, and that wait belongs to the operating system.
            let answered = addresses.iter().any(|address| {
                std::net::TcpStream::connect_timeout(address, REACHABLE_WITHIN).is_ok()
            });

            if answered {
                return Ok(());
            }

            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(Error::DevServerStopped {
                    script: script.to_owned(),
                    status: status.to_string(),
                });
            }

            if std::time::Instant::now() >= deadline {
                return Err(Error::DevServerSilent {
                    script: script.to_owned(),
                    origin: self.origin.clone(),
                    seconds: patience.as_secs(),
                });
            }

            std::thread::sleep(ASK_AGAIN_AFTER);
        }
    }
}

/// Long enough for a cold start on a large project; reaching it means something is wrong.
const GIVE_UP_STARTING_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// Between attempts, both while starting and while stopping. Short enough that neither answer
/// arrives late, long enough that half a minute of waiting is not a spin.
const ASK_AGAIN_AFTER: std::time::Duration = std::time::Duration::from_millis(25);

const REACHABLE_WITHIN: std::time::Duration = std::time::Duration::from_secs(1);

impl Drop for DevServer {
    fn drop(&mut self) {
        if let Err(error) = stop(&mut self.child) {
            tracing::warn!(origin = %self.origin, %error, "the dev server did not stop");
        }
    }
}

/// Every address a name stands for — `localhost` is both `127.0.0.1` and `::1` — or `None` where
/// it stands for nothing this machine can reach.
pub(crate) fn addresses(host: &str, port: u16) -> Option<Vec<std::net::SocketAddr>> {
    use std::net::ToSocketAddrs as _;

    (host, port)
        .to_socket_addrs()
        .ok()
        .map(Iterator::collect)
        .filter(|found: &Vec<_>| !found.is_empty())
}

/// Taken on any one of a name's addresses counts as taken: which one a bundler binds is its own
/// business, and Vite reaches for the IPv6 loopback first.
pub(crate) fn taken(addresses: &[std::net::SocketAddr]) -> bool {
    addresses
        .iter()
        .any(|address| std::net::TcpListener::bind(address).is_err())
}

/// Stops the dev server and everything it started, and returns when all of the process group has
/// gone, so that no worker still holds the port.
///
/// The signal goes to the process group, because a bundler is a supervisor and killing only the
/// parent leaves its workers holding the port.
///
/// POSIX gives no event when a process that is not a child exits, so the group is asked again
/// until `kill(-group, 0)` finds no process. The leader is collected before that question,
/// because Linux counts a leader that is not collected as a member of its group.
#[cfg(unix)]
fn stop(child: &mut std::process::Child) -> std::io::Result<()> {
    stop_within(child, GIVE_UP_STOPPING_AFTER)
}

#[cfg(unix)]
fn stop_within(child: &mut std::process::Child, wait: std::time::Duration) -> std::io::Result<()> {
    use rustix::process::{Pid, Signal};

    let group = Pid::from_child(child);

    signal_group(group, Signal::TERM)?;

    if wait_until_group_gone(child, group, wait)? {
        return Ok(());
    }

    signal_group(group, Signal::KILL)?;

    if wait_until_group_gone(child, group, wait)? {
        return Ok(());
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "process group {} is still there {}s after KILL",
            child.id(),
            wait.as_secs_f32()
        ),
    ))
}

#[cfg(unix)]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> std::io::Result<()> {
    use rustix::io::Errno;

    match rustix::process::kill_process_group(group, signal) {
        // macOS gives PERM when each process left in the group is a zombie or belongs to another
        // user. The wait after the signal finds out which.
        Ok(()) | Err(Errno::SRCH | Errno::PERM) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn wait_until_group_gone(
    child: &mut std::process::Child,
    group: rustix::process::Pid,
    wait: std::time::Duration,
) -> std::io::Result<bool> {
    use rustix::io::Errno;

    let deadline = std::time::Instant::now() + wait;

    loop {
        let leader_collected = match child.try_wait() {
            Ok(status) => status.is_some(),
            // Something else collected the leader, for example the kernel when this process
            // ignores `SIGCHLD`.
            Err(error) if Errno::from_io_error(&error) == Some(Errno::CHILD) => true,
            Err(error) => return Err(error),
        };

        if leader_collected {
            match rustix::process::test_kill_process_group(group) {
                Err(Errno::SRCH) => return Ok(true),
                Ok(()) | Err(Errno::PERM) => {}
                Err(error) => return Err(error.into()),
            }
        }

        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }

        std::thread::sleep(ASK_AGAIN_AFTER);
    }
}

/// A bundler that meant to go has gone in milliseconds; one that has not is holding up whatever
/// asked it to.
#[cfg(unix)]
const GIVE_UP_STOPPING_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

/// `/PID` before the flags is the order the documentation gives; taskkill tolerates the other
/// way round, and nothing here can check that.
#[cfg(not(unix))]
fn ending_the_tree(process: u32) -> [String; 4] {
    [
        "/PID".to_owned(),
        process.to_string(),
        "/T".to_owned(),
        "/F".to_owned(),
    ]
}

/// Stops the dev server and everything it started.
///
/// `taskkill /T` ends the child and everything below it, the way a process group does above. The
/// tidier way — a job object closing with this process — means the Windows API and an unsafe
/// block this workspace forbids.
///
/// There is no polite half: nothing on Windows means "please stop" to a console process the way
/// `SIGTERM` does, so the tree is ended rather than asked. Naming the child by number is safe
/// because a `Child` holds its handle open, which keeps the number reserved until the `wait`.
#[cfg(not(unix))]
fn stop(child: &mut std::process::Child) -> std::io::Result<()> {
    let tree_ending = std::process::Command::new("taskkill")
        .args(ending_the_tree(child.id()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    child.kill()?;
    child.wait()?;

    tree_ending.map(drop)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::{bun_frontend, lease_port, temp_dir};

    const NO_SCRIPTS: &str = r#"{"name":"t"}"#;

    // Each stand-in process exits after this time, also when the test binary is killed.
    #[cfg(unix)]
    const STAND_IN_PROCESS_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30);

    /// A `DevServer` whose process runs and listens nowhere.
    #[cfg(unix)]
    fn sleeping_dev_server(origin: &str) -> DevServer {
        use std::os::unix::process::CommandExt as _;

        DevServer {
            child: std::process::Command::new("sleep")
                .arg(STAND_IN_PROCESS_LIFETIME.as_secs().to_string())
                .process_group(0)
                .spawn()
                .unwrap(),
            origin: origin.to_owned(),
        }
    }

    #[cfg(unix)]
    fn get_response_body(port: u16) -> Option<String> {
        use std::io::{Read as _, Write as _};

        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.write_all(b"GET / HTTP/1.0\r\n\r\n").ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;

        response.rsplit("\r\n\r\n").next().map(str::to_owned)
    }

    #[test]
    fn spawn_on_a_taken_port_returns_port_taken() {
        let listener = std::net::TcpListener::bind(("localhost", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let frontend = bun_frontend(NO_SCRIPTS);

        let spawned = Dev::new(frontend.path()).port(port).spawn();

        assert_matches!(spawned, Err(Error::PortTaken { port: refused, .. }) if refused == port);
    }

    #[test]
    fn spawn_with_an_unresolvable_host_returns_unknown_host() {
        let frontend = bun_frontend(NO_SCRIPTS);

        for unresolvable_host in ["http://localhost", "locahost.invalid"] {
            let spawned = Dev::new(frontend.path()).host(unresolvable_host).spawn();

            assert_matches!(
                spawned,
                Err(Error::UnknownHost { host }) if host == unresolvable_host
            );
        }
    }

    #[test]
    fn spawn_with_port_zero_returns_port_not_chosen() {
        let frontend = bun_frontend(NO_SCRIPTS);

        let spawned = Dev::new(frontend.path()).port(0).spawn();

        assert_matches!(spawned, Err(Error::PortNotChosen));
    }

    #[test]
    fn spawn_without_lockfile_returns_no_lockfile() {
        let frontend = temp_dir();

        let spawned = Dev::new(frontend.path()).spawn();

        assert_matches!(spawned, Err(Error::NoLockfile { .. }));
    }

    #[test]
    fn spawn_with_a_server_that_exits_returns_dev_server_stopped() {
        let lease = lease_port();
        let frontend = bun_frontend(NO_SCRIPTS);

        let spawned = Dev::new(frontend.path()).port(lease.port()).spawn();

        assert_matches!(spawned, Err(Error::DevServerStopped { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_returns_a_server_answering_at_its_origin() {
        let lease = lease_port();
        let frontend = bun_frontend(&crate::testing::node_dev_server_package_json("dev"));

        let dev_server = Dev::new(frontend.path())
            .port(lease.port())
            .env("SAYS", "hello")
            .spawn()
            .unwrap();

        assert_eq!(
            dev_server.origin(),
            format!("http://localhost:{}", lease.port())
        );
        assert_eq!(get_response_body(lease.port()).as_deref(), Some("hello"));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_runs_the_named_script() {
        let lease = lease_port();
        let frontend = bun_frontend(&crate::testing::node_dev_server_package_json("start"));

        let spawned = Dev::new(frontend.path())
            .port(lease.port())
            .script("start")
            .spawn();

        assert_matches!(spawned, Ok(_));
    }

    #[cfg(unix)]
    #[test]
    fn drop_stops_the_dev_server() {
        let lease = lease_port();
        let frontend = bun_frontend(&crate::testing::node_dev_server_package_json("dev"));
        let dev_server = Dev::new(frontend.path())
            .port(lease.port())
            .spawn()
            .unwrap();

        drop(dev_server);

        assert_eq!(get_response_body(lease.port()), None);
    }

    #[test]
    fn taken_with_one_address_of_the_name_held_returns_true() {
        for bound_ip in ["127.0.0.1", "::1"] {
            let listener = std::net::TcpListener::bind((bound_ip, 0))
                .unwrap_or_else(|error| panic!("the test needs a loopback at {bound_ip}: {error}"));
            let port = listener.local_addr().unwrap().port();

            let is_taken = taken(&[
                format!("127.0.0.1:{port}").parse().unwrap(),
                format!("[::1]:{port}").parse().unwrap(),
            ]);

            assert!(is_taken, "{bound_ip}:{port}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn wait_until_listening_with_one_address_answering_returns_ok() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let listening_address = listener.local_addr().unwrap();
        // Port 1 is privileged, so nothing listens on it.
        let closed_address = std::net::SocketAddr::from(([127, 0, 0, 1], 1));
        let mut server = sleeping_dev_server(&format!("http://{listening_address}"));

        let waited = server.wait_until_listening(
            &[closed_address, listening_address],
            "dev",
            std::time::Duration::from_millis(500),
        );

        assert_matches!(waited, Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn wait_until_listening_past_the_deadline_returns_dev_server_silent() {
        let mut server = sleeping_dev_server("http://localhost:1");
        let closed_address = std::net::SocketAddr::from(([127, 0, 0, 1], 1));

        let waited = server.wait_until_listening(
            &[closed_address],
            "dev",
            std::time::Duration::from_millis(100),
        );

        assert_matches!(
            waited,
            Err(Error::DevServerSilent { origin, seconds: 0, .. }) if origin == "http://localhost:1"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_signals_term_before_kill() {
        use std::io::{BufRead as _, Read as _};
        use std::os::unix::process::CommandExt as _;

        // `read` waits on the pipe that `child` holds open. A trapped signal stops `read` at once,
        // and the shell exits when the test process closes the pipe.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("trap 'echo term; exit 0' TERM; echo trap; read line")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        // A signal that arrives before the shell installs the trap kills the shell.
        let mut trap_line = String::new();
        output.read_line(&mut trap_line).unwrap();

        // A busy machine can run the trap after the 1 s that `stop` gives; KILL would then come
        // first. `stop_within` returns as soon as the group has gone.
        stop_within(&mut child, std::time::Duration::from_secs(30)).unwrap();

        let mut output_after_stop = String::new();
        output.read_to_string(&mut output_after_stop).unwrap();
        assert_eq!(trap_line, "trap\n");
        assert_eq!(
            output_after_stop, "term\n",
            "the dev server was never asked to stop, only made to"
        );
    }

    // The worker ignores `TERM` and outlives the supervisor, so only `KILL` to the group stops it.
    #[cfg(unix)]
    #[test]
    fn stop_kills_the_whole_process_group() {
        use std::io::BufRead as _;
        use std::os::unix::process::CommandExt as _;

        let directory = temp_dir();
        let supervisor_script = directory.write_file(
            "supervisor.sh",
            format!(
                "sh -c 'trap \"\" TERM; echo $$; sleep {} & wait' &\n\
                 wait\n",
                STAND_IN_PROCESS_LIFETIME.as_secs()
            ),
        );
        let mut child = std::process::Command::new("sh")
            .arg(&supervisor_script)
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut worker_pid_line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut worker_pid_line)
            .unwrap();
        let worker_pid =
            rustix::process::Pid::from_raw(worker_pid_line.trim().parse().unwrap()).unwrap();

        stop(&mut child).unwrap();

        assert_eq!(
            rustix::process::test_kill_process(worker_pid),
            Err(rustix::io::Errno::SRCH),
            "the worker outlived the dev server and still holds what it held"
        );
    }
}
