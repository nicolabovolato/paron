use super::create::{AuthenticationCommand, ConnectivityCommand};
use crate::application::machine::ports::SshKeypairs;
use crate::domain::machine::{Connectivity, Machine, MachineName, ProtocolKind, Protocols, ssh};

pub struct UpdateMachineCommand {
    pub id: uuid::Uuid,
    pub name: Option<MachineName>,
    pub connectivity: Option<ConnectivityCommand>,
    pub protocols: UpdateProtocolsCommand,
}

impl UpdateMachineCommand {
    pub fn apply_to(
        self,
        stored: &Machine,
        keypairs: &dyn SshKeypairs,
    ) -> Result<Machine, InvalidUpdate> {
        let ssh = match self.protocols.ssh {
            None => stored.protocols.ssh.clone(),
            Some(ProtocolUpdate::Remove) => None,
            Some(ProtocolUpdate::Set(cmd)) => {
                Some(cmd.apply_to(stored.protocols.ssh.as_ref(), keypairs)?)
            }
        };

        Ok(Machine {
            id: stored.id,
            name: self.name.unwrap_or_else(|| stored.name.clone()),
            connectivity: match (self.connectivity, &stored.connectivity) {
                (None, stored) => stored.clone(),
                (Some(ConnectivityCommand::Tunneled), Connectivity::Tunneled(enrollment)) => {
                    Connectivity::Tunneled(enrollment.clone())
                }
                (Some(cmd), _) => cmd.into_domain(),
            },
            protocols: Protocols { ssh },
        })
    }
}

#[derive(Default)]
pub struct UpdateProtocolsCommand {
    pub ssh: Option<ProtocolUpdate<UpdateSshCommand>>,
}

pub enum ProtocolUpdate<C> {
    Remove,
    Set(C),
}

pub struct UpdateSshCommand {
    pub port: Option<std::num::NonZeroU16>,
    pub user: Option<ssh::User>,
    pub host_key: Option<ssh::PublicKey>,
    pub authentication: Option<AuthenticationCommand>,
    pub connect_timeout: Option<ssh::ConnectTimeout>,
}

impl UpdateSshCommand {
    fn apply_to(
        self,
        stored: Option<&ssh::Access>,
        keypairs: &dyn SshKeypairs,
    ) -> Result<ssh::Access, InvalidUpdate> {
        let incomplete = || InvalidUpdate::IncompleteProtocol(ProtocolKind::Ssh);

        let port = self
            .port
            .or(stored.map(|access| access.port))
            .ok_or_else(incomplete)?;
        let user = self
            .user
            .or_else(|| stored.map(|access| access.user.clone()))
            .ok_or_else(incomplete)?;
        let connect_timeout = self
            .connect_timeout
            .or(stored.map(|access| access.connect_timeout))
            .ok_or_else(incomplete)?;

        // Last, so that a managed keypair is generated only for an update that is complete.
        let stored_authentication = stored.map(|access| access.authentication.clone());
        let authentication = match (stored_authentication, self.host_key, self.authentication) {
            (_, Some(host_key), Some(cmd)) => {
                ssh::Authentication::new(host_key, cmd.into_domain(keypairs))
            }
            (Some(authentication), None, None) => authentication,
            (Some(authentication), None, Some(cmd)) => {
                authentication.with_method(cmd.into_domain(keypairs))
            }
            (Some(authentication), Some(host_key), None) => {
                authentication.with_host_key(host_key)?
            }
            (None, _, _) => return Err(incomplete()),
        };

        Ok(ssh::Access {
            port,
            user,
            authentication,
            connect_timeout,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidUpdate {
    #[error(transparent)]
    PasswordBoundToHostKey(#[from] ssh::PasswordBoundToHostKey),

    #[error("the machine has no {0} protocol to update")]
    IncompleteProtocol(ProtocolKind),
}
