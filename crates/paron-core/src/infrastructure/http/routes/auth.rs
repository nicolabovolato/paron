use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_utoipa_validate::{newtype, validate};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use webauthn_rs_proto::{PublicKeyCredential, PublicKeyCredentialRequestOptions};

use super::account::{PasskeyCreationOptionsJson, PasskeyRegistrationJson, TotpCode};
use super::users::{Password, UserResponse, Username};
use crate::application;
use crate::application::identity::authentication::account_recovery::{
    RecoverAccountCommand, RecoveryAction, RecoveryOutcome,
};
use crate::application::identity::authentication::credentials::{
    AcceptedCredential, CredentialToVerify,
};
use crate::application::identity::authentication::sign_in::{
    ContinueSignInCommand, SignInOutcome, SignInStep, StartSignInCommand,
};
use crate::application::identity::errors::{
    ContinueSignInError, RecoverAccountError, RecoveryOptionsError, SignOutError, StartSignInError,
};
use crate::infrastructure::http::auth::{
    self, CurrentUser, DeviceCookie, RecoveryToken, SignInToken,
};
use crate::infrastructure::http::error;
use crate::infrastructure::http::error::{ApiError, ErrorCode};
use crate::infrastructure::http::public_origins::RequestOrigin;
use crate::infrastructure::http::state::Deps;

pub fn routes() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(start_sign_in))
        .routes(routes!(verify_sign_in))
        .routes(routes!(read_session, sign_out))
        .routes(routes!(recovery_options, recover_account))
}

#[utoipa::path(
    post,
    path = "/auth/sign-in",
    tag = "auth",
    params(StartSignInParams),
    responses((status = OK, body = StartedSignInResponse, headers(("Set-Cookie")))),
)]
async fn start_sign_in(
    State(deps): State<Deps>,
    RequestOrigin(origin): RequestOrigin,
    Query(params): Query<StartSignInParams>,
    user: Result<CurrentUser, ApiError>,
    cookies: CookieJar,
) -> error::Result<impl IntoResponse> {
    let cmd = if params.step_up {
        StartSignInCommand::Reauthentication {
            origin,
            user: user?.into(),
        }
    } else {
        StartSignInCommand::NewSession { origin }
    };
    let started = deps.authentication.start_sign_in(cmd).await?;

    Ok((
        cookies.add(auth::sign_in_cookie(
            started.token.expose_secret().to_owned(),
            started.expires_at,
        )),
        Json(StartedSignInResponse {
            accepted: started
                .accepted
                .into_iter()
                .map(AcceptedCredentialResponse::from)
                .collect(),
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/auth/sign-in/verify",
    tag = "auth",
    responses((status = OK, body = SignInResponse, headers(("Set-Cookie")))),
)]
async fn verify_sign_in(
    State(deps): State<Deps>,
    SignInToken(sign_in_token): SignInToken,
    device_cookie: Option<DeviceCookie>,
    cookies: CookieJar,
    Json(body): Json<SignInCredentialRequest>,
) -> error::Result<impl IntoResponse> {
    let (username, credential) = body
        .validated()
        .map_err(|_| -> ApiError {
            ContinueSignInError::InvalidCredentials(anyhow::anyhow!(
                "the format of the credential is not correct"
            ))
            .into()
        })?
        .into_credential();

    let outcome = deps
        .authentication
        .continue_sign_in(ContinueSignInCommand {
            token: sign_in_token,
            device_token: device_cookie.map(|DeviceCookie(device_token)| device_token),
            username,
            credential,
        })
        .await?;

    Ok(match outcome {
        SignInOutcome::SignedIn {
            token,
            expires_at,
            device_token,
        } => (
            auth::remove_sign_in_cookie(cookies)
                .add(auth::session_cookie(
                    token.expose_secret().to_owned(),
                    expires_at,
                ))
                .add(auth::device_cookie(device_token.expose_secret().to_owned())),
            Json(SignInResponse { accepted: None }),
        ),
        SignInOutcome::NextStep(SignInStep {
            token,
            expires_at,
            accepted,
        }) => (
            cookies.add(auth::sign_in_cookie(
                token.expose_secret().to_owned(),
                expires_at,
            )),
            Json(SignInResponse {
                accepted: Some(
                    accepted
                        .into_iter()
                        .map(AcceptedCredentialResponse::from)
                        .collect(),
                ),
            }),
        ),
    })
}

#[utoipa::path(
    get,
    path = "/auth/recovery",
    tag = "auth",
    security(("recovery_token" = [])),
    responses((status = OK, body = RecoveryOptionsResponse)),
)]
async fn recovery_options(
    State(deps): State<Deps>,
    RecoveryToken(token): RecoveryToken,
    RequestOrigin(origin): RequestOrigin,
) -> error::Result<impl IntoResponse> {
    let options = deps.account_recovery.options(token, origin).await?;

    Ok(Json(RecoveryOptionsResponse {
        methods: options.methods.into_iter().map(Into::into).collect(),
        expires_at: options.expires_at,
    }))
}

#[utoipa::path(
    post,
    path = "/auth/recovery",
    tag = "auth",
    security(("recovery_token" = [])),
    responses((status = OK, body = RecoveredAccountResponse)),
)]
async fn recover_account(
    State(deps): State<Deps>,
    RecoveryToken(token): RecoveryToken,
    RequestOrigin(origin): RequestOrigin,
    Json(body): Json<RecoverAccountRequest>,
) -> error::Result<impl IntoResponse> {
    let cmd = RecoverAccountCommand {
        token,
        action: body.validated()?.into_action(origin),
    };

    Ok(Json(RecoveredAccountResponse::from(
        deps.account_recovery.recover(cmd).await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/auth/session",
    tag = "auth",
    security(("session" = [])),
    responses((status = NO_CONTENT, headers(("Set-Cookie")))),
)]
async fn sign_out(
    State(deps): State<Deps>,
    cookies: CookieJar,
    user: CurrentUser,
) -> error::Result<impl IntoResponse> {
    deps.authentication.sign_out(&user.into()).await?;

    Ok((StatusCode::NO_CONTENT, auth::remove_session_cookie(cookies)))
}

#[utoipa::path(
    get,
    path = "/auth/session",
    tag = "auth",
    security(("session" = [])),
    responses((status = OK, body = UserResponse)),
)]
async fn read_session(user: CurrentUser) -> impl IntoResponse {
    Json(UserResponse::from(user))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StartSignInParams {
    #[serde(default)]
    step_up: bool,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignInCredentialRequest {
    Password {
        username: Username,
        password: Password,
    },
    Totp {
        code: TotpCode,
    },
    RecoveryCode {
        code: RecoveryCode,
    },
    Passkey {
        response: PasskeyAssertionJson,
    },
}

impl SignInCredentialRequestParsed {
    fn into_credential(self) -> (Option<crate::domain::user::Username>, CredentialToVerify) {
        match self {
            SignInCredentialRequestParsed::Password { username, password } => {
                (Some(username), CredentialToVerify::Password(password))
            }
            SignInCredentialRequestParsed::Totp { code } => (None, CredentialToVerify::Totp(code)),
            SignInCredentialRequestParsed::RecoveryCode { code } => {
                (None, CredentialToVerify::RecoveryCode(code))
            }
            SignInCredentialRequestParsed::Passkey { response } => {
                (None, CredentialToVerify::Passkey(response.0))
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryOptionsResponse {
    pub methods: Vec<AuthenticationMethod>,
    pub expires_at: jiff::Timestamp,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RecoverAccountRequest {
    SetPassword { password: Password },
    StartPasskeyRegistration,
    FinishPasskeyRegistration { response: PasskeyRegistrationJson },
}

impl RecoverAccountRequestParsed {
    fn into_action(self, origin: url::Origin) -> RecoveryAction {
        match self {
            RecoverAccountRequestParsed::SetPassword { password } => {
                RecoveryAction::SetPassword(password)
            }
            RecoverAccountRequestParsed::StartPasskeyRegistration => {
                RecoveryAction::StartPasskeyRegistration { origin }
            }
            RecoverAccountRequestParsed::FinishPasskeyRegistration { response } => {
                RecoveryAction::FinishPasskeyRegistration {
                    origin,
                    response: response.0,
                }
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum RecoveredAccountResponse {
    PasskeyCreationOptions { options: PasskeyCreationOptionsJson },
    Done,
}

impl From<RecoveryOutcome> for RecoveredAccountResponse {
    fn from(outcome: RecoveryOutcome) -> Self {
        match outcome {
            RecoveryOutcome::PasskeyCreationOptions(options) => Self::PasskeyCreationOptions {
                options: PasskeyCreationOptionsJson(options),
            },
            RecoveryOutcome::Recovered(_) => Self::Done,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StartedSignInResponse {
    pub accepted: Vec<AcceptedCredentialResponse>,
}

newtype! {
    pub AuthenticationMethod: String -> application::identity::authentication::methods::AuthenticationMethod {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(
                application::identity::authentication::methods::AuthenticationMethod::ALL
                    .iter()
                    .map(|method| method.as_str()),
            )),
        deserialize: |raw: String| raw.parse::<application::identity::authentication::methods::AuthenticationMethod>(),
        serialize: |method: &application::identity::authentication::methods::AuthenticationMethod| Ok(method.as_str()),
    }
}

newtype! {
    pub RecoveryCode: String -> application::identity::authentication::methods::recovery_code::RecoveryCode {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .pattern(Some(format!(
                "^[{}]{{{}}}$",
                String::from_utf8_lossy(application::identity::authentication::methods::recovery_code::RecoveryCode::ALPHABET),
                application::identity::authentication::methods::recovery_code::RecoveryCode::LENGTH,
            ))),
        deserialize: |raw: String| application::identity::authentication::methods::recovery_code::RecoveryCode::parse(&raw),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignInResponse {
    pub accepted: Option<Vec<AcceptedCredentialResponse>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptedCredentialResponse {
    Password,
    Totp,
    RecoveryCode,
    Passkey { options: PasskeyRequestOptionsJson },
}

impl From<AcceptedCredential> for AcceptedCredentialResponse {
    fn from(accepted: AcceptedCredential) -> Self {
        match accepted {
            AcceptedCredential::Password => Self::Password,
            AcceptedCredential::Totp => Self::Totp,
            AcceptedCredential::RecoveryCode => Self::RecoveryCode,
            AcceptedCredential::Passkey { options, .. } => Self::Passkey {
                options: PasskeyRequestOptionsJson(options),
            },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(value_type = Object)]
pub struct PasskeyRequestOptionsJson(pub PublicKeyCredentialRequestOptions);

#[derive(Debug, Deserialize, ToSchema)]
#[schema(value_type = Object)]
pub struct PasskeyAssertionJson(pub PublicKeyCredential);

impl serde_utoipa_validate::Wire for PasskeyAssertionJson {
    type Parsed = Self;

    fn parse(self, _: &mut serde_utoipa_validate::ValidationErrors) -> Option<Self> {
        Some(self)
    }
}

impl From<ContinueSignInError> for ApiError {
    fn from(error: ContinueSignInError) -> Self {
        match error {
            ContinueSignInError::InvalidCredentials(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            ContinueSignInError::Suspended => {
                Self::detail(StatusCode::FORBIDDEN, error.to_string())
            }
            ContinueSignInError::SignInEnded => {
                Self::detail(StatusCode::UNAUTHORIZED, error.to_string())
            }
            ContinueSignInError::TooManyAttempts { retry_after } => Self::from_code(
                ErrorCode::TooManyFailedAttempts { retry_after },
                error.to_string(),
            ),
            ContinueSignInError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<StartSignInError> for ApiError {
    fn from(error: StartSignInError) -> Self {
        match error {
            StartSignInError::NoMethodAtOrigin => {
                Self::detail(StatusCode::CONFLICT, error.to_string())
            }
            StartSignInError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<SignOutError> for ApiError {
    fn from(error: SignOutError) -> Self {
        match error {
            SignOutError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<RecoveryOptionsError> for ApiError {
    fn from(error: RecoveryOptionsError) -> Self {
        match error {
            RecoveryOptionsError::InvalidToken(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            RecoveryOptionsError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<RecoverAccountError> for ApiError {
    fn from(error: RecoverAccountError) -> Self {
        match error {
            RecoverAccountError::InvalidToken(_) | RecoverAccountError::InvalidCredentials(_) => {
                Self::detail(StatusCode::BAD_REQUEST, error.to_string())
            }
            RecoverAccountError::Unexpected(cause) => cause.into(),
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
    use tower::ServiceExt as _;

    use super::*;
    use crate::application::identity::authentication::account_recovery::RecoveryOptions;
    use crate::application::identity::authentication::credentials::AcceptedCredentials;
    use crate::application::identity::authorization::actor::Actor;
    use crate::application::identity::errors::RecoverAccountError;
    use crate::application::identity::use_cases::MockAccountRecovery;
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};
    use crate::testing::identity::{self, PASSWORD};

    const SIGN_IN_COOKIE: &str = "__Host-Http-paron_sign_in=a-sign-in-token";

    #[test]
    fn from_error_returns_the_status_of_its_kind() {
        let errors_and_statuses: [(ApiError, StatusCode); 2] = [
            (
                ContinueSignInError::InvalidCredentials(anyhow::anyhow!(
                    "the format of the credential is not correct"
                ))
                .into(),
                StatusCode::BAD_REQUEST,
            ),
            (ContinueSignInError::Suspended.into(), StatusCode::FORBIDDEN),
        ];

        for (error, expected_status) in errors_and_statuses {
            assert_eq!(error.status(), expected_status, "{error:?}");
        }
    }

    fn set_cookies(response: &axum::response::Response) -> Vec<String> {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|cookie| cookie.to_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn start_sign_in_with_a_host_header_gives_the_origin_of_the_request() {
        let mut authentication = http::authentication();
        authentication
            .expect_start_sign_in()
            .once()
            .withf(|cmd| {
                matches!(
                    cmd,
                    StartSignInCommand::NewSession { origin }
                        if origin.ascii_serialization() == "https://paron.example:8443"
                )
            })
            .returning(|_| {
                Ok(SignInStep {
                    token: SecretString::from("a-sign-in-token"),
                    expires_at: jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(10),
                    accepted: AcceptedCredentials::new(vec![AcceptedCredential::Password]).unwrap(),
                })
            });
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let response = api
            .oneshot(
                Request::post("/api/auth/sign-in")
                    .header(header::HOST, "Paron.Example:8443")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            set_cookies(&response)
                .iter()
                .any(|cookie| cookie.starts_with(SIGN_IN_COOKIE))
        );
    }

    #[tokio::test]
    async fn start_sign_in_with_step_up_starts_it_for_the_user_of_the_cookie() {
        let mut authentication = http::authentication();
        authentication
            .expect_start_sign_in()
            .once()
            .withf(|cmd| {
                matches!(
                    cmd,
                    StartSignInCommand::Reauthentication { user, .. }
                        if http::is_signed_in_user(&Actor::User(user.clone()))
                )
            })
            .returning(|_| {
                Ok(SignInStep {
                    token: SecretString::from("a-sign-in-token"),
                    expires_at: jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(10),
                    accepted: AcceptedCredentials::new(vec![AcceptedCredential::Password]).unwrap(),
                })
            });
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/auth/sign-in?step_up=true")
                .header(header::HOST, "paron.example:8443")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn start_sign_in_with_step_up_without_a_session_returns_unauthorized() {
        let mut authentication = http::authentication();
        authentication.expect_start_sign_in().never();
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/auth/sign-in?step_up=true")
                .header(header::HOST, "paron.example:8443")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }

    #[tokio::test]
    async fn sign_in_after_too_many_wrong_attempts_returns_too_many_requests_with_retry_after_and_its_code()
     {
        let mut authentication = http::authentication();
        authentication
            .expect_continue_sign_in()
            .once()
            .returning(|_| {
                Err(ContinueSignInError::TooManyAttempts {
                    retry_after: std::time::Duration::from_millis(1500),
                })
            });
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let response = api
            .oneshot(
                Request::post("/api/auth/sign-in/verify")
                    .header(header::HOST, http::HOST)
                    .header(header::COOKIE, SIGN_IN_COOKIE)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "kind": "password",
                            "username": identity::USERNAME,
                            "password": PASSWORD,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["code"], "too_many_failed_attempts", "{body}");
    }

    #[tokio::test]
    async fn sign_in_with_right_password_sets_the_session_and_the_device_and_clears_the_sign_in_cookie()
     {
        let mut authentication = http::authentication();
        authentication
            .expect_continue_sign_in()
            .once()
            .withf(|cmd| {
                cmd.token.expose_secret() == "a-sign-in-token"
                    && cmd
                        .device_token
                        .as_ref()
                        .is_some_and(|token| token.expose_secret() == "an-old-device-token")
                    && cmd
                        .username
                        .as_ref()
                        .is_some_and(|username| **username == *identity::USERNAME)
                    && matches!(
                        &cmd.credential,
                        CredentialToVerify::Password(password) if password.expose_secret() == PASSWORD
                    )
            })
            .returning(|_| {
                Ok(SignInOutcome::SignedIn {
                    token: SecretString::from("a-token"),
                    expires_at: jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_hours(24),
                    device_token: SecretString::from("a-new-device-token"),
                })
            });
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let response = api
            .oneshot(
                Request::post("/api/auth/sign-in/verify")
                    .header(header::HOST, http::HOST)
                    .header(
                        header::COOKIE,
                        format!("{SIGN_IN_COOKIE}; __Host-Http-paron_device=an-old-device-token"),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "kind": "password",
                            "username": identity::USERNAME,
                            "password": PASSWORD,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cookies = set_cookies(&response);
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.starts_with(SESSION_COOKIE))
        );
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.starts_with("__Host-Http-paron_sign_in=;"))
        );
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.starts_with("__Host-Http-paron_device=a-new-device-token;"))
        );
    }

    #[tokio::test]
    async fn sign_in_with_a_second_factor_sets_the_next_sign_in_cookie() {
        let mut authentication = http::authentication();
        authentication
            .expect_continue_sign_in()
            .once()
            .returning(|_| {
                Ok(SignInOutcome::NextStep(SignInStep {
                    token: SecretString::from("the-next-sign-in-token"),
                    expires_at: jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(10),
                    accepted: AcceptedCredentials::new(vec![AcceptedCredential::Totp]).unwrap(),
                }))
            });
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let response = api
            .oneshot(
                Request::post("/api/auth/sign-in/verify")
                    .header(header::HOST, http::HOST)
                    .header(header::COOKIE, SIGN_IN_COOKIE)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "kind": "password",
                            "username": identity::USERNAME,
                            "password": PASSWORD,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            set_cookies(&response)
                .iter()
                .any(|cookie| cookie
                    .starts_with("__Host-Http-paron_sign_in=the-next-sign-in-token;"))
        );
    }

    #[tokio::test]
    async fn sign_in_without_the_sign_in_cookie_returns_unauthorized() {
        let (status, _) = http::send(
            &router(http::deps()),
            Request::post("/api/auth/sign-in/verify")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "kind": "totp", "code": "123456" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sign_in_with_an_invalid_username_returns_bad_request() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::post("/api/auth/sign-in/verify")
                .header(header::COOKIE, SIGN_IN_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "kind": "password", "username": "", "password": PASSWORD }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["validation_errors"].is_null(), "{body}");
    }

    #[tokio::test]
    async fn recovery_options_with_a_lower_case_scheme_reads_the_token() {
        let mut account_recovery = MockAccountRecovery::new();
        account_recovery
            .expect_options()
            .once()
            .withf(|token, _| token.expose_secret() == "a-token")
            .returning(|_, _| {
                Ok(RecoveryOptions {
                    methods: Vec::new(),
                    expires_at: jiff::Timestamp::UNIX_EPOCH,
                })
            });
        let api = router(Deps {
            account_recovery: Arc::new(account_recovery),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get("/api/auth/recovery")
                .header(header::HOST, "paron.example:8443")
                .header(header::AUTHORIZATION, "bearer a-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn recover_account_with_an_invalid_token_returns_bad_request() {
        let mut account_recovery = MockAccountRecovery::new();
        account_recovery.expect_recover().once().returning(|_| {
            Err(RecoverAccountError::InvalidToken(anyhow::anyhow!(
                "the account recovery expired"
            )))
        });
        let api = router(Deps {
            account_recovery: Arc::new(account_recovery),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post("/api/auth/recovery")
                .header(header::HOST, "paron.example:8443")
                .header(header::AUTHORIZATION, "Bearer a-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "set_password", "password": "correct horse battery" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn recover_account_with_a_short_password_returns_its_validation_error() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::post("/api/auth/recovery")
                .header(header::HOST, "paron.example:8443")
                .header(header::AUTHORIZATION, "Bearer a-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "action": "set_password", "password": "short" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body["validation_errors"]["password"].is_array(), "{body}");
    }

    #[tokio::test]
    async fn sign_out_with_a_session_cookie_ends_its_session() {
        let mut authentication = http::authentication();
        authentication
            .expect_sign_out()
            .once()
            .withf(http::is_signed_in_user)
            .returning(|_| Ok(()));
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let (status, _) = http::send(
            &api,
            Request::delete("/api/auth/session")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn read_session_returns_the_user_of_the_session() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::get("/api/auth/session")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], http::signed_in_user().id.to_string());
    }
}
