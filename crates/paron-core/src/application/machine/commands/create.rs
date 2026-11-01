use crate::application::machine::ports::{SshKeyAlgorithm, SshKeypairs};
use crate::domain::machine::{Connectivity, Host, Machine, MachineName, Protocols, ssh, tunnel};

pub struct CreateMachineCommand {
    pub name: MachineName,
    pub connectivity: ConnectivityCommand,
    pub protocols: ProtocolsCommand,
}

impl CreateMachineCommand {
    pub fn into_domain(self, keypairs: &dyn SshKeypairs) -> Machine {
        Machine {
            id: uuid::Uuid::now_v7(),
            name: self.name,
            connectivity: self.connectivity.into_domain(),
            protocols: Protocols {
                ssh: self.protocols.ssh.map(|cmd| cmd.into_domain(keypairs)),
            },
        }
    }
}

pub enum ConnectivityCommand {
    Direct { host: Host },
    Tunneled,
}

impl ConnectivityCommand {
    pub fn into_domain(self) -> Connectivity {
        match self {
            ConnectivityCommand::Direct { host } => Connectivity::Direct { host },
            ConnectivityCommand::Tunneled => {
                Connectivity::Tunneled(tunnel::Enrollment::NotEnrolled)
            }
        }
    }
}

#[derive(Default)]
pub struct ProtocolsCommand {
    pub ssh: Option<SshCommand>,
}

pub struct SshCommand {
    pub port: std::num::NonZeroU16,
    pub user: ssh::User,
    pub host_key: ssh::PublicKey,
    pub authentication: AuthenticationCommand,
    pub connect_timeout: ssh::ConnectTimeout,
}

impl SshCommand {
    fn into_domain(self, keypairs: &dyn SshKeypairs) -> ssh::Access {
        ssh::Access {
            port: self.port,
            user: self.user,
            authentication: ssh::Authentication::new(
                self.host_key,
                self.authentication.into_domain(keypairs),
            ),
            connect_timeout: self.connect_timeout,
        }
    }
}

pub enum AuthenticationCommand {
    Password(ssh::Password),
    ManagedKeypair(SshKeyAlgorithm),
    ImportedKeypair(ssh::PrivateKey),
}

impl AuthenticationCommand {
    pub fn into_domain(self, keypairs: &dyn SshKeypairs) -> ssh::AuthMethod {
        match self {
            AuthenticationCommand::Password(password) => ssh::AuthMethod::Password(password),
            AuthenticationCommand::ImportedKeypair(private_key) => ssh::AuthMethod::Keypair {
                private_key,
                source: ssh::KeySource::Imported,
            },
            AuthenticationCommand::ManagedKeypair(algorithm) => ssh::AuthMethod::Keypair {
                private_key: keypairs.generate(algorithm),
                source: ssh::KeySource::Managed,
            },
        }
    }
}
