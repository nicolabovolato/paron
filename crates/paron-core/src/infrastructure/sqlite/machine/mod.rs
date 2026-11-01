use secrecy::{ExposeSecret, SecretString};
use serde_with::hex::Hex;

use crate::domain::machine::{Connectivity, Host, Machine, MachineName, Protocols, ssh, tunnel};
use crate::domain::token_mac::TokenMac;
use crate::infrastructure::crypto::cipher::AeadCipher;
use crate::infrastructure::sqlite::EncryptedColumn;

pub mod repository;

extract_discriminant! {
    #[derive(sqlx::Type, Clone, Copy)]
    #[sqlx(rename_all = "snake_case")]
    enum ConnectivityKind from Connectivity { Direct, Tunneled }
}

extract_discriminant! {
    #[derive(sqlx::Type, Clone, Copy)]
    #[sqlx(rename_all = "snake_case")]
    enum TunnelEnrollmentKind from tunnel::Enrollment { NotEnrolled, Invited, Enrolled }
}

extract_discriminant! {
    #[derive(sqlx::Type, serde::Deserialize, Clone, Copy)]
    #[sqlx(rename_all = "snake_case")]
    #[serde(rename_all = "snake_case")]
    enum SshAuthMethodKind from ssh::AuthMethod { Password, Keypair }
}

extract_discriminant! {
    #[derive(sqlx::Type, serde::Deserialize, Clone, Copy)]
    #[sqlx(rename_all = "snake_case")]
    #[serde(rename_all = "snake_case")]
    enum SshKeySourceKind from ssh::KeySource { Managed, Imported }
}

impl From<SshKeySourceKind> for ssh::KeySource {
    fn from(kind: SshKeySourceKind) -> Self {
        match kind {
            SshKeySourceKind::Managed => Self::Managed,
            SshKeySourceKind::Imported => Self::Imported,
        }
    }
}

#[derive(sqlx::FromRow)]
struct MachineRow {
    id: uuid::Uuid,
    name: String,

    connectivity: ConnectivityKind,
    direct_host: Option<String>,
    tunnel_enrollment: Option<TunnelEnrollmentKind>,
    tunnel_token_mac: Option<Vec<u8>>,
    tunnel_token_expires_at_unix_ms: Option<i64>,
    tunnel_client_public_key: Option<Vec<u8>>,

    #[sqlx(json(nullable))]
    protocol_ssh: Option<ProtocolSshRow>,
}

#[serde_with::serde_as]
#[derive(serde::Deserialize)]
struct ProtocolSshRow {
    machine_id: uuid::Uuid,
    user: String,
    port: i64,
    #[serde_as(as = "Hex")]
    host_key: Vec<u8>,
    authentication_method: SshAuthMethodKind,
    #[serde_as(as = "Option<Hex>")]
    password_encrypted: Option<Vec<u8>>,
    #[serde_as(as = "Option<Hex>")]
    keypair_private_key_encrypted: Option<Vec<u8>>,
    keypair_source: Option<SshKeySourceKind>,
    connect_timeout_seconds: i64,
}

const NULL: &str = "a column required by the row's kind is null";

impl MachineRow {
    fn from_entity(machine: &Machine, cipher: &dyn AeadCipher) -> Self {
        let row = Self {
            id: machine.id,
            name: machine.name.to_string(),

            connectivity: ConnectivityKind::from(&machine.connectivity),
            direct_host: None,
            tunnel_enrollment: None,
            tunnel_token_mac: None,
            tunnel_token_expires_at_unix_ms: None,
            tunnel_client_public_key: None,

            protocol_ssh: machine
                .protocols
                .ssh
                .as_ref()
                .map(|access| ProtocolSshRow::from_entity(machine.id, access, cipher)),
        };

        match &machine.connectivity {
            Connectivity::Direct { host } => Self {
                direct_host: Some(host.to_string()),
                ..row
            },
            Connectivity::Tunneled(enrollment) => {
                let row = Self {
                    tunnel_enrollment: Some(TunnelEnrollmentKind::from(enrollment)),
                    ..row
                };

                match enrollment {
                    tunnel::Enrollment::NotEnrolled => row,
                    tunnel::Enrollment::Invited { mac, expires_at } => Self {
                        tunnel_token_mac: Some(mac.to_vec()),
                        tunnel_token_expires_at_unix_ms: Some(expires_at.as_millisecond()),
                        ..row
                    },
                    tunnel::Enrollment::Enrolled { client_public_key } => Self {
                        tunnel_client_public_key: Some(client_public_key.to_vec()),
                        ..row
                    },
                }
            }
        }
    }

    fn try_into_entity(
        self,
        cipher: &dyn AeadCipher,
    ) -> Result<Machine, Box<dyn std::error::Error + Send + Sync>> {
        use anyhow::Context as _;

        let connectivity = match self.connectivity {
            ConnectivityKind::Direct => Connectivity::Direct {
                host: Host::parse(&self.direct_host.context(NULL)?)?,
            },
            ConnectivityKind::Tunneled => {
                Connectivity::Tunneled(match self.tunnel_enrollment.context(NULL)? {
                    TunnelEnrollmentKind::NotEnrolled => tunnel::Enrollment::NotEnrolled,
                    TunnelEnrollmentKind::Invited => tunnel::Enrollment::Invited {
                        mac: TokenMac::from(self.tunnel_token_mac.context(NULL)?),
                        expires_at: jiff::Timestamp::from_millisecond(
                            self.tunnel_token_expires_at_unix_ms.context(NULL)?,
                        )?,
                    },
                    TunnelEnrollmentKind::Enrolled => tunnel::Enrollment::Enrolled {
                        client_public_key: tunnel::PublicKey::parse(
                            self.tunnel_client_public_key.context(NULL)?,
                        )?,
                    },
                })
            }
        };

        let ssh = self
            .protocol_ssh
            .map(|ssh| ssh.try_into_entity(cipher))
            .transpose()?;

        Ok(Machine {
            id: self.id,
            name: MachineName::parse(self.name)?,
            connectivity,
            protocols: Protocols { ssh },
        })
    }
}

impl ProtocolSshRow {
    fn from_entity(machine_id: uuid::Uuid, access: &ssh::Access, cipher: &dyn AeadCipher) -> Self {
        let encrypt = |column: EncryptedColumn, plaintext: &[u8]| {
            cipher.encrypt(&column.associated_data(machine_id), plaintext)
        };

        let row = Self {
            machine_id,
            user: access.user.to_string(),
            port: i64::from(access.port.get()),
            host_key: access.authentication.host_key().to_vec(),
            authentication_method: SshAuthMethodKind::from(access.authentication.method()),
            password_encrypted: None,
            keypair_private_key_encrypted: None,
            keypair_source: None,
            connect_timeout_seconds: i64::try_from(access.connect_timeout.as_secs())
                .unwrap_or(i64::MAX),
        };

        match access.authentication.method() {
            ssh::AuthMethod::Password(password) => Self {
                password_encrypted: Some(encrypt(
                    EncryptedColumn::SshPassword,
                    password.expose_secret().as_bytes(),
                )),
                ..row
            },
            ssh::AuthMethod::Keypair {
                private_key,
                source,
            } => Self {
                keypair_private_key_encrypted: Some(encrypt(
                    EncryptedColumn::SshPrivateKey,
                    private_key.expose_secret(),
                )),
                keypair_source: Some(SshKeySourceKind::from(source)),
                ..row
            },
        }
    }

    fn try_into_entity(
        self,
        cipher: &dyn AeadCipher,
    ) -> Result<ssh::Access, Box<dyn std::error::Error + Send + Sync>> {
        use anyhow::Context as _;

        let decrypt = |column: EncryptedColumn, ciphertext: &[u8]| {
            cipher.decrypt(&column.associated_data(self.machine_id), ciphertext)
        };

        let method = match self.authentication_method {
            SshAuthMethodKind::Password => {
                let password = decrypt(
                    EncryptedColumn::SshPassword,
                    &self.password_encrypted.context(NULL)?,
                )?;

                ssh::AuthMethod::Password(ssh::Password::parse(SecretString::from(
                    str::from_utf8(password.expose_secret())?,
                ))?)
            }
            SshAuthMethodKind::Keypair => ssh::AuthMethod::Keypair {
                private_key: ssh::PrivateKey::parse(decrypt(
                    EncryptedColumn::SshPrivateKey,
                    &self.keypair_private_key_encrypted.context(NULL)?,
                )?)?,
                source: self.keypair_source.context(NULL)?.into(),
            },
        };

        Ok(ssh::Access {
            user: ssh::User::parse(self.user)?,
            port: std::num::NonZeroU16::try_from(u16::try_from(self.port)?)?,
            authentication: ssh::Authentication::new(ssh::PublicKey::parse(self.host_key)?, method),
            connect_timeout: ssh::ConnectTimeout::new(std::time::Duration::from_secs(
                u64::try_from(self.connect_timeout_seconds)?,
            ))?,
        })
    }
}
