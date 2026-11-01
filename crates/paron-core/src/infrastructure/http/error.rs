use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use strum::EnumDiscriminants;
use utoipa::ToSchema;

use serde_utoipa_validate::ValidationErrors;

use crate::application::identity::authorization::actor::Denied;

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, EnumDiscriminants)]
#[serde(into = "ErrorCodeName")]
#[strum_discriminants(
    name(ErrorCodeName),
    derive(Serialize, ToSchema),
    serde(rename_all = "snake_case"),
    schema(as = ErrorCode)
)]
pub enum ErrorCode {
    InsufficientUserAuthentication,
    TooManyFailedAttempts { retry_after: Duration },
}

impl ErrorCode {
    fn status(self) -> StatusCode {
        match self {
            Self::InsufficientUserAuthentication => StatusCode::FORBIDDEN,
            Self::TooManyFailedAttempts { .. } => StatusCode::TOO_MANY_REQUESTS,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    #[serde(with = "http_serde::status_code")]
    #[schema(value_type = u16)]
    status: StatusCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<ErrorCodeName>)]
    code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation_errors: Option<BTreeMap<String, Vec<String>>>,
}

impl ApiError {
    fn new(status: StatusCode) -> Self {
        Self {
            status,
            code: None,
            detail: None,
            validation_errors: None,
        }
    }

    pub fn detail(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::new(status)
        }
    }

    pub fn from_code(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            ..Self::detail(code.status(), detail)
        }
    }

    pub fn internal(cause: impl std::fmt::Display) -> Self {
        let cause = format!("{cause:#}");

        tracing::error!(%cause, "internal server error");

        Self::new(StatusCode::INTERNAL_SERVER_ERROR)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry_after = match self.code {
            Some(ErrorCode::TooManyFailedAttempts { retry_after }) => Some([(
                axum::http::header::RETRY_AFTER,
                retry_after.as_secs_f64().ceil().to_string(),
            )]),
            _ => None,
        };

        (self.status, retry_after, axum::Json(self)).into_response()
    }
}

impl<'r> utoipa::ToResponse<'r> for ApiError {
    fn response() -> (
        &'r str,
        utoipa::openapi::RefOr<utoipa::openapi::response::Response>,
    ) {
        static NAME: std::sync::LazyLock<std::borrow::Cow<'static, str>> =
            std::sync::LazyLock::new(ApiError::name);

        let response = utoipa::openapi::response::ResponseBuilder::new()
            .content(
                mime::APPLICATION_JSON.as_ref(),
                utoipa::openapi::content::ContentBuilder::new()
                    .schema(Some(utoipa::openapi::Ref::from_schema_name(Self::name())))
                    .build(),
            )
            .build();

        (&NAME, response.into())
    }
}

pub(super) fn declare_error_responses(openapi: &mut utoipa::openapi::OpenApi) {
    let (name, _) = <ApiError as utoipa::ToResponse>::response();

    for item in openapi.paths.paths.values_mut() {
        let operations = [
            &mut item.get,
            &mut item.put,
            &mut item.post,
            &mut item.delete,
            &mut item.options,
            &mut item.head,
            &mut item.patch,
            &mut item.trace,
        ];

        for operation in operations.into_iter().flatten() {
            for range in ["4XX", "5XX"] {
                operation
                    .responses
                    .responses
                    .entry(range.to_owned())
                    .or_insert_with(|| utoipa::openapi::Ref::from_response_name(name).into());
            }
        }
    }
}

/// Transform any 4xx or 5xx response (with content type missing or text/plain) into an ApiError
pub async fn normalise_error_response(response: Response) -> Response {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }

    let content_type = response.headers().get(axum::http::header::CONTENT_TYPE);
    let is_text = content_type
        .and_then(|kind| kind.to_str().ok())
        .and_then(|kind| kind.parse::<mime::Mime>().ok())
        .is_some_and(|kind| kind.essence_str() == mime::TEXT_PLAIN.essence_str());

    if content_type.is_some() && !is_text {
        return response;
    }

    let (mut parts, body) = response.into_parts();

    let detail = if is_text && status.is_client_error() {
        axum::body::to_bytes(body, 512)
            .await
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .filter(|text| !text.is_empty())
    } else {
        None
    };

    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
    );
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);

    Response::from_parts(
        parts,
        axum::body::Body::from(
            serde_json::to_vec(&ApiError {
                detail,
                ..ApiError::new(status)
            })
            .unwrap_or_default(),
        ),
    )
}

impl From<Denied> for ApiError {
    fn from(denied: Denied) -> Self {
        match denied {
            Denied::MissingPermission(_) | Denied::NotAllowed => {
                Self::detail(StatusCode::FORBIDDEN, denied.to_string())
            }
            Denied::RecentAuthenticationRequired => Self::from_code(
                ErrorCode::InsufficientUserAuthentication,
                denied.to_string(),
            ),
        }
    }
}

impl From<ValidationErrors> for ApiError {
    fn from(errors: ValidationErrors) -> Self {
        Self {
            validation_errors: Some(errors.into_fields()),
            ..Self::detail(StatusCode::UNPROCESSABLE_ENTITY, "validation failed")
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(cause: anyhow::Error) -> Self {
        Self::internal(cause)
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt as _;
    use serde_json::json;

    use super::*;
    use crate::domain::user::Permission;
    use crate::testing::identity;

    #[test]
    fn from_denied_missing_permission_returns_forbidden() {
        let denied = identity::actor_with_roles(&[])
            .authorize(Permission::MachineDelete)
            .unwrap_err();

        let error = ApiError::from(denied);

        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn into_response_returns_the_envelope_with_status_code_and_detail() {
        let response = ApiError::detail(StatusCode::NOT_FOUND, "no such user").into_response();

        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({ "status": 404, "detail": "no such user" })
        );
    }

    #[tokio::test]
    async fn into_response_unexpected_error_hides_its_cause() {
        let error = ApiError::from(anyhow::anyhow!("no such table: machines"));

        let response = error.into_response();

        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({ "status": 500 })
        );
    }
}
