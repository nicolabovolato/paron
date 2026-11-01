use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_utoipa_validate::{newtype, validate};
use utoipa::ToSchema;
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use webauthn_rs_proto::{PublicKeyCredentialCreationOptions, RegisterPublicKeyCredential};

use super::users::{DisplayName, Password, Username};
use crate::application;
use crate::application::identity::authentication::credentials::{
    AccountCredentials, CredentialToAdd, CredentialToRemove, CredentialUpdateResult,
    UpdateCredentialsCommand,
};
use crate::application::identity::authorization::actor::AuthenticatedUser;
use crate::application::identity::errors::{ListCredentialsError, UpdateCredentialsError};
use crate::infrastructure::http::auth::CurrentUser;
use crate::infrastructure::http::error;
use crate::infrastructure::http::error::ApiError;
use crate::infrastructure::http::public_origins::RequestOrigin;
use crate::infrastructure::http::state::Deps;

pub fn routes() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(read_account))
        .routes(routes!(credentials, update_credentials))
}

#[utoipa::path(
    get,
    path = "/account",
    tag = "account",
    security(("session" = [])),
    responses((status = OK, body = AccountResponse)),
)]
async fn read_account(user: CurrentUser) -> impl IntoResponse {
    let user = AuthenticatedUser::from(user).user;

    Json(AccountResponse {
        id: user.id,
        username: user.username.into(),
        display_name: user.display_name.into(),
        created_at: user.created_at,
    })
}

#[utoipa::path(
    get,
    path = "/account/credentials",
    tag = "account",
    operation_id = "own_credentials",
    security(("session" = [])),
    responses((status = OK, body = CredentialsResponse)),
)]
async fn credentials(
    State(deps): State<Deps>,
    user: CurrentUser,
) -> error::Result<impl IntoResponse> {
    let user_id = user.user.id;
    let credentials = deps.credentials.list(&user.into(), user_id).await?;

    Ok(Json(CredentialsResponse::from(credentials)))
}

#[utoipa::path(
    post,
    path = "/account/credentials",
    tag = "account",
    security(("session" = [])),
    responses((status = OK, body = UpdateCredentialsResponse)),
)]
async fn update_credentials(
    State(deps): State<Deps>,
    user: CurrentUser,
    RequestOrigin(origin): RequestOrigin,
    Json(body): Json<UpdateCredentialsRequest>,
) -> error::Result<Json<UpdateCredentialsResponse>> {
    let cmd = body.validated()?.into_command(origin);
    let user_id = user.user.id;
    let updated = deps.credentials.update(&user.into(), user_id, cmd).await?;

    Ok(Json(UpdateCredentialsResponse {
        result: updated.result.into(),
        recovery_codes: updated.recovery_codes.map(|codes| {
            codes
                .iter()
                .map(|code| code.expose_secret().to_owned())
                .collect()
        }),
    }))
}

newtype! {
    pub PasskeyName: String -> application::identity::authentication::methods::passkey::PasskeyName {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(application::identity::authentication::methods::passkey::PasskeyName::MIN))
            .max_length(Some(application::identity::authentication::methods::passkey::PasskeyName::MAX)),
        deserialize: |raw: String| application::identity::authentication::methods::passkey::PasskeyName::parse(raw),
        serialize: |name: &application::identity::authentication::methods::passkey::PasskeyName| Ok(name.to_string()),
    }
}

newtype! {
    pub TotpCode: String -> application::identity::authentication::methods::totp::TotpCode {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .pattern(Some(r"^\d{6}$"))
            .extensions(Some(ExtensionsBuilder::new().add("x-pattern-message", "Six digits.").build())),
        deserialize: |raw: String| application::identity::authentication::methods::totp::TotpCode::parse(&raw),
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UpdateCredentialsRequest {
    SetPassword { new_password: Password },
    RemovePassword,
    StartTotpRegistration,
    FinishTotpRegistration { code: TotpCode },
    RemoveTotp,
    StartPasskeyRegistration,
    FinishPasskeyRegistration { response: PasskeyRegistrationJson },
    RenamePasskey { id: uuid::Uuid, name: PasskeyName },
    RemovePasskey { id: uuid::Uuid },
    GenerateRecoveryCodes,
}

impl UpdateCredentialsRequestParsed {
    fn into_command(self, origin: url::Origin) -> UpdateCredentialsCommand {
        match self {
            UpdateCredentialsRequestParsed::SetPassword { new_password } => {
                UpdateCredentialsCommand::Add(CredentialToAdd::Password(new_password))
            }
            UpdateCredentialsRequestParsed::RemovePassword => {
                UpdateCredentialsCommand::Remove(CredentialToRemove::Password)
            }
            UpdateCredentialsRequestParsed::StartTotpRegistration => {
                UpdateCredentialsCommand::StartTotpRegistration
            }
            UpdateCredentialsRequestParsed::FinishTotpRegistration { code } => {
                UpdateCredentialsCommand::Add(CredentialToAdd::Totp(code))
            }
            UpdateCredentialsRequestParsed::RemoveTotp => {
                UpdateCredentialsCommand::Remove(CredentialToRemove::Totp)
            }
            UpdateCredentialsRequestParsed::StartPasskeyRegistration => {
                UpdateCredentialsCommand::StartPasskeyRegistration { origin }
            }
            UpdateCredentialsRequestParsed::FinishPasskeyRegistration { response } => {
                UpdateCredentialsCommand::Add(CredentialToAdd::Passkey {
                    origin,
                    response: response.0,
                })
            }
            UpdateCredentialsRequestParsed::RenamePasskey { id, name } => {
                UpdateCredentialsCommand::RenamePasskey { id, name }
            }
            UpdateCredentialsRequestParsed::RemovePasskey { id } => {
                UpdateCredentialsCommand::Remove(CredentialToRemove::Passkey(id))
            }
            UpdateCredentialsRequestParsed::GenerateRecoveryCodes => {
                UpdateCredentialsCommand::GenerateRecoveryCodes
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateCredentialsResponse {
    pub result: CredentialUpdateResultResponse,
    pub recovery_codes: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum CredentialUpdateResultResponse {
    Done,
    TotpRegistrationStarted { secret: String, key_uri: String },
    PasskeyCreationOptions { options: PasskeyCreationOptionsJson },
}

impl From<CredentialUpdateResult> for CredentialUpdateResultResponse {
    fn from(result: CredentialUpdateResult) -> Self {
        match result {
            CredentialUpdateResult::Done => Self::Done,
            CredentialUpdateResult::TotpRegistrationStarted(setup) => {
                Self::TotpRegistrationStarted {
                    secret: setup.secret.expose_secret().to_owned(),
                    key_uri: setup.key_uri.expose_secret().to_owned(),
                }
            }
            CredentialUpdateResult::PasskeyCreationOptions(options) => {
                Self::PasskeyCreationOptions {
                    options: PasskeyCreationOptionsJson(options),
                }
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(value_type = Object)]
pub struct PasskeyCreationOptionsJson(pub PublicKeyCredentialCreationOptions);

#[derive(Debug, Deserialize, ToSchema)]
#[schema(value_type = Object)]
pub struct PasskeyRegistrationJson(pub RegisterPublicKeyCredential);

impl serde_utoipa_validate::Wire for PasskeyRegistrationJson {
    type Parsed = Self;

    fn parse(self, _: &mut serde_utoipa_validate::ValidationErrors) -> Option<Self> {
        Some(self)
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccountResponse {
    pub id: uuid::Uuid,
    pub username: Username,
    pub display_name: DisplayName,
    pub created_at: jiff::Timestamp,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialsResponse {
    pub password: Option<PasswordCredentialResponse>,
    pub totp: Option<TotpCredentialResponse>,
    pub passkeys: Vec<PasskeyCredentialResponse>,
    pub recovery_codes: Option<RecoveryCodesResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PasswordCredentialResponse {
    pub updated_at: jiff::Timestamp,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpCredentialResponse {
    pub created_at: jiff::Timestamp,
    pub last_used_at: jiff::Timestamp,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PasskeyCredentialResponse {
    pub id: uuid::Uuid,
    pub name: PasskeyName,
    pub created_at: jiff::Timestamp,
    pub last_used_at: Option<jiff::Timestamp>,
    pub backed_up: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    pub generated_at: jiff::Timestamp,
    pub remaining: usize,
}

impl From<AccountCredentials> for CredentialsResponse {
    fn from(account: AccountCredentials) -> Self {
        Self {
            password: account.password.map(|password| PasswordCredentialResponse {
                updated_at: password.updated_at,
            }),
            totp: account.totp.map(|totp| TotpCredentialResponse {
                created_at: totp.created_at,
                last_used_at: totp.last_used_at,
            }),
            passkeys: account
                .passkeys
                .into_iter()
                .map(|passkey| PasskeyCredentialResponse {
                    id: passkey.id,
                    name: passkey.name.into(),
                    created_at: passkey.created_at,
                    last_used_at: passkey.last_used_at,
                    backed_up: passkey.backed_up,
                })
                .collect(),
            recovery_codes: account.recovery_codes.map(|codes| RecoveryCodesResponse {
                generated_at: codes.generated_at,
                remaining: codes.remaining,
            }),
        }
    }
}

impl From<ListCredentialsError> for ApiError {
    fn from(error: ListCredentialsError) -> Self {
        match error {
            ListCredentialsError::Denied(denied) => denied.into(),
            ListCredentialsError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<UpdateCredentialsError> for ApiError {
    fn from(error: UpdateCredentialsError) -> Self {
        match error {
            UpdateCredentialsError::Denied(denied) => denied.into(),
            UpdateCredentialsError::InvalidCredentials(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            UpdateCredentialsError::AlreadyExists(_)
            | UpdateCredentialsError::LastSignInMethod
            | UpdateCredentialsError::LimitReached(_) => {
                Self::detail(StatusCode::CONFLICT, error.to_string())
            }
            UpdateCredentialsError::CredentialNotFound | UpdateCredentialsError::UserNotFound => {
                Self::detail(StatusCode::NOT_FOUND, error.to_string())
            }
            UpdateCredentialsError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use secrecy::SecretString;
    use serde_json::json;

    use super::*;
    use crate::application::identity::authentication::credentials::UpdatedCredentials;
    use crate::application::identity::authorization::actor::Denied;
    use crate::application::identity::errors::UpdateCredentialsError;
    use crate::application::identity::use_cases::MockCredentialManagement;
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};

    #[tokio::test]
    async fn update_credentials_with_a_wrong_code_returns_bad_request() {
        let mut credentials = MockCredentialManagement::new();
        credentials.expect_update().once().returning(|_, _, _| {
            Err(UpdateCredentialsError::InvalidCredentials(anyhow::anyhow!(
                "the code is not correct"
            )))
        });
        let api = router(Deps {
            credentials: Arc::new(credentials),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/account/credentials")
                .header(header::HOST, "paron.example:8443")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "finish_totp_registration", "code": "000000" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["validation_errors"].is_null(), "{body}");
    }

    #[tokio::test]
    async fn update_credentials_without_a_recent_sign_in_returns_the_reauthentication_challenge() {
        let mut credentials = MockCredentialManagement::new();
        credentials.expect_update().once().returning(|_, _, _| {
            Err(UpdateCredentialsError::Denied(
                Denied::RecentAuthenticationRequired,
            ))
        });
        let api = router(Deps {
            credentials: Arc::new(credentials),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/account/credentials")
                .header(header::HOST, "paron.example:8443")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "start_totp_registration" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "insufficient_user_authentication", "{body}");
    }

    #[tokio::test]
    async fn update_credentials_finish_totp_registration_with_a_code_returns_the_recovery_codes() {
        let mut credentials = MockCredentialManagement::new();
        credentials
            .expect_update()
            .once()
            .withf(|actor, user_id, cmd| {
                http::is_signed_in_user(actor)
                    && *user_id == http::signed_in_user().id
                    && matches!(
                        cmd,
                        UpdateCredentialsCommand::Add(CredentialToAdd::Totp(code)) if &**code == "123456"
                    )
            })
            .returning(|_, _, _| {
                Ok(UpdatedCredentials {
                    result: CredentialUpdateResult::Done,
                    recovery_codes: Some(vec![SecretString::from("abcde-fghjk")]),
                })
            });
        let api = router(Deps {
            credentials: Arc::new(credentials),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/account/credentials")
                .header(header::HOST, "paron.example:8443")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "finish_totp_registration", "code": "123456" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({ "result": { "kind": "done" }, "recovery_codes": ["abcde-fghjk"] })
        );
    }

    #[tokio::test]
    async fn update_credentials_finish_totp_registration_with_letters_returns_a_validation_error() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::post("/api/account/credentials")
                .header(header::HOST, "paron.example:8443")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "finish_totp_registration", "code": "12345a" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(!body["validation_errors"]["code"].is_null(), "{body}");
    }
}
