mod websocket;

use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::schema::{ObjectBuilder, Type};

use crate::application::machine::{
    AuthenticationCommand, ConnectivityCommand, CreateMachineCommand, CreateMachineError,
    DeleteMachineError, GetMachineError, IssueEnrollmentTokenCommand, IssueEnrollmentTokenError,
    ListMachinesError, ProtocolUpdate, ProtocolsCommand, SshCommand, TestConnectionCommand,
    TestConnectionError, TransportError, UnenrollError, UpdateMachineCommand, UpdateMachineError,
    UpdateProtocolsCommand, UpdateSshCommand, test_connection,
};
use crate::domain::machine::{
    self, Capabilities, Connectivity, Machine, ProtocolKind, Provider, ssh, tunnel,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::infrastructure::http::auth::CurrentUser;
use crate::infrastructure::http::error;
use crate::infrastructure::http::error::ApiError;
use crate::infrastructure::http::pagination::{ListParams, PageResponse};
use crate::infrastructure::http::state::Deps;
use serde_utoipa_validate::{Patch, newtype, validate};

pub fn routes() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_by_id, update, delete))
        .routes(routes!(test_connection))
        .routes(routes!(test_new_connection))
        .routes(routes!(issue_enrollment_token, unenroll))
        .merge(websocket::routes())
}

#[utoipa::path(
    get,
    path = "/machines",
    tag = "machines",
    operation_id = "list_machines",
    params(ListParams),
    responses((status = OK, body = PageResponse<MachineResponse>)),
    security(("session" = [])),
)]
async fn list(
    State(deps): State<Deps>,
    user: CurrentUser,
    Query(query): Query<ListParams>,
) -> error::Result<impl IntoResponse> {
    let query = query.validated()?;
    let machines = deps.machines.list(&user.into(), &query.into()).await?;

    Ok(Json(PageResponse::<MachineResponse>::from(machines)))
}

#[utoipa::path(
    get,
    path = "/machines/{id}",
    tag = "machines",
    operation_id = "read_machine",
    responses((status = OK, body = MachineResponse)),
    security(("session" = [])),
)]
async fn get_by_id(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    let machine = deps.machines.get(&user.into(), id).await?;

    Ok(Json(MachineResponse::from(machine)))
}

#[utoipa::path(
    post,
    path = "/machines",
    tag = "machines",
    operation_id = "create_machine",
    responses((status = CREATED, body = MachineResponse)),
    security(("session" = [])),
)]
async fn create(
    State(deps): State<Deps>,
    user: CurrentUser,
    Json(body): Json<MachineRequest>,
) -> error::Result<impl IntoResponse> {
    let body = body.validated()?;

    let created = deps.machines.create(&user.into(), body.into()).await?;

    Ok((StatusCode::CREATED, Json(MachineResponse::from(created))))
}

#[utoipa::path(
    patch,
    path = "/machines/{id}",
    tag = "machines",
    operation_id = "update_machine",
    responses((status = OK, body = MachineResponse)),
    security(("session" = [])),
)]
async fn update(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<PartialUpdateMachineRequest>,
) -> error::Result<impl IntoResponse> {
    let body = body.validated()?;

    let updated = deps
        .machines
        .update(&user.into(), body.into_command(id))
        .await?;

    Ok(Json(MachineResponse::from(updated)))
}

#[utoipa::path(
    delete,
    path = "/machines/{id}",
    tag = "machines",
    operation_id = "remove_machine",
    responses((status = NO_CONTENT)),
    security(("session" = [])),
)]
async fn delete(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    deps.machines.delete(&user.into(), id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/machines/{id}/test-connection",
    tag = "machines",
    operation_id = "test_machine_connection",
    params(ProtocolParams),
    responses((status = NO_CONTENT)),
    security(("session" = [])),
)]
async fn test_connection(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
    Query(params): Query<ProtocolParams>,
    body: Option<Json<PartialUpdateMachineRequest>>,
) -> error::Result<impl IntoResponse> {
    let protocol = params.validated()?.protocol;
    let command = TestConnectionCommand {
        protocol,
        machine: match body {
            None => test_connection::Machine::Stored(id),
            Some(Json(body)) => {
                test_connection::Machine::Changed(body.validated()?.into_command(id))
            }
        },
    };

    deps.machine_connections.test(&user.into(), command).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/machines/test-connection",
    tag = "machines",
    operation_id = "test_new_machine_connection",
    params(ProtocolParams),
    responses((status = NO_CONTENT)),
    security(("session" = [])),
)]
async fn test_new_connection(
    State(deps): State<Deps>,
    user: CurrentUser,
    Query(params): Query<ProtocolParams>,
    Json(body): Json<MachineRequest>,
) -> error::Result<impl IntoResponse> {
    let protocol = params.validated()?.protocol;
    let command = TestConnectionCommand {
        protocol,
        machine: test_connection::Machine::New(body.validated()?.into()),
    };

    deps.machine_connections.test(&user.into(), command).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/machines/{id}/enrollment",
    tag = "machines",
    operation_id = "issue_machine_enrollment_token",
    responses((status = OK, body = EnrollmentInvitationResponse)),
    security(("session" = [])),
)]
async fn issue_enrollment_token(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    let issued = deps
        .machine_enrollment
        .issue_enrollment_token(&user.into(), IssueEnrollmentTokenCommand { machine_id: id })
        .await?;

    Ok(Json(EnrollmentInvitationResponse {
        token: issued.token.url().expose_secret().to_owned(),
        expires_at: issued.expires_at,
    }))
}

#[utoipa::path(
    delete,
    path = "/machines/{id}/enrollment",
    tag = "machines",
    operation_id = "withdraw_machine_enrollment",
    responses((status = NO_CONTENT)),
    security(("session" = [])),
)]
async fn unenroll(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    deps.machine_enrollment.unenroll(&user.into(), id).await?;

    Ok(StatusCode::NO_CONTENT)
}

newtype! {
    pub Host: String -> machine::Host {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some("A hostname or IP address, without a scheme or a path."))
            .max_length(Some(machine::Host::MAX))
            .pattern(Some(r"^[^\s/\\@?#]+$"))
            .extensions(Some(
                ExtensionsBuilder::new()
                    .add("x-pattern-message", "A hostname or IP address, without a scheme or a path.")
                    .build(),
            )),
        deserialize: |raw: String| machine::Host::parse(&raw),
        serialize: |host: &machine::Host| Ok(host.to_string()),
    }
}

newtype! {
    pub MachineName: String -> machine::MachineName {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(machine::MachineName::MIN))
            .max_length(Some(machine::MachineName::MAX)),
        deserialize: |raw: String| machine::MachineName::parse(raw),
        serialize: |name: &machine::MachineName| Ok(name.to_string()),
    }
}

newtype! {
    pub SshUser: String -> ssh::User {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(ssh::User::MIN))
            .max_length(Some(ssh::User::MAX)),
        deserialize: |raw: String| ssh::User::parse(raw),
        serialize: |user: &ssh::User| Ok(user.to_string()),
    }
}

newtype! {
    pub Protocol: String -> ProtocolKind {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(ProtocolKind::ALL.iter().map(|kind| kind.as_str()))),
        deserialize: |raw: String| raw.parse::<ProtocolKind>(),
        serialize: |kind: &ProtocolKind| Ok(kind.as_str()),
    }
}

newtype! {
    pub Port: u64 -> std::num::NonZeroU16 {
        schema: ObjectBuilder::new()
            .schema_type(Type::Integer)
            .minimum(Some(1))
            .maximum(Some(u16::MAX)),
        deserialize: |raw: u64| {
            u16::try_from(raw)
                .ok()
                .and_then(std::num::NonZeroU16::new)
                .ok_or("expected an integer between 1 and 65535")
        },
        serialize: |port: &std::num::NonZeroU16| Ok(port.get()),
    }
}

newtype! {
    pub ConnectTimeout: u64 -> ssh::ConnectTimeout {
        schema: ObjectBuilder::new()
            .schema_type(Type::Integer)
            .minimum(Some(ssh::ConnectTimeout::MIN.as_secs()))
            .maximum(Some(ssh::ConnectTimeout::MAX.as_secs())),
        deserialize: |raw: u64| {
            ssh::ConnectTimeout::new(std::time::Duration::from_secs(raw))
                .map_err(|invalid| invalid.to_string())
        },
        serialize: |timeout: &ssh::ConnectTimeout| Ok(timeout.as_secs()),
    }
}

newtype! {
    pub PublicKey: String -> ssh::PublicKey {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(
                "An SSH public key, exactly as `ssh-keyscan` prints it: `ssh-ed25519 AAAA...`.",
            ))
            .pattern(Some(r"^(ssh|ecdsa|sk)-\S+\s+\S+"))
            .extensions(Some(
                ExtensionsBuilder::new()
                    .add("x-pattern-message", "An SSH public key, exactly as `ssh-keyscan` prints it: `ssh-ed25519 AAAA...`.")
                    .build(),
            )),
        deserialize: |raw: String| {
            let bytes = ssh_key::PublicKey::from_openssh(&raw)
                .and_then(|key| key.to_bytes())
                .map_err(|_| "expected an SSH public key, as in `ssh-ed25519 AAAA...`".to_owned())?;

            ssh::PublicKey::parse(bytes).map_err(|error| error.to_string())
        },
        serialize: |key: &ssh::PublicKey| {
            ssh_key::PublicKey::from_bytes(key)
                .and_then(|key| key.to_openssh())
                .map_err(|error| error.to_string())
        },
    }
}

newtype! {
    pub SshPassword: String -> ssh::Password {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(ssh::Password::MIN))
            .max_length(Some(ssh::Password::MAX)),
        deserialize: |raw: String| ssh::Password::parse(SecretString::from(raw)),
    }
}

newtype! {
    pub PrivateKey: String -> ssh::PrivateKey {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(
                "An OpenSSH private key, exactly as `ssh-keygen` wrote the file. Must not be passphrase-protected.",
            ))
            .pattern(Some(r"^-----BEGIN OPENSSH PRIVATE KEY-----"))
            .extensions(Some(
                ExtensionsBuilder::new()
                    .add("x-pattern-message", "An OpenSSH private key, exactly as `ssh-keygen` wrote the file. Must not be passphrase-protected.")
                    .build(),
            )),
        deserialize: |raw: String| {
            let bytes = ssh_key::PrivateKey::from_openssh(&raw)
                .and_then(|key| key.to_bytes())
                .map_err(|_| {
                    "expected an OpenSSH private key, beginning \
                     `-----BEGIN OPENSSH PRIVATE KEY-----`"
                        .to_owned()
                })?;

            ssh::PrivateKey::parse(secrecy::SecretSlice::from(bytes.to_vec()))
                .map_err(|e| e.to_string())
        },
    }
}

newtype! {
    pub KeySource: String -> ssh::KeySource {
        Managed => "managed",
        Imported => "imported",
    }
}

newtype! {
    pub KeyAlgorithm: String -> crate::application::machine::SshKeyAlgorithm {
        Ed25519 => "ed25519",
    }
}

newtype! {
    pub ClientPublicKey: String -> tunnel::PublicKey {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some("The key the machine presented when it enrolled.")),
        serialize: |key: &tunnel::PublicKey| Ok(base16ct::lower::encode_string(key)),
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProtocolParams {
    pub protocol: Protocol,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct MachineRequest {
    pub name: MachineName,
    pub connectivity: ConnectivityRequest,
    pub protocols: ProtocolsRequest,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectivityRequest {
    Direct { host: Host },
    Tunneled,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProtocolsRequest {
    pub ssh: Option<SshRequest>,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct SshRequest {
    pub user: SshUser,
    pub port: Port,
    pub authentication: SshAuthenticationRequest,
    pub connect_timeout_seconds: ConnectTimeout,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct SshAuthenticationRequest {
    pub host_key: PublicKey,
    pub method: SshAuthMethodRequest,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SshAuthMethodRequest {
    Password { password: SshPassword },
    Keypair { source: KeySourceRequest },
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeySourceRequest {
    Managed { algorithm: KeyAlgorithm },
    Imported { private_key: PrivateKey },
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartialUpdateMachineRequest {
    pub name: Patch<MachineName>,
    pub connectivity: Patch<ConnectivityRequest>,
    pub protocols: Patch<PartialProtocolsRequest>,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartialProtocolsRequest {
    pub ssh: Patch<Option<PartialSshRequest>>,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartialSshRequest {
    pub user: Patch<SshUser>,
    pub port: Patch<Port>,
    pub authentication: Patch<PartialSshAuthenticationRequest>,
    pub connect_timeout_seconds: Patch<ConnectTimeout>,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartialSshAuthenticationRequest {
    pub host_key: Patch<PublicKey>,
    pub method: Patch<SshAuthMethodRequest>,
}

impl From<MachineRequestParsed> for CreateMachineCommand {
    fn from(form: MachineRequestParsed) -> Self {
        Self {
            name: form.name,
            connectivity: form.connectivity.into(),
            protocols: ProtocolsCommand {
                ssh: form.protocols.ssh.map(Into::into),
            },
        }
    }
}

impl From<ConnectivityRequestParsed> for ConnectivityCommand {
    fn from(form: ConnectivityRequestParsed) -> Self {
        match form {
            ConnectivityRequestParsed::Direct { host } => Self::Direct { host },
            ConnectivityRequestParsed::Tunneled => Self::Tunneled,
        }
    }
}

impl From<SshRequestParsed> for SshCommand {
    fn from(form: SshRequestParsed) -> Self {
        Self {
            user: form.user,
            port: form.port,
            host_key: form.authentication.host_key,
            authentication: form.authentication.method.into(),
            connect_timeout: form.connect_timeout_seconds,
        }
    }
}

impl From<SshAuthMethodRequestParsed> for AuthenticationCommand {
    fn from(form: SshAuthMethodRequestParsed) -> Self {
        match form {
            SshAuthMethodRequestParsed::Password { password } => Self::Password(password),
            SshAuthMethodRequestParsed::Keypair {
                source: KeySourceRequestParsed::Managed { algorithm },
            } => Self::ManagedKeypair(algorithm),
            SshAuthMethodRequestParsed::Keypair {
                source: KeySourceRequestParsed::Imported { private_key },
            } => Self::ImportedKeypair(private_key),
        }
    }
}

impl PartialUpdateMachineRequestParsed {
    fn into_command(self, id: uuid::Uuid) -> UpdateMachineCommand {
        let ssh = self
            .protocols
            .and_then(|protocols| protocols.ssh)
            .map(|ssh| match ssh {
                None => ProtocolUpdate::Remove,
                Some(ssh) => ProtocolUpdate::Set(ssh.into()),
            });

        UpdateMachineCommand {
            id,
            name: self.name,
            connectivity: self.connectivity.map(Into::into),
            protocols: UpdateProtocolsCommand { ssh },
        }
    }
}

impl From<PartialSshRequestParsed> for UpdateSshCommand {
    fn from(form: PartialSshRequestParsed) -> Self {
        let (host_key, method) = match form.authentication {
            Some(authentication) => (authentication.host_key, authentication.method),
            None => (None, None),
        };

        Self {
            user: form.user,
            port: form.port,
            host_key,
            authentication: method.map(Into::into),
            connect_timeout: form.connect_timeout_seconds,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MachineResponse {
    pub id: uuid::Uuid,
    pub name: MachineName,
    pub connectivity: ConnectivityResponse,
    pub protocols: ProtocolsResponse,
    pub capabilities: CapabilitiesResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectivityResponse {
    Direct { host: Host },
    Tunneled { enrollment: EnrollmentResponse },
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EnrollmentResponse {
    NotEnrolled,
    Invited { expires_at: jiff::Timestamp },
    Enrolled { public_key: ClientPublicKey },
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProtocolsResponse {
    pub ssh: Option<SshResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SshResponse {
    pub user: SshUser,
    pub port: Port,
    pub authentication: SshAuthenticationResponse,
    pub connect_timeout_seconds: ConnectTimeout,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SshAuthenticationResponse {
    pub host_key: PublicKey,
    pub method: SshAuthMethodResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SshAuthMethodResponse {
    Password,
    Keypair {
        source: KeySource,
        public_key: PublicKey,
    },
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CapabilitiesResponse {
    pub connection_test: Vec<ProviderResponse>,
    pub terminal: Vec<ProviderResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderResponse {
    Protocol { protocol: Protocol },
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentInvitationResponse {
    pub token: String,
    pub expires_at: jiff::Timestamp,
}

impl From<Capabilities> for CapabilitiesResponse {
    fn from(capabilities: Capabilities) -> Self {
        Self {
            connection_test: capabilities
                .connection_test
                .into_iter()
                .map(Into::into)
                .collect(),
            terminal: capabilities.terminal.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Provider> for ProviderResponse {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::Protocol(kind) => Self::Protocol {
                protocol: Protocol::from(kind),
            },
        }
    }
}

impl From<Machine> for MachineResponse {
    fn from(machine: Machine) -> Self {
        Self {
            capabilities: CapabilitiesResponse::from(machine.capabilities()),
            id: machine.id,
            name: machine.name.into(),
            connectivity: match machine.connectivity {
                Connectivity::Direct { host } => ConnectivityResponse::Direct {
                    host: Host::from(host),
                },
                Connectivity::Tunneled(enrollment) => ConnectivityResponse::Tunneled {
                    enrollment: match enrollment {
                        tunnel::Enrollment::NotEnrolled => EnrollmentResponse::NotEnrolled,
                        tunnel::Enrollment::Invited { expires_at, .. } => {
                            EnrollmentResponse::Invited { expires_at }
                        }
                        tunnel::Enrollment::Enrolled { client_public_key } => {
                            EnrollmentResponse::Enrolled {
                                public_key: ClientPublicKey::from(client_public_key),
                            }
                        }
                    },
                },
            },
            protocols: ProtocolsResponse {
                ssh: machine.protocols.ssh.map(SshResponse::from),
            },
        }
    }
}

impl From<ssh::Access> for SshResponse {
    fn from(access: ssh::Access) -> Self {
        Self {
            user: access.user.into(),
            port: Port::from(access.port),
            connect_timeout_seconds: ConnectTimeout::from(access.connect_timeout),
            authentication: SshAuthenticationResponse {
                host_key: PublicKey::from(access.authentication.host_key().clone()),
                method: match access.authentication.method() {
                    ssh::AuthMethod::Password(_) => SshAuthMethodResponse::Password,
                    ssh::AuthMethod::Keypair {
                        private_key,
                        source,
                    } => SshAuthMethodResponse::Keypair {
                        public_key: PublicKey::from(private_key.public_key()),
                        source: KeySource::from(*source),
                    },
                },
            },
        }
    }
}

impl From<TransportError> for ApiError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::NotEnrolled | TransportError::AuthenticationFailed => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            TransportError::Unreachable(_)
            | TransportError::HostKeyMismatch
            | TransportError::TerminalRefused => {
                Self::detail(StatusCode::BAD_GATEWAY, error.to_string())
            }
            TransportError::Unexpected(cause) => anyhow::Error::from_boxed(cause).into(),
        }
    }
}

impl From<ListMachinesError> for ApiError {
    fn from(error: ListMachinesError) -> Self {
        match error {
            ListMachinesError::Denied(denied) => denied.into(),
            ListMachinesError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<GetMachineError> for ApiError {
    fn from(error: GetMachineError) -> Self {
        match error {
            GetMachineError::Denied(denied) => denied.into(),
            GetMachineError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            GetMachineError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<CreateMachineError> for ApiError {
    fn from(error: CreateMachineError) -> Self {
        match error {
            CreateMachineError::Denied(denied) => denied.into(),
            CreateMachineError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<UpdateMachineError> for ApiError {
    fn from(error: UpdateMachineError) -> Self {
        match error {
            UpdateMachineError::Denied(denied) => denied.into(),
            UpdateMachineError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            UpdateMachineError::InvalidUpdate(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            UpdateMachineError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<DeleteMachineError> for ApiError {
    fn from(error: DeleteMachineError) -> Self {
        match error {
            DeleteMachineError::Denied(denied) => denied.into(),
            DeleteMachineError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            DeleteMachineError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<IssueEnrollmentTokenError> for ApiError {
    fn from(error: IssueEnrollmentTokenError) -> Self {
        match error {
            IssueEnrollmentTokenError::Denied(denied) => denied.into(),
            IssueEnrollmentTokenError::NotFound => {
                Self::detail(StatusCode::NOT_FOUND, error.to_string())
            }
            IssueEnrollmentTokenError::NotEnrollable
            | IssueEnrollmentTokenError::AlreadyEnrolled => {
                Self::detail(StatusCode::CONFLICT, error.to_string())
            }
            IssueEnrollmentTokenError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<UnenrollError> for ApiError {
    fn from(error: UnenrollError) -> Self {
        match error {
            UnenrollError::Denied(denied) => denied.into(),
            UnenrollError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            UnenrollError::NotEnrollable => Self::detail(StatusCode::CONFLICT, error.to_string()),
            UnenrollError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<TestConnectionError> for ApiError {
    fn from(error: TestConnectionError) -> Self {
        match error {
            TestConnectionError::Denied(denied) => denied.into(),
            TestConnectionError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            TestConnectionError::InvalidUpdate(_)
            | TestConnectionError::CapabilityUnavailable(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            TestConnectionError::Transport(error) => error.into(),
            TestConnectionError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, header};
    use paron_tunnel_protocol::AgentId;
    use serde_json::json;
    use serde_utoipa_validate::{ValidationErrors, Wire as _};

    use super::*;
    use crate::application::machine::{
        EnrollmentToken, MockMachineConnections, MockMachineEnrollment, MockMachineManagement,
        OpenTerminalError,
    };
    use crate::domain::machine::CapabilityUnavailable;
    use crate::domain::machine::ssh::PasswordBoundToHostKey;
    use crate::domain::machine::tunnel::token::{ServerEndpoint, Token};
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};
    use crate::testing::{enrollment, keys, machine};

    #[test]
    fn from_error_returns_the_status_of_its_kind() {
        let errors_and_statuses: [(ApiError, StatusCode); 5] = [
            (GetMachineError::NotFound.into(), StatusCode::NOT_FOUND),
            (
                TestConnectionError::Transport(TransportError::HostKeyMismatch).into(),
                StatusCode::BAD_GATEWAY,
            ),
            (
                OpenTerminalError::Transport(TransportError::NotEnrolled).into(),
                StatusCode::BAD_REQUEST,
            ),
            (
                UpdateMachineError::InvalidUpdate(PasswordBoundToHostKey.into()).into(),
                StatusCode::BAD_REQUEST,
            ),
            (
                TestConnectionError::CapabilityUnavailable(CapabilityUnavailable::Missing(
                    ProtocolKind::Ssh,
                ))
                .into(),
                StatusCode::BAD_REQUEST,
            ),
        ];

        for (error, expected_status) in errors_and_statuses {
            assert_eq!(error.status(), expected_status, "{error:?}");
        }
    }

    fn ssh_request_with_port(port: &serde_json::Value) -> SshRequest {
        let host_key = ssh_key::PublicKey::from_bytes(keys::SSH_PUBLIC_KEY)
            .unwrap()
            .to_openssh()
            .unwrap();

        serde_json::from_value(json!({
            "user": "deploy",
            "port": port,
            "authentication": {
                "host_key": host_key,
                "method": { "kind": "password", "password": "hunter2" },
            },
            "connect_timeout_seconds": 30,
        }))
        .expect("an invalid value is an error of `parse`, not of deserialization")
    }

    #[test]
    fn parse_ssh_with_port_outside_the_range_returns_error_for_port() {
        for port in [
            json!(0),
            json!(65_536),
            json!(-1),
            json!(22.5),
            json!(u64::MAX),
        ] {
            let mut errors = ValidationErrors::new();

            let parsed = ssh_request_with_port(&port).parse(&mut errors);

            assert!(parsed.is_none(), "{port}");
            assert_eq!(
                errors.into_fields().into_keys().collect::<Vec<_>>(),
                ["port"],
                "{port}"
            );
        }
    }

    #[test]
    fn parse_ssh_with_highest_port_succeeds() {
        let parsed = ssh_request_with_port(&json!(65_535)).parse(&mut ValidationErrors::new());

        assert!(parsed.is_some());
    }

    #[tokio::test]
    async fn update_with_null_ssh_passes_the_removal_of_ssh() {
        let mut machines = MockMachineManagement::new();
        machines
            .expect_update()
            .once()
            .withf(|actor, cmd| {
                http::is_signed_in_user(actor)
                    && matches!(cmd.protocols.ssh, Some(ProtocolUpdate::Remove))
            })
            .returning(|_, _| Ok(machine::machine_builder().without_ssh().build()));
        let api = router(Deps {
            machines: Arc::new(machines),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::patch(format!("/api/machines/{}", uuid::Uuid::now_v7()))
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "protocols": { "ssh": null } }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn update_without_protocols_passes_no_change_of_ssh() {
        let mut machines = MockMachineManagement::new();
        machines
            .expect_update()
            .once()
            .withf(|actor, cmd| http::is_signed_in_user(actor) && cmd.protocols.ssh.is_none())
            .returning(|_, _| Ok(machine::direct_machine()));
        let api = router(Deps {
            machines: Arc::new(machines),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::patch(format!("/api/machines/{}", uuid::Uuid::now_v7()))
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "name": "web" }).to_string()))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn get_by_id_returns_the_providers_of_each_capability() {
        let mut machines = MockMachineManagement::new();
        machines
            .expect_get()
            .once()
            .returning(|_, _| Ok(machine::direct_machine()));
        let api = router(Deps {
            machines: Arc::new(machines),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get(format!("/api/machines/{}", uuid::Uuid::now_v7()))
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["capabilities"],
            json!({
                "connection_test": [{ "kind": "protocol", "protocol": "ssh" }],
                "terminal": [{ "kind": "protocol", "protocol": "ssh" }],
            })
        );
    }

    #[tokio::test]
    async fn test_connection_with_a_protocol_passes_it_to_the_test() {
        let machine_id = uuid::Uuid::now_v7();
        let mut machine_connections = MockMachineConnections::new();
        machine_connections
            .expect_test()
            .once()
            .withf(move |actor, cmd| {
                http::is_signed_in_user(actor)
                    && matches!(
                        cmd,
                        TestConnectionCommand {
                            protocol: ProtocolKind::Ssh,
                            machine: test_connection::Machine::Stored(id),
                        } if *id == machine_id
                    )
            })
            .returning(|_, _| Ok(()));
        let api = router(Deps {
            machine_connections: Arc::new(machine_connections),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post(format!(
                "/api/machines/{machine_id}/test-connection?protocol=ssh"
            ))
            .header(header::COOKIE, SESSION_COOKIE)
            .body(Body::empty())
            .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    #[tokio::test]
    async fn test_connection_with_an_unknown_protocol_returns_unprocessable_entity() {
        let mut machine_connections = MockMachineConnections::new();
        machine_connections.expect_test().never();
        let api = router(Deps {
            machine_connections: Arc::new(machine_connections),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post(format!(
                "/api/machines/{}/test-connection?protocol=telnet",
                uuid::Uuid::now_v7()
            ))
            .header(header::COOKIE, SESSION_COOKIE)
            .body(Body::empty())
            .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    #[tokio::test]
    async fn issue_enrollment_token_returns_the_issued_token() {
        let machine_id = uuid::Uuid::now_v7();
        let mut machine_enrollment = MockMachineEnrollment::new();
        machine_enrollment
            .expect_issue_enrollment_token()
            .once()
            .withf(move |actor, cmd| http::is_signed_in_user(actor) && cmd.machine_id == machine_id)
            .returning(|_, cmd| {
                Ok(EnrollmentToken {
                    token: Token {
                        server_endpoint: ServerEndpoint {
                            address: enrollment::broker_address(),
                            machine: AgentId::new(cmd.machine_id),
                            fingerprint: enrollment::broker_fingerprint(),
                        },
                        secret: enrollment::enrollment_token(),
                    },
                    expires_at: enrollment::expires_at(),
                })
            });
        let api = router(Deps {
            machine_enrollment: Arc::new(machine_enrollment),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post(format!("/api/machines/{machine_id}/enrollment"))
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let token: Token = body["token"].as_str().unwrap().parse().unwrap();
        assert_eq!(token.server_endpoint.machine, AgentId::new(machine_id));
        assert_eq!(body["expires_at"], enrollment::expires_at().to_string());
    }
}
