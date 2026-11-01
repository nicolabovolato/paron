use secrecy::{ExposeSecret as _, SecretString};
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::schema::{ObjectBuilder, Type};

use std::collections::BTreeSet;

use super::account::CredentialsResponse;
use crate::application::identity::authorization::actor::AuthenticatedUser;
use crate::application::identity::errors::{
    CreateUserError, DeleteUserError, GetUserError, ListUsersError, StartRecoveryError,
    UpdateUserError,
};
use crate::application::identity::users::{CreateUserCommand, UpdateUserCommand};
use crate::domain::user::User;
use crate::{application, domain};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
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
        .routes(routes!(user_credentials))
        .routes(routes!(start_recovery))
}

#[utoipa::path(
    get,
    path = "/users",
    tag = "users",
    operation_id = "list_users",
    params(ListParams),
    responses((status = OK, body = PageResponse<UserResponse>)),
    security(("session" = [])),
)]
async fn list(
    State(deps): State<Deps>,
    user: CurrentUser,
    Query(query): Query<ListParams>,
) -> error::Result<impl IntoResponse> {
    let query = query.validated()?;

    let users = deps.users.list(&user.into(), &query.into()).await?;

    Ok(Json(PageResponse::<UserResponse>::from(users)))
}

#[utoipa::path(
    get,
    path = "/users/{id}",
    tag = "users",
    operation_id = "read_user",
    responses((status = OK, body = UserResponse)),
    security(("session" = [])),
)]
async fn get_by_id(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    let user = deps.users.get_by_id(&user.into(), id).await?;

    Ok(Json(UserResponse::from(user)))
}

#[utoipa::path(
    post,
    path = "/users",
    tag = "users",
    operation_id = "create_user",
    responses((status = CREATED, body = UserResponse)),
    security(("session" = [])),
)]
async fn create(
    State(deps): State<Deps>,
    user: CurrentUser,
    Json(body): Json<CreateUserRequest>,
) -> error::Result<impl IntoResponse> {
    let body = body.validated()?;

    let created = deps
        .users
        .create(
            &user.into(),
            CreateUserCommand {
                username: body.username,
                display_name: body.display_name,
                roles: body.roles.into_iter().collect::<BTreeSet<_>>(),
            },
        )
        .await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(created))))
}

#[utoipa::path(
    patch,
    path = "/users/{id}",
    tag = "users",
    operation_id = "update_user",
    responses((status = OK, body = UserResponse)),
    security(("session" = [])),
)]
async fn update(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<PartialUpdateUserRequest>,
) -> error::Result<impl IntoResponse> {
    let body = body.validated()?;

    let updated = deps
        .users
        .update(
            &user.into(),
            UpdateUserCommand {
                id,
                username: body.username,
                display_name: body.display_name,
                roles: body.roles.map(|roles| roles.into_iter().collect()),
                status: body.status,
            },
        )
        .await?;

    Ok(Json(UserResponse::from(updated)))
}

#[utoipa::path(
    delete,
    path = "/users/{id}",
    tag = "users",
    operation_id = "remove_user",
    responses((status = NO_CONTENT)),
    security(("session" = [])),
)]
async fn delete(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    deps.users.delete(&user.into(), id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/users/{id}/credentials",
    tag = "users",
    operation_id = "user_credentials",
    responses((status = OK, body = CredentialsResponse)),
    security(("session" = [])),
)]
async fn user_credentials(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    let credentials = deps.credentials.list(&user.into(), id).await?;

    Ok(Json(CredentialsResponse::from(credentials)))
}

#[utoipa::path(
    post,
    path = "/users/{id}/recovery",
    tag = "users",
    operation_id = "start_recovery",
    responses((status = OK, body = RecoveryLinkResponse)),
    security(("session" = [])),
)]
async fn start_recovery(
    State(deps): State<Deps>,
    user: CurrentUser,
    Path(id): Path<uuid::Uuid>,
) -> error::Result<impl IntoResponse> {
    let link = deps
        .account_recovery
        .start_recovery(&user.into(), id)
        .await?;

    Ok(Json(RecoveryLinkResponse {
        token: link.token.expose_secret().to_owned(),
        public_origin: deps.public_origins.canonical().ascii_serialization(),
        expires_at: link.expires_at,
    }))
}

newtype! {
    pub Permission: String -> domain::user::Permission {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(
                domain::user::Permission::ALL
                    .iter()
                    .map(|permission| permission.as_str()),
            )),
        serialize: |permission: &domain::user::Permission| Ok(permission.as_str()),
    }
}

newtype! {
    pub Role: String -> domain::user::Role {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(
                domain::user::Role::ALL.iter().map(|role| role.as_str()),
            )),
        deserialize: |raw: String| raw.parse::<domain::user::Role>(),
        serialize: |role: &domain::user::Role| Ok(role.as_str()),
    }
}

newtype! {
    pub UserStatus: String -> domain::user::UserStatus {
        Active => "active",
        Suspended => "suspended",
    }
}

newtype! {
    pub Username: String -> domain::user::Username {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(
                "Letters, digits, and single '.', '-' or '_' between them. Stored in lower case.",
            ))
            .min_length(Some(domain::user::Username::MIN))
            .max_length(Some(domain::user::Username::MAX))
            .pattern(Some(r"^[A-Za-z0-9]([A-Za-z0-9]|[._-][A-Za-z0-9])*$"))
            .extensions(Some(
                ExtensionsBuilder::new()
                    .add("x-pattern-message", "Letters, digits, and single '.', '-' or '_' between them. Stored in lower case.")
                    .build(),
            )),
        deserialize: |raw: String| domain::user::Username::parse(&raw),
        serialize: |username: &domain::user::Username| Ok(username.to_string()),
    }
}

newtype! {
    pub DisplayName: String -> domain::user::DisplayName {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(domain::user::DisplayName::MIN))
            .max_length(Some(domain::user::DisplayName::MAX)),
        deserialize: |raw: String| domain::user::DisplayName::parse(raw),
        serialize: |name: &domain::user::DisplayName| Ok(name.to_string()),
    }
}

newtype! {
    pub Password: String -> application::identity::authentication::methods::password::Password {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(application::identity::authentication::methods::password::Password::MIN))
            .max_length(Some(application::identity::authentication::methods::password::Password::MAX)),
        deserialize: |raw: String| {
            application::identity::authentication::methods::password::Password::parse(SecretString::from(raw))
        },
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username: Username,
    pub display_name: DisplayName,
    pub roles: Vec<Role>,
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartialUpdateUserRequest {
    pub username: Patch<Username>,
    pub display_name: Patch<DisplayName>,
    pub roles: Patch<Vec<Role>>,
    pub status: Patch<UserStatus>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryLinkResponse {
    pub token: String,
    pub public_origin: String,
    pub expires_at: jiff::Timestamp,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: uuid::Uuid,
    pub username: Username,
    pub display_name: DisplayName,
    pub status: UserStatus,
    pub roles: Vec<Role>,
    pub permissions: Vec<Permission>,
    pub created_at: jiff::Timestamp,
}

impl From<CurrentUser> for UserResponse {
    fn from(user: CurrentUser) -> Self {
        Self::from(AuthenticatedUser::from(user).user)
    }
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        let permissions = user
            .permissions()
            .into_iter()
            .map(Permission::from)
            .collect();

        Self {
            id: user.id,
            username: user.username.into(),
            display_name: user.display_name.into(),
            status: user.status.into(),
            roles: user.roles.into_iter().map(Role::from).collect(),
            permissions,
            created_at: user.created_at,
        }
    }
}

impl From<ListUsersError> for ApiError {
    fn from(error: ListUsersError) -> Self {
        match error {
            ListUsersError::Denied(denied) => denied.into(),
            ListUsersError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<GetUserError> for ApiError {
    fn from(error: GetUserError) -> Self {
        match error {
            GetUserError::Denied(denied) => denied.into(),
            GetUserError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            GetUserError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<CreateUserError> for ApiError {
    fn from(error: CreateUserError) -> Self {
        match error {
            CreateUserError::Denied(denied) => denied.into(),
            CreateUserError::UsernameTaken => Self::detail(StatusCode::CONFLICT, error.to_string()),
            CreateUserError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<UpdateUserError> for ApiError {
    fn from(error: UpdateUserError) -> Self {
        match error {
            UpdateUserError::Denied(denied) => denied.into(),
            UpdateUserError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            UpdateUserError::UsernameTaken => Self::detail(StatusCode::CONFLICT, error.to_string()),
            UpdateUserError::LastAdministrator => {
                Self::detail(StatusCode::CONFLICT, error.to_string())
            }
            UpdateUserError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<DeleteUserError> for ApiError {
    fn from(error: DeleteUserError) -> Self {
        match error {
            DeleteUserError::Denied(denied) => denied.into(),
            DeleteUserError::NotFound => Self::detail(StatusCode::NOT_FOUND, error.to_string()),
            DeleteUserError::LastAdministrator => {
                Self::detail(StatusCode::CONFLICT, error.to_string())
            }
            DeleteUserError::Unexpected(cause) => cause.into(),
        }
    }
}

impl From<StartRecoveryError> for ApiError {
    fn from(error: StartRecoveryError) -> Self {
        match error {
            StartRecoveryError::Denied(denied) => denied.into(),
            StartRecoveryError::UserNotFound => {
                Self::detail(StatusCode::NOT_FOUND, error.to_string())
            }
            StartRecoveryError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, header};

    use super::*;
    use crate::application::identity::authentication::account_recovery::AccountRecoveryToken;
    use crate::application::identity::use_cases::MockAccountRecovery;
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};

    #[tokio::test]
    async fn start_recovery_under_another_origin_returns_the_canonical_public_origin() {
        let mut account_recovery = MockAccountRecovery::new();
        account_recovery
            .expect_start_recovery()
            .once()
            .returning(|_, _| {
                Ok(AccountRecoveryToken {
                    token: SecretString::from("a-recovery-token"),
                    expires_at: jiff::Timestamp::UNIX_EPOCH,
                })
            });
        let api = router(Deps {
            account_recovery: Arc::new(account_recovery),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::post(format!("/api/users/{}/recovery", uuid::Uuid::now_v7()))
                .header(header::HOST, "paron.local")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["token"], "a-recovery-token");
        assert_eq!(body["public_origin"], "https://paron.example:8443");
    }

    #[test]
    fn from_error_returns_the_status_of_its_kind() {
        let errors_and_statuses: [(ApiError, StatusCode); 2] = [
            (CreateUserError::UsernameTaken.into(), StatusCode::CONFLICT),
            (
                UpdateUserError::LastAdministrator.into(),
                StatusCode::CONFLICT,
            ),
        ];

        for (error, expected_status) in errors_and_statuses {
            assert_eq!(error.status(), expected_status, "{error:?}");
        }
    }
}
