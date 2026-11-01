use std::collections::BTreeSet;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::application::audit::AuditKind;
use crate::application::identity::event as identity;
use crate::application::machine::event as machine;
use crate::domain::user::{UserStatus, Username};
type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "payload")]
pub enum AuditPayload {
    #[serde(rename = "user.signed_in")]
    UserSignedIn {
        session_id: uuid::Uuid,
        methods: Vec<String>,
    },
    #[serde(rename = "user.signed_out")]
    UserSignedOut { session_id: uuid::Uuid },
    #[serde(rename = "user.created")]
    UserCreated { user: AuditUser },
    #[serde(rename = "user.updated")]
    UserUpdated { before: AuditUser, after: AuditUser },
    #[serde(rename = "user.deleted")]
    UserDeleted {},
    #[serde(rename = "user.recovery_started")]
    UserRecoveryStarted { expires_at: jiff::Timestamp },
    #[serde(rename = "user.credential_added")]
    UserCredentialAdded { kind: String },
    #[serde(rename = "user.credential_removed")]
    UserCredentialRemoved { kind: String },
    #[serde(rename = "machine.created")]
    MachineCreated {
        #[serde(with = "AuditMachine")]
        #[schema(value_type = AuditMachine)]
        machine: machine::MachineSnapshot,
    },
    #[serde(rename = "machine.updated")]
    MachineUpdated {
        #[serde(with = "AuditMachine")]
        #[schema(value_type = AuditMachine)]
        before: machine::MachineSnapshot,
        #[serde(with = "AuditMachine")]
        #[schema(value_type = AuditMachine)]
        after: machine::MachineSnapshot,
        credentials_replaced: BTreeSet<String>,
    },
    #[serde(rename = "machine.deleted")]
    MachineDeleted {},
    #[serde(rename = "machine.terminal_opened")]
    MachineTerminalOpened {
        session_id: uuid::Uuid,
        protocol: String,
    },
    #[serde(rename = "machine.terminal_closed")]
    MachineTerminalClosed { session_id: uuid::Uuid },
}

#[derive(Serialize, Deserialize)]
pub struct TypedPayload {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub payload: serde_json::Value,
}

impl From<&AuditKind> for TypedPayload {
    #[expect(
        clippy::expect_used,
        reason = "the payload is plain data, which serde_json encodes and decodes without failure"
    )]
    fn from(kind: &AuditKind) -> Self {
        serde_json::to_value(AuditPayload::from(kind))
            .and_then(serde_json::from_value)
            .expect("could not encode audit payload")
    }
}

impl TryFrom<&TypedPayload> for AuditKind {
    type Error = Error;

    fn try_from(typed: &TypedPayload) -> Result<Self, Error> {
        AuditPayload::deserialize(serde_json::to_value(typed)?)?.try_into()
    }
}

impl From<&AuditKind> for AuditPayload {
    fn from(kind: &AuditKind) -> Self {
        match kind {
            AuditKind::User(identity::AuditKind::SignedIn {
                session_id,
                methods,
            }) => Self::UserSignedIn {
                session_id: *session_id,
                methods: methods
                    .iter()
                    .map(|method| method.as_str().to_owned())
                    .collect(),
            },
            AuditKind::User(identity::AuditKind::SignedOut { session_id }) => Self::UserSignedOut {
                session_id: *session_id,
            },
            AuditKind::User(identity::AuditKind::Created(user)) => {
                Self::UserCreated { user: user.into() }
            }
            AuditKind::User(identity::AuditKind::Updated { before, after }) => Self::UserUpdated {
                before: before.into(),
                after: after.into(),
            },
            AuditKind::User(identity::AuditKind::Deleted) => Self::UserDeleted {},
            AuditKind::User(identity::AuditKind::RecoveryStarted { expires_at }) => {
                Self::UserRecoveryStarted {
                    expires_at: *expires_at,
                }
            }
            AuditKind::User(identity::AuditKind::CredentialAdded { kind }) => {
                Self::UserCredentialAdded {
                    kind: kind.as_str().to_owned(),
                }
            }
            AuditKind::User(identity::AuditKind::CredentialRemoved { kind }) => {
                Self::UserCredentialRemoved {
                    kind: kind.as_str().to_owned(),
                }
            }
            AuditKind::Machine(machine::AuditKind::Created(machine)) => Self::MachineCreated {
                machine: machine.clone(),
            },
            AuditKind::Machine(machine::AuditKind::Updated {
                before,
                after,
                credentials_replaced,
            }) => Self::MachineUpdated {
                before: before.clone(),
                after: after.clone(),
                credentials_replaced: credentials_replaced
                    .iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
            },
            AuditKind::Machine(machine::AuditKind::Deleted) => Self::MachineDeleted {},
            AuditKind::Machine(machine::AuditKind::TerminalOpened {
                session_id,
                protocol,
            }) => Self::MachineTerminalOpened {
                session_id: *session_id,
                protocol: protocol.as_str().to_owned(),
            },
            AuditKind::Machine(machine::AuditKind::TerminalClosed { session_id }) => {
                Self::MachineTerminalClosed {
                    session_id: *session_id,
                }
            }
        }
    }
}

impl TryFrom<AuditPayload> for AuditKind {
    type Error = Error;

    fn try_from(payload: AuditPayload) -> Result<Self, Error> {
        Ok(match payload {
            AuditPayload::UserSignedIn {
                session_id,
                methods,
            } => identity::AuditKind::SignedIn {
                session_id,
                methods: methods
                    .iter()
                    .map(|method| method.parse())
                    .collect::<Result<_, _>>()?,
            }
            .into(),
            AuditPayload::UserSignedOut { session_id } => {
                identity::AuditKind::SignedOut { session_id }.into()
            }
            AuditPayload::UserCreated { user } => {
                identity::AuditKind::Created(user.try_into()?).into()
            }
            AuditPayload::UserUpdated { before, after } => identity::AuditKind::Updated {
                before: before.try_into()?,
                after: after.try_into()?,
            }
            .into(),
            AuditPayload::UserDeleted {} => identity::AuditKind::Deleted.into(),
            AuditPayload::UserRecoveryStarted { expires_at } => {
                identity::AuditKind::RecoveryStarted { expires_at }.into()
            }
            AuditPayload::UserCredentialAdded { kind } => identity::AuditKind::CredentialAdded {
                kind: kind.parse()?,
            }
            .into(),
            AuditPayload::UserCredentialRemoved { kind } => {
                identity::AuditKind::CredentialRemoved {
                    kind: kind.parse()?,
                }
                .into()
            }
            AuditPayload::MachineCreated { machine } => machine::AuditKind::Created(machine).into(),
            AuditPayload::MachineUpdated {
                before,
                after,
                credentials_replaced,
            } => machine::AuditKind::Updated {
                before,
                after,
                credentials_replaced: credentials_replaced
                    .iter()
                    .map(|kind| kind.parse())
                    .collect::<Result<_, _>>()?,
            }
            .into(),
            AuditPayload::MachineDeleted {} => machine::AuditKind::Deleted.into(),
            AuditPayload::MachineTerminalOpened {
                session_id,
                protocol,
            } => machine::AuditKind::TerminalOpened {
                session_id,
                protocol: protocol.parse()?,
            }
            .into(),
            AuditPayload::MachineTerminalClosed { session_id } => {
                machine::AuditKind::TerminalClosed { session_id }.into()
            }
        })
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(remote = "UserStatus", rename_all = "snake_case")]
enum AuditUserStatus {
    Active,
    Suspended,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct AuditUser {
    username: String,
    display_name: String,
    roles: Vec<String>,
    #[serde(with = "AuditUserStatus")]
    #[schema(value_type = AuditUserStatus)]
    status: UserStatus,
}

impl From<&identity::UserSnapshot> for AuditUser {
    fn from(user: &identity::UserSnapshot) -> Self {
        Self {
            username: user.username.to_string(),
            display_name: user.display_name.clone(),
            roles: user
                .roles
                .iter()
                .map(|role| role.as_str().to_owned())
                .collect(),
            status: user.status,
        }
    }
}

impl TryFrom<AuditUser> for identity::UserSnapshot {
    type Error = Error;

    fn try_from(stored: AuditUser) -> Result<Self, Error> {
        Ok(Self {
            username: Username::parse(&stored.username)?,
            display_name: stored.display_name,
            roles: stored
                .roles
                .iter()
                .map(|role| role.parse())
                .collect::<Result<_, _>>()?,
            status: stored.status,
        })
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(remote = "machine::AuthenticationMethod", rename_all = "snake_case")]
enum AuditAuthenticationMethod {
    Password,
    ManagedKeypair,
    ImportedKeypair,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(
    remote = "machine::EnrollmentSnapshot",
    tag = "state",
    rename_all = "snake_case"
)]
enum AuditEnrollment {
    NotEnrolled,
    Invited { expires_at: jiff::Timestamp },
    Enrolled,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(
    remote = "machine::ConnectivitySnapshot",
    tag = "kind",
    rename_all = "snake_case"
)]
enum AuditConnectivity {
    Direct {
        host: String,
    },
    Tunneled {
        #[serde(with = "AuditEnrollment")]
        #[schema(value_type = AuditEnrollment)]
        enrollment: machine::EnrollmentSnapshot,
    },
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(remote = "machine::SshSnapshot")]
struct AuditSsh {
    user: String,
    #[schema(value_type = u16)]
    port: NonZeroU16,
    host_key_fingerprint: String,
    #[serde(with = "AuditAuthenticationMethod")]
    #[schema(value_type = AuditAuthenticationMethod)]
    authentication: machine::AuthenticationMethod,
}

#[derive(Serialize, Deserialize)]
struct AuditSshValue(#[serde(with = "AuditSsh")] machine::SshSnapshot);

mod optional_ssh {
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    use super::{AuditSshValue, machine};

    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(
        ssh: &Option<machine::SshSnapshot>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ssh.clone().map(AuditSshValue).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<machine::SshSnapshot>, D::Error> {
        Ok(Option::<AuditSshValue>::deserialize(deserializer)?.map(|AuditSshValue(ssh)| ssh))
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(remote = "machine::ProtocolsSnapshot")]
struct AuditProtocols {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_ssh"
    )]
    #[schema(value_type = Option<AuditSsh>)]
    ssh: Option<machine::SshSnapshot>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(remote = "machine::MachineSnapshot")]
struct AuditMachine {
    name: String,
    #[serde(with = "AuditConnectivity")]
    #[schema(value_type = AuditConnectivity)]
    connectivity: machine::ConnectivitySnapshot,
    #[serde(with = "AuditProtocols")]
    #[schema(value_type = AuditProtocols)]
    protocols: machine::ProtocolsSnapshot,
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::application::audit::AuditEntryType;
    use crate::application::identity::authentication::methods::AuthenticationMethod;
    use crate::domain::machine::ProtocolKind;
    use crate::domain::user::Role;

    const SESSION: uuid::Uuid = uuid::Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0002);

    fn user_snapshot() -> identity::UserSnapshot {
        identity::UserSnapshot {
            username: Username::parse("nicola").unwrap(),
            display_name: "Nicola".to_owned(),
            roles: BTreeSet::from([Role::Administrator, Role::Operator]),
            status: UserStatus::Active,
        }
    }

    fn machine_snapshot() -> machine::MachineSnapshot {
        machine::MachineSnapshot {
            name: "build".to_owned(),
            connectivity: machine::ConnectivitySnapshot::Tunneled {
                enrollment: machine::EnrollmentSnapshot::Invited {
                    expires_at: jiff::Timestamp::UNIX_EPOCH,
                },
            },
            protocols: machine::ProtocolsSnapshot {
                ssh: Some(machine::SshSnapshot {
                    user: "deploy".to_owned(),
                    port: NonZeroU16::new(22).unwrap(),
                    host_key_fingerprint: "SHA256:abc".to_owned(),
                    authentication: machine::AuthenticationMethod::Password,
                }),
            },
        }
    }

    type KindWithStoredForm = (AuditKind, &'static str, serde_json::Value);

    fn user_kinds_with_their_stored_form() -> Vec<KindWithStoredForm> {
        let session = SESSION.to_string();
        let stored_user = json!({
            "username": "nicola",
            "display_name": "Nicola",
            "roles": ["administrator", "operator"],
            "status": "active",
        });
        vec![
            (
                identity::AuditKind::SignedIn {
                    session_id: SESSION,
                    methods: vec![AuthenticationMethod::Password, AuthenticationMethod::Totp],
                }
                .into(),
                "user.signed_in",
                json!({ "session_id": session, "methods": ["password", "totp"] }),
            ),
            (
                identity::AuditKind::SignedOut {
                    session_id: SESSION,
                }
                .into(),
                "user.signed_out",
                json!({ "session_id": session }),
            ),
            (
                identity::AuditKind::Created(user_snapshot()).into(),
                "user.created",
                json!({ "user": stored_user }),
            ),
            (
                identity::AuditKind::Updated {
                    before: user_snapshot(),
                    after: identity::UserSnapshot {
                        status: UserStatus::Suspended,
                        ..user_snapshot()
                    },
                }
                .into(),
                "user.updated",
                json!({
                    "before": stored_user,
                    "after": {
                        "username": "nicola",
                        "display_name": "Nicola",
                        "roles": ["administrator", "operator"],
                        "status": "suspended",
                    },
                }),
            ),
            (
                identity::AuditKind::Deleted.into(),
                "user.deleted",
                json!({}),
            ),
            (
                identity::AuditKind::RecoveryStarted {
                    expires_at: jiff::Timestamp::UNIX_EPOCH,
                }
                .into(),
                "user.recovery_started",
                json!({ "expires_at": "1970-01-01T00:00:00Z" }),
            ),
            (
                identity::AuditKind::CredentialAdded {
                    kind: AuthenticationMethod::Passkey,
                }
                .into(),
                "user.credential_added",
                json!({ "kind": "passkey" }),
            ),
            (
                identity::AuditKind::CredentialRemoved {
                    kind: AuthenticationMethod::Totp,
                }
                .into(),
                "user.credential_removed",
                json!({ "kind": "totp" }),
            ),
        ]
    }

    fn machine_kinds_with_their_stored_form() -> Vec<KindWithStoredForm> {
        let session = SESSION.to_string();
        let stored_machine = json!({
            "name": "build",
            "connectivity": {
                "kind": "tunneled",
                "enrollment": { "state": "invited", "expires_at": "1970-01-01T00:00:00Z" },
            },
            "protocols": {
                "ssh": {
                    "user": "deploy",
                    "port": 22,
                    "host_key_fingerprint": "SHA256:abc",
                    "authentication": "password",
                },
            },
        });

        vec![
            (
                machine::AuditKind::Created(machine_snapshot()).into(),
                "machine.created",
                json!({ "machine": stored_machine }),
            ),
            (
                machine::AuditKind::Updated {
                    before: machine_snapshot(),
                    after: machine::MachineSnapshot {
                        name: "builder".to_owned(),
                        connectivity: machine::ConnectivitySnapshot::Direct {
                            host: "10.0.0.1".to_owned(),
                        },
                        protocols: machine::ProtocolsSnapshot { ssh: None },
                    },
                    credentials_replaced: BTreeSet::from([ProtocolKind::Ssh]),
                }
                .into(),
                "machine.updated",
                json!({
                    "before": stored_machine,
                    "after": {
                        "name": "builder",
                        "connectivity": { "kind": "direct", "host": "10.0.0.1" },
                        "protocols": {},
                    },
                    "credentials_replaced": ["ssh"],
                }),
            ),
            (
                machine::AuditKind::Deleted.into(),
                "machine.deleted",
                json!({}),
            ),
            (
                machine::AuditKind::TerminalOpened {
                    session_id: SESSION,
                    protocol: ProtocolKind::Ssh,
                }
                .into(),
                "machine.terminal_opened",
                json!({ "session_id": session, "protocol": "ssh" }),
            ),
            (
                machine::AuditKind::TerminalClosed {
                    session_id: SESSION,
                }
                .into(),
                "machine.terminal_closed",
                json!({ "session_id": session }),
            ),
        ]
    }

    #[test]
    fn from_each_kind_returns_its_stored_type_and_payload() {
        for (kind, entry_type, payload) in user_kinds_with_their_stored_form()
            .into_iter()
            .chain(machine_kinds_with_their_stored_form())
        {
            let typed = TypedPayload::from(&kind);

            assert_eq!(typed.entry_type, entry_type);
            assert_eq!(typed.payload, payload, "{entry_type}");
        }
    }

    #[test]
    fn entry_types_are_the_stored_types() {
        let stored_types: BTreeSet<&str> = user_kinds_with_their_stored_form()
            .into_iter()
            .chain(machine_kinds_with_their_stored_form())
            .map(|(_, entry_type, _)| entry_type)
            .collect();

        let entry_types: BTreeSet<&str> = AuditEntryType::ALL
            .iter()
            .map(|entry_type| entry_type.as_str())
            .collect();

        assert_eq!(entry_types, stored_types);
    }

    #[test]
    fn try_from_each_stored_type_returns_the_recorded_kind() {
        for (kind, entry_type, payload) in user_kinds_with_their_stored_form()
            .into_iter()
            .chain(machine_kinds_with_their_stored_form())
        {
            let recorded_kind = AuditKind::try_from(&TypedPayload {
                entry_type: entry_type.to_owned(),
                payload,
            });

            assert_eq!(recorded_kind.unwrap(), kind, "{entry_type}");
        }
    }

    #[test]
    fn try_from_payload_with_unknown_field_ignores_it() {
        let kind = AuditKind::try_from(&TypedPayload {
            entry_type: "user.deleted".to_owned(),
            payload: json!({ "added_by_a_later_release": true }),
        });

        assert_eq!(kind.unwrap(), identity::AuditKind::Deleted.into());
    }

    #[test]
    fn try_from_unknown_type_returns_error() {
        let kind = AuditKind::try_from(&TypedPayload {
            entry_type: "user.from_a_later_release".to_owned(),
            payload: json!({ "any": "thing" }),
        });

        assert_matches!(kind, Err(_));
    }

    #[test]
    fn try_from_payload_that_does_not_fit_its_type_returns_error() {
        let kind = AuditKind::try_from(&TypedPayload {
            entry_type: "user.signed_in".to_owned(),
            payload: json!({}),
        });

        assert_matches!(kind, Err(_));
    }

    #[test]
    fn try_from_role_that_no_longer_exists_returns_error() {
        let kind = AuditKind::try_from(&TypedPayload {
            entry_type: "user.created".to_owned(),
            payload: json!({
                "user": {
                    "username": "nicola",
                    "display_name": "Nicola",
                    "roles": ["a_role_that_was_removed"],
                    "status": "active",
                },
            }),
        });

        assert_matches!(kind, Err(_));
    }

    #[test]
    fn try_from_protocol_that_no_longer_exists_returns_error() {
        let kind = AuditKind::try_from(&TypedPayload {
            entry_type: "machine.terminal_opened".to_owned(),
            payload: json!({
                "session_id": SESSION.to_string(),
                "protocol": "a_protocol_that_was_removed",
            }),
        });

        assert_matches!(kind, Err(_));
    }
}
