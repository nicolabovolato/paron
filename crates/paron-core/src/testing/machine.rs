use std::num::NonZeroU16;
use std::time::Duration;

use secrecy::SecretString;

use crate::domain::machine::{Connectivity, Host, Machine, MachineName, Protocols, ssh, tunnel};
use crate::domain::token_mac::TokenMac;
use crate::testing::{enrollment, keys};

const MACHINE_NAME: &str = "machine-1";
pub const DIRECT_HOST: &str = "10.0.0.1";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub const SSH_PORT: NonZeroU16 = NonZeroU16::new(22).unwrap();
pub const SSH_USER: &str = "deploy";
pub const SSH_PASSWORD: &str = "hunter2";

pub fn direct_machine() -> Machine {
    machine_builder().build()
}

pub fn unenrolled_machine() -> Machine {
    machine_builder()
        .tunneled(tunnel::Enrollment::NotEnrolled)
        .build()
}

pub fn invited_machine() -> Machine {
    machine_builder()
        .tunneled(tunnel::Enrollment::Invited {
            mac: TokenMac::from(enrollment::TOKEN_MAC.to_vec()),
            expires_at: enrollment::expires_at(),
        })
        .build()
}

pub fn enrolled_machine() -> Machine {
    machine_builder()
        .tunneled(tunnel::Enrollment::Enrolled {
            client_public_key: keys::tunnel_public_key(),
        })
        .build()
}

pub fn machine_builder() -> MachineBuilder {
    MachineBuilder {
        name: MachineName::parse(MACHINE_NAME).unwrap(),
        connectivity: Connectivity::Direct {
            host: Host::parse(DIRECT_HOST).unwrap(),
        },
        port: SSH_PORT,
        host_key: keys::ssh_public_key(),
        method: password_method(SSH_PASSWORD),
        connect_timeout: CONNECT_TIMEOUT,
        ssh: true,
    }
}

pub struct MachineBuilder {
    name: MachineName,
    connectivity: Connectivity,
    port: NonZeroU16,
    host_key: ssh::PublicKey,
    method: ssh::AuthMethod,
    connect_timeout: Duration,
    ssh: bool,
}

impl MachineBuilder {
    pub fn name(mut self, name: &str) -> Self {
        self.name = MachineName::parse(name).unwrap();
        self
    }

    pub fn direct_host(mut self, host: &str) -> Self {
        self.connectivity = Connectivity::Direct {
            host: Host::parse(host).unwrap(),
        };
        self
    }

    pub fn tunneled(mut self, enrollment: tunnel::Enrollment) -> Self {
        self.connectivity = Connectivity::Tunneled(enrollment);
        self
    }

    pub fn port(mut self, port: NonZeroU16) -> Self {
        self.port = port;
        self
    }

    pub fn host_key(mut self, host_key: ssh::PublicKey) -> Self {
        self.host_key = host_key;
        self
    }

    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    pub fn authentication(mut self, method: ssh::AuthMethod) -> Self {
        self.method = method;
        self
    }

    pub fn without_ssh(mut self) -> Self {
        self.ssh = false;
        self
    }

    pub fn build(self) -> Machine {
        Machine {
            id: uuid::Uuid::now_v7(),
            name: self.name,
            connectivity: self.connectivity,
            protocols: Protocols {
                ssh: self.ssh.then(|| ssh::Access {
                    user: ssh::User::parse(SSH_USER).unwrap(),
                    port: self.port,
                    authentication: ssh::Authentication::new(self.host_key, self.method),
                    connect_timeout: ssh::ConnectTimeout::new(self.connect_timeout).unwrap(),
                }),
            },
        }
    }
}

pub fn password_method(secret: &str) -> ssh::AuthMethod {
    ssh::AuthMethod::Password(ssh::Password::parse(SecretString::from(secret)).unwrap())
}

pub fn imported_keypair_method(private_key: ssh::PrivateKey) -> ssh::AuthMethod {
    ssh::AuthMethod::Keypair {
        private_key,
        source: ssh::KeySource::Imported,
    }
}

pub fn managed_keypair_method(private_key: ssh::PrivateKey) -> ssh::AuthMethod {
    ssh::AuthMethod::Keypair {
        private_key,
        source: ssh::KeySource::Managed,
    }
}

pub fn ssh_access(machine: &Machine) -> &ssh::Access {
    machine.protocols.ssh.as_ref().unwrap()
}
