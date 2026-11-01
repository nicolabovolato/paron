use std::sync::Arc;

use russh::keys::{
    Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
};
use russh::{ChannelMsg, Preferred, client};
use secrecy::ExposeSecret;

use crate::application::machine::{
    TerminalEvent, TerminalExit, TerminalInput, TerminalSession, TerminalSize, TerminalType,
    TransportError,
};
use crate::domain::machine::ssh;
use crate::infrastructure::transport::Destination;

pub struct SshProtocol {
    pub keepalive_interval: std::time::Duration,
}

impl SshProtocol {
    /// A connection with the machine, over which channels can be opened. Getting there means a
    /// handshake, which proves the machine, and then authentication, which proves us.
    async fn connect(
        &self,
        access: &ssh::Access,
        destination: &impl Destination,
    ) -> Result<client::Handle<ExpectHostKey>, TransportError> {
        let host_key = PublicKey::from_bytes(access.authentication.host_key())
            .map_err(|error| TransportError::Unexpected(Box::new(error)))?;

        let key = match host_key.algorithm() {
            Algorithm::Rsa { .. } => vec![
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha512),
                },
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha256),
                },
                Algorithm::Rsa { hash: None },
            ],
            algorithm => vec![algorithm],
        };

        let config = client::Config {
            preferred: Preferred {
                key: key.into(),
                ..Preferred::DEFAULT
            },
            // A shell can be idle for hours; without these, a machine that went away is noticed
            // only when somebody types. Three unanswered keepalives end the connection.
            keepalive_interval: Some(self.keepalive_interval),
            ..client::Config::default()
        };

        let connection = async {
            let stream = destination.tcp(access.port.get()).await?;

            let mut connection = client::connect_stream(
                Arc::new(config),
                stream,
                ExpectHostKey { expected: host_key },
            )
            .await
            .map_err(|error| match error {
                russh::Error::UnknownKey => TransportError::HostKeyMismatch,
                error => TransportError::Unreachable(Box::new(error)),
            })?;

            let auth_attempt = match access.authentication.method() {
                ssh::AuthMethod::Password(password) => {
                    connection
                        .authenticate_password(&*access.user, password.expose_secret())
                        .await
                }
                ssh::AuthMethod::Keypair { private_key, .. } => {
                    let key = PrivateKey::from_bytes(private_key.expose_secret())
                        .map_err(|error| TransportError::Unexpected(Box::new(error)))?;

                    let hash = if matches!(key.algorithm(), Algorithm::Rsa { .. }) {
                        connection
                            .best_supported_rsa_hash()
                            .await
                            .map_err(|error| TransportError::Unreachable(Box::new(error)))?
                            .flatten()
                    } else {
                        None
                    };

                    connection
                        .authenticate_publickey(
                            &*access.user,
                            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                        )
                        .await
                }
            };

            match auth_attempt {
                Ok(auth) if auth.success() => Ok(connection),
                Ok(_) => Err(TransportError::AuthenticationFailed),
                Err(error) => Err(TransportError::Unreachable(Box::new(error))),
            }
        };

        tokio::time::timeout(*access.connect_timeout, connection)
            .await
            .map_err(|timeout| TransportError::Unreachable(Box::new(timeout)))?
    }
}

struct ExpectHostKey {
    expected: PublicKey,
}

impl client::Handler for ExpectHostKey {
    type Error = russh::Error;

    #[expect(clippy::unused_async_trait_impl)]
    async fn check_server_key(
        &mut self,
        presented: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(matches!(
            presented,
            PublicKeyOrCertificate::PublicKey { key, .. } if key.key_data() == self.expected.key_data()
        ))
    }
}

impl SshProtocol {
    pub async fn shell(
        &self,
        access: &ssh::Access,
        destination: &impl Destination,
        size: TerminalSize,
        terminal: TerminalType,
    ) -> Result<TerminalSession, TransportError> {
        // The connect timeout bounds everything until the shell runs: a server that accepts the
        // login and then never answers still answers keepalives. A running shell may be silent.
        let (term, colorterm) = match terminal {
            // The one value every SSH server knows, OpenSSH on Windows included.
            TerminalType::XtermTrueColor => ("xterm-256color", "truecolor"),
        };

        let setup = async {
            let connection = self.connect(access, destination).await?;
            let mut channel = connection
                .channel_open_session()
                .await
                .map_err(|error| TransportError::Unreachable(Box::new(error)))?;

            channel
                .request_pty(
                    true,
                    term,
                    size.columns.get().into(),
                    size.rows.get().into(),
                    0,
                    0,
                    &[],
                )
                .await
                .map_err(|error| TransportError::Unreachable(Box::new(error)))?;
            // Most servers accept only the variables their configuration names; a refusal is harmless.
            channel
                .set_env(false, "COLORTERM", colorterm)
                .await
                .map_err(|error| TransportError::Unreachable(Box::new(error)))?;
            // The shell's bytes are UTF-8, on a server whose default locale may be plain C. Only the
            // character type: language, dates and numbers stay as the server has them.
            channel
                .set_env(false, "LC_CTYPE", "C.UTF-8")
                .await
                .map_err(|error| TransportError::Unreachable(Box::new(error)))?;
            channel
                .request_shell(true)
                .await
                .map_err(|error| TransportError::Unreachable(Box::new(error)))?;

            // The answers come in the order of the requests: the terminal, then the shell.
            let mut granted = 0;
            while granted < 2 {
                match channel.wait().await {
                    Some(ChannelMsg::Success) => granted += 1,
                    Some(ChannelMsg::Failure) => return Err(TransportError::TerminalRefused),
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                        return Err(TransportError::Unreachable(
                            "the machine closed the session before the shell started".into(),
                        ));
                    }
                    Some(_) => {}
                }
            }

            Ok((connection, channel))
        };
        let (connection, channel) = tokio::time::timeout(*access.connect_timeout, setup)
            .await
            .map_err(|timeout| TransportError::Unreachable(Box::new(timeout)))??;

        let (reader, writer) = channel.split();

        let input = futures_util::sink::unfold(writer, async |writer, input| {
            match input {
                TerminalInput::Data(bytes) => writer.data(&bytes[..]).await,
                TerminalInput::Resize(size) => {
                    writer
                        .window_change(size.columns.get().into(), size.rows.get().into(), 0, 0)
                        .await
                }
            }
            .map_err(|error| TransportError::Unreachable(Box::new(error)))?;

            Ok(writer)
        });

        let output = futures_util::stream::unfold(
            Some(SshShellOutput {
                reader,
                exit: None,
                _connection: connection,
            }),
            async |state| {
                let mut state = state?;
                loop {
                    match state.reader.wait().await {
                        Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                            return Some((TerminalEvent::Output(data.to_vec()), Some(state)));
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            state.exit = Some(TerminalExit::Status(exit_status));
                        }
                        Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                            state.exit = Some(TerminalExit::Signal(format!("{signal_name:?}")));
                        }
                        Some(ChannelMsg::Close) | None => {
                            let exit = state.exit.unwrap_or(TerminalExit::Disconnected);
                            return Some((TerminalEvent::Exit(exit), None));
                        }
                        Some(_) => {}
                    }
                }
            },
        );

        Ok(TerminalSession {
            input: Box::pin(input),
            output: Box::pin(output),
        })
    }

    pub async fn verify(
        &self,
        access: &ssh::Access,
        destination: &impl Destination,
    ) -> Result<(), TransportError> {
        self.connect(access, destination).await?;

        Ok(())
    }
}

struct SshShellOutput {
    reader: russh::ChannelReadHalf,
    exit: Option<TerminalExit>,
    /// The session ends when this handle drops, so the output half, which lives as long as
    /// somebody reads, keeps it.
    _connection: client::Handle<ExpectHostKey>,
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::num::NonZeroU16;
    use std::sync::{Mutex, Weak};
    use std::time::Duration;

    use futures_util::{SinkExt as _, StreamExt as _};
    use paron_tunnel_protocol::ConnectedAgents;
    use testcontainers::core::{IntoContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{ContainerAsync, GenericImage, ImageExt};
    use tokio::time::Instant;

    use super::*;
    use crate::application::machine::{MachineTransport as _, Target};
    use crate::domain::machine::{ConnectionTest, Terminal};
    use crate::infrastructure::transport::Transport;
    use crate::testing::machine::{SSH_PASSWORD, SSH_USER, ssh_access};
    use crate::testing::{keys, machine};

    const SSHD_PORT: u16 = 2222;
    const LOOPBACK: &str = "127.0.0.1";
    const WRONG_PASSWORD: &str = "not-the-password";
    const SHORT_CONNECT_TIMEOUT: Duration = ssh::ConnectTimeout::MIN;

    fn transport_without_agents() -> Transport {
        Transport::new(Arc::new(ConnectedAgents::new()), Duration::from_secs(15))
    }

    async fn closed_port() -> NonZeroU16 {
        let listener = tokio::net::TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let port = NonZeroU16::new(listener.local_addr().unwrap().port()).expect("a bound port");
        drop(listener);

        port
    }

    async fn silent_port() -> NonZeroU16 {
        let listener = tokio::net::TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let port = NonZeroU16::new(listener.local_addr().unwrap().port()).expect("a bound port");

        tokio::spawn(async move {
            let mut open_sockets = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                open_sockets.push(socket);
            }
        });

        port
    }

    #[tokio::test]
    async fn verify_with_closed_port_returns_unreachable() {
        let machine = machine::machine_builder()
            .direct_host(LOOPBACK)
            .port(closed_port().await)
            .build();

        let verified = transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await;

        assert_matches!(verified, Err(TransportError::Unreachable(_)));
    }

    // The clock is paused, so the timeout ends the wait at once and the duration is exact.
    #[tokio::test(start_paused = true)]
    async fn verify_with_silent_host_gives_up_after_the_connect_timeout_of_the_machine() {
        let machine = machine::machine_builder()
            .direct_host(LOOPBACK)
            .port(silent_port().await)
            .connect_timeout(SHORT_CONNECT_TIMEOUT)
            .build();
        let verify_started_at = Instant::now();

        let verified = transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await;

        let verify_duration = verify_started_at.elapsed();
        assert_matches!(verified, Err(TransportError::Unreachable(_)));
        assert!(
            verify_duration >= SHORT_CONNECT_TIMEOUT && verify_duration < SHORT_CONNECT_TIMEOUT * 2,
            "{verify_duration:?}"
        );
    }

    static SHARED_SSHD: tokio::sync::Mutex<Weak<Sshd>> = tokio::sync::Mutex::const_new(Weak::new());

    async fn shared_sshd() -> Arc<Sshd> {
        // The lock is held while the container starts, so concurrent tests wait for one container.
        let mut shared_sshd = SHARED_SSHD.lock().await;

        if let Some(sshd) = shared_sshd.upgrade() {
            return sshd;
        }

        let sshd = Arc::new(Sshd::start().await);
        *shared_sshd = Arc::downgrade(&sshd);

        sshd
    }

    struct Sshd {
        _container: ContainerAsync<GenericImage>,
        port: NonZeroU16,
        host_key: Vec<u8>,
    }

    impl Sshd {
        async fn start() -> Self {
            let container = GenericImage::new("lscr.io/linuxserver/openssh-server", "latest")
                .with_exposed_port(SSHD_PORT.tcp())
                .with_wait_for(WaitFor::message_on_stdout("[ls.io-init] done."))
                .with_env_var("PASSWORD_ACCESS", "true")
                .with_env_var("USER_NAME", SSH_USER)
                .with_env_var("USER_PASSWORD", SSH_PASSWORD)
                .with_env_var("PUBLIC_KEY", authorized_keys())
                // All tests connect to this server at the same time. Above 10 unauthenticated
                // connections (the default), sshd drops new connections at random.
                .with_copy_to(
                    "/config/sshd/sshd_config.d/max-startups.conf",
                    b"MaxStartups 100\n".to_vec(),
                )
                // All tests come from one address, and some fail a login. sshd adds a penalty for
                // each failure and then drops all connections from that address.
                .with_copy_to(
                    "/config/sshd/sshd_config.d/per-source-penalties.conf",
                    b"PerSourcePenalties no\n".to_vec(),
                )
                .start()
                .await
                .expect("the sshd container must start");

            let port =
                NonZeroU16::new(container.get_host_port_ipv4(SSHD_PORT.tcp()).await.unwrap())
                    .expect("a bound port");

            Self {
                _container: container,
                port,
                host_key: presented_host_key(port, Algorithm::Ed25519).await.to_vec(),
            }
        }

        fn machine_builder(&self) -> machine::MachineBuilder {
            machine::machine_builder()
                .direct_host(LOOPBACK)
                .port(self.port)
                .host_key(ssh::PublicKey::parse(self.host_key.clone()).unwrap())
        }
    }

    fn authorized_keys() -> String {
        [
            keys::SSH_PUBLIC_KEY.to_vec(),
            public_key_of(keys::SSH_RSA_PRIVATE_KEY),
        ]
        .map(|key| {
            ssh_key::PublicKey::from_bytes(&key)
                .unwrap()
                .to_openssh()
                .unwrap()
        })
        .join("\n")
    }

    fn public_key_of(private_key: &[u8]) -> Vec<u8> {
        ssh_key::PrivateKey::from_bytes(private_key)
            .unwrap()
            .public_key()
            .to_bytes()
            .unwrap()
    }

    struct HostKeyRecorder(Arc<Mutex<Vec<u8>>>);

    impl client::Handler for HostKeyRecorder {
        type Error = russh::Error;

        #[expect(clippy::unused_async_trait_impl)]
        async fn check_server_key(
            &mut self,
            host_key: &PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            let PublicKeyOrCertificate::PublicKey { key, .. } = host_key else {
                return Ok(false);
            };
            *self.0.lock().unwrap() = key.to_bytes()?;

            Ok(true)
        }
    }

    async fn presented_host_key(port: NonZeroU16, algorithm: Algorithm) -> ssh::PublicKey {
        let recorded_host_key = Arc::new(Mutex::new(Vec::new()));

        client::connect(
            Arc::new(client::Config {
                preferred: Preferred {
                    key: vec![algorithm].into(),
                    ..Preferred::DEFAULT
                },
                ..client::Config::default()
            }),
            (LOOPBACK, port.get()),
            HostKeyRecorder(Arc::clone(&recorded_host_key)),
        )
        .await
        .expect("the server must complete a handshake");

        let host_key = recorded_host_key.lock().unwrap().clone();

        ssh::PublicKey::parse(host_key).expect("the server must present a usable host key")
    }

    fn imported_keypair_method(private_key: &[u8]) -> ssh::AuthMethod {
        machine::imported_keypair_method(
            ssh::PrivateKey::parse(secrecy::SecretSlice::from(private_key.to_vec())).unwrap(),
        )
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_matching_host_key_and_password_succeeds() {
        let sshd = shared_sshd().await;
        let machine = sshd.machine_builder().build();

        transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await
            .unwrap();
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_wrong_password_returns_authentication_failed() {
        let sshd = shared_sshd().await;
        let machine = sshd
            .machine_builder()
            .authentication(machine::password_method(WRONG_PASSWORD))
            .build();

        let verified = transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await;

        assert_matches!(verified, Err(TransportError::AuthenticationFailed));
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_unexpected_host_key_returns_host_key_mismatch() {
        let sshd = shared_sshd().await;
        let machine = sshd
            .machine_builder()
            .host_key(keys::ssh_public_key())
            .build();

        let verified = transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await;

        assert_matches!(verified, Err(TransportError::HostKeyMismatch));
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_ecdsa_host_key_succeeds() {
        let sshd = shared_sshd().await;
        let host_key = presented_host_key(
            sshd.port,
            Algorithm::Ecdsa {
                curve: russh::keys::EcdsaCurve::NistP256,
            },
        )
        .await;
        let machine = sshd.machine_builder().host_key(host_key).build();

        transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await
            .unwrap();
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_rsa_host_key_succeeds() {
        let sshd = shared_sshd().await;
        let host_key = presented_host_key(
            sshd.port,
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
        )
        .await;
        let machine = sshd.machine_builder().host_key(host_key).build();

        transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await
            .unwrap();
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_authorized_keypair_succeeds() {
        let sshd = shared_sshd().await;
        let machine = sshd
            .machine_builder()
            .authentication(imported_keypair_method(keys::SSH_PRIVATE_KEY))
            .build();

        transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await
            .unwrap();
    }

    // By default russh signs an RSA key as `ssh-rsa`, which OpenSSH refuses since 8.8.
    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_authorized_rsa_keypair_succeeds() {
        let sshd = shared_sshd().await;
        let machine = sshd
            .machine_builder()
            .authentication(imported_keypair_method(keys::SSH_RSA_PRIVATE_KEY))
            .build();

        transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await
            .unwrap();
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn verify_with_unauthorized_keypair_returns_authentication_failed() {
        let sshd = shared_sshd().await;
        let machine = sshd
            .machine_builder()
            .authentication(imported_keypair_method(keys::SSH_UNAUTHORIZED_PRIVATE_KEY))
            .build();

        let verified = transport_without_agents()
            .verify(Target {
                machine_id: machine.id,
                connectivity: machine.connectivity.clone(),
                capability: ConnectionTest::Ssh(ssh_access(&machine).clone()),
            })
            .await;

        assert_matches!(verified, Err(TransportError::AuthenticationFailed));
    }

    fn terminal_size() -> TerminalSize {
        TerminalSize {
            columns: NonZeroU16::new(80).unwrap(),
            rows: NonZeroU16::new(24).unwrap(),
        }
    }

    async fn read_output_until_exit(shell: &mut TerminalSession) -> (String, TerminalExit) {
        let read_output = async {
            let mut output_bytes = Vec::new();
            loop {
                match shell.output.next().await {
                    Some(TerminalEvent::Output(bytes)) => output_bytes.extend_from_slice(&bytes),
                    Some(TerminalEvent::Exit(exit_status)) => {
                        return (
                            String::from_utf8_lossy(&output_bytes).into_owned(),
                            exit_status,
                        );
                    }
                    None => panic!("the output ended without saying how the shell ended"),
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(20), read_output)
            .await
            .expect("the shell must end within the time")
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn shell_runs_input_and_reports_the_exit_status() {
        let sshd = shared_sshd().await;
        let machine = sshd.machine_builder().build();
        let mut shell = transport_without_agents()
            .terminal(
                Target {
                    machine_id: machine.id,
                    connectivity: machine.connectivity.clone(),
                    capability: Terminal::Ssh(ssh_access(&machine).clone()),
                },
                terminal_size(),
                TerminalType::XtermTrueColor,
            )
            .await
            .unwrap();

        // The arithmetic is done by the shell, so the echo of the typed line cannot match.
        shell
            .input
            .send(TerminalInput::Data(
                b"echo marker-$((40+2)); exit 3\n".to_vec(),
            ))
            .await
            .unwrap();
        let (shell_output, exit_status) = read_output_until_exit(&mut shell).await;

        assert!(
            shell_output.contains("marker-42"),
            "printed: {shell_output:?}"
        );
        assert_eq!(exit_status, TerminalExit::Status(3));
    }

    #[cfg(not(disable_testcontainers))]
    #[tokio::test]
    async fn shell_resize_changes_the_size_of_the_terminal() {
        let sshd = shared_sshd().await;
        let machine = sshd.machine_builder().build();
        let mut shell = transport_without_agents()
            .terminal(
                Target {
                    machine_id: machine.id,
                    connectivity: machine.connectivity.clone(),
                    capability: Terminal::Ssh(ssh_access(&machine).clone()),
                },
                terminal_size(),
                TerminalType::XtermTrueColor,
            )
            .await
            .unwrap();

        shell
            .input
            .send(TerminalInput::Resize(TerminalSize {
                columns: NonZeroU16::new(132).unwrap(),
                rows: NonZeroU16::new(40).unwrap(),
            }))
            .await
            .unwrap();
        shell
            .input
            .send(TerminalInput::Data(b"stty size; exit\n".to_vec()))
            .await
            .unwrap();
        let (shell_output, _) = read_output_until_exit(&mut shell).await;

        assert!(shell_output.contains("40 132"), "printed: {shell_output:?}");
    }
}
