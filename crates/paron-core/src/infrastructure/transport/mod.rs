pub mod protocol;
pub mod tunnel;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use paron_tunnel_protocol::{AgentId, ConnectedAgents, Selector, TunnelError};

use crate::application::machine::{
    MachineTransport, Target, TerminalSession, TerminalSize, TerminalType, TransportError,
};
use crate::domain::machine::tunnel::Enrollment;
use crate::domain::machine::{ConnectionTest, Connectivity, Terminal};

pub use paron_tunnel_protocol::Stream;

impl From<TunnelError> for TransportError {
    fn from(error: TunnelError) -> Self {
        Self::Unreachable(error.into())
    }
}

/// A way to reach a service on a machine, however that machine is reached.
///
/// One method per kind of service, rather than one method naming a kind. What a machine reached
/// by address can serve is not what one reached by a tunnel can — a socket path names a file on
/// the machine's own host, and only an agent is there to open it — so a second kind is a second
/// method, added when something asks for it.
pub trait Destination: Send + Sync {
    fn tcp(
        &self,
        port: u16,
    ) -> impl Future<Output = Result<Box<dyn Stream>, TransportError>> + Send;
}

/// Any closure that hands back a stream for a port is a way to connect.
impl<F, S> Destination for F
where
    F: Fn(u16) -> S + Send + Sync,
    S: Future<Output = Result<Box<dyn Stream>, TransportError>> + Send,
{
    fn tcp(
        &self,
        port: u16,
    ) -> impl Future<Output = Result<Box<dyn Stream>, TransportError>> + Send {
        self(port)
    }
}

pub struct Transport {
    tunnel: Arc<ConnectedAgents>,
    ssh: protocol::ssh::SshProtocol,
}

impl Transport {
    pub fn new(tunnel: Arc<ConnectedAgents>, ssh_keepalive_interval: Duration) -> Self {
        Self {
            tunnel,
            ssh: protocol::ssh::SshProtocol {
                keepalive_interval: ssh_keepalive_interval,
            },
        }
    }

    /// Resolves whether there's direct connection to the machine or a tunnel
    async fn connect_tcp(
        &self,
        machine_id: uuid::Uuid,
        connectivity: &Connectivity,
        port: u16,
    ) -> Result<Box<dyn Stream>, TransportError> {
        match connectivity {
            Connectivity::Direct { host } => {
                let stream = tokio::net::TcpStream::connect((host.to_string(), port))
                    .await
                    .map_err(|error| TransportError::Unreachable(Box::new(error)))?;

                let _ = stream.set_nodelay(true);

                Ok(Box::new(stream))
            }
            Connectivity::Tunneled(Enrollment::NotEnrolled | Enrollment::Invited { .. }) => {
                Err(TransportError::NotEnrolled)
            }
            Connectivity::Tunneled(Enrollment::Enrolled { .. }) => {
                let selector = Selector::tcp(port)
                    .ok_or_else(|| TransportError::Unreachable("port 0 is not a service".into()))?;

                Ok(self
                    .tunnel
                    .open(AgentId::new(machine_id), &selector)
                    .await?)
            }
        }
    }
}

#[async_trait]
impl MachineTransport for Transport {
    async fn verify(&self, target: Target<ConnectionTest>) -> Result<(), TransportError> {
        let destination = |port| self.connect_tcp(target.machine_id, &target.connectivity, port);

        match &target.capability {
            ConnectionTest::Ssh(access) => self.ssh.verify(access, &destination).await,
        }
    }

    async fn terminal(
        &self,
        target: Target<Terminal>,
        size: TerminalSize,
        terminal: TerminalType,
    ) -> Result<TerminalSession, TransportError> {
        let destination = |port| self.connect_tcp(target.machine_id, &target.connectivity, port);

        match &target.capability {
            Terminal::Ssh(access) => self.ssh.shell(access, &destination, size, terminal).await,
        }
    }

    async fn disconnect(&self, machine_id: uuid::Uuid) {
        self.tunnel.disconnect(AgentId::new(machine_id)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::machine;

    const SSH_PORT: u16 = 22;

    fn transport_without_agents() -> Transport {
        Transport::new(Arc::new(ConnectedAgents::new()), Duration::from_secs(15))
    }

    #[tokio::test]
    async fn connect_tcp_to_machine_without_enrollment_returns_not_enrolled() {
        for (enrollment_state, machine) in [
            ("unenrolled", machine::unenrolled_machine()),
            ("invited", machine::invited_machine()),
        ] {
            let stream = transport_without_agents()
                .connect_tcp(machine.id, &machine.connectivity, SSH_PORT)
                .await;

            assert_matches!(
                stream.err(),
                Some(TransportError::NotEnrolled),
                "{enrollment_state}"
            );
        }
    }

    #[tokio::test]
    async fn connect_tcp_to_enrolled_machine_without_tunnel_returns_unreachable() {
        let machine = machine::enrolled_machine();

        let stream = transport_without_agents()
            .connect_tcp(machine.id, &machine.connectivity, SSH_PORT)
            .await;

        assert_matches!(stream.err(), Some(TransportError::Unreachable(_)));
    }
}
