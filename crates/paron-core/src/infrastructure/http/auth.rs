use crate::application::identity::authorization::actor::{Actor, AuthenticatedUser};
use crate::application::identity::errors::{AuthenticateError, ContinueSignInError};
use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use secrecy::SecretString;
use utoipa::Modify;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};

use super::error::ApiError;
use super::state::Deps;

const SESSION_COOKIE: &str = "__Host-Http-paron_session";
const SIGN_IN_COOKIE: &str = "__Host-Http-paron_sign_in";
const DEVICE_COOKIE: &str = "__Host-Http-paron_device";
const DEVICE_COOKIE_LIFETIME: cookie::time::Duration = cookie::time::Duration::days(400);

pub(super) struct SessionCookieSecurityScheme;

impl Modify for SessionCookieSecurityScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_default()
            .add_security_scheme(
                "session",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(SESSION_COOKIE))),
            );
    }
}

pub(super) struct RecoveryTokenSecurityScheme;

impl Modify for RecoveryTokenSecurityScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_default()
            .add_security_scheme(
                "recovery_token",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
    }
}

pub struct RecoveryToken(pub SecretString);

impl FromRequestParts<Deps> for RecoveryToken {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &Deps,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split_once(' '))
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map(|(_, token)| Self(SecretString::from(token.trim())))
                .ok_or_else(|| {
                    ApiError::detail(
                        axum::http::StatusCode::BAD_REQUEST,
                        "missing recovery token",
                    )
                }),
        )
    }
}

#[derive(derive_more::Deref)]
pub struct CurrentUser(AuthenticatedUser);

impl From<CurrentUser> for AuthenticatedUser {
    fn from(user: CurrentUser) -> Self {
        user.0
    }
}

impl From<CurrentUser> for Actor {
    fn from(user: CurrentUser) -> Self {
        Self::User(user.0)
    }
}

impl FromRequestParts<Deps> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Deps) -> Result<Self, Self::Rejection> {
        let cookies = CookieJar::from_headers(&parts.headers);
        let session_token = cookies
            .get(SESSION_COOKIE)
            .map(|cookie| SecretString::from(cookie.value().to_owned()))
            .ok_or_else(|| {
                ApiError::detail(
                    axum::http::StatusCode::UNAUTHORIZED,
                    "missing session cookie".to_owned(),
                )
            })?;

        let authenticated = deps.authentication.authenticate(&session_token).await?;

        Ok(Self(authenticated))
    }
}

/// A sign-in that waits for its second step. Its own cookie, so that it never counts as a
/// session.
pub fn sign_in_cookie(value: String, expires_at: jiff::Timestamp) -> Cookie<'static> {
    let expires: cookie::time::OffsetDateTime = std::time::SystemTime::from(expires_at).into();

    Cookie::build((SIGN_IN_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .expires(expires)
        .build()
}

pub struct SignInToken(pub SecretString);

impl FromRequestParts<Deps> for SignInToken {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &Deps,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            CookieJar::from_headers(&parts.headers)
                .get(SIGN_IN_COOKIE)
                .map(|cookie| Self(SecretString::from(cookie.value().to_owned())))
                .ok_or_else(|| ContinueSignInError::SignInEnded.into()),
        )
    }
}

/// The device token of the sign-in service. It stays after the sign-out.
pub fn device_cookie(value: String) -> Cookie<'static> {
    Cookie::build((DEVICE_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .max_age(DEVICE_COOKIE_LIFETIME)
        .build()
}

pub struct DeviceCookie(pub SecretString);

impl OptionalFromRequestParts<Deps> for DeviceCookie {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _: &Deps,
    ) -> impl Future<Output = Result<Option<Self>, Self::Rejection>> + Send {
        std::future::ready(Ok(CookieJar::from_headers(&parts.headers)
            .get(DEVICE_COOKIE)
            .map(|cookie| Self(SecretString::from(cookie.value().to_owned())))))
    }
}

pub fn session_cookie(value: String, expires_at: jiff::Timestamp) -> Cookie<'static> {
    let expires: cookie::time::OffsetDateTime = std::time::SystemTime::from(expires_at).into();

    Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .expires(expires)
        .build()
}

pub fn remove_sign_in_cookie(cookies: CookieJar) -> CookieJar {
    cookies.remove(Cookie::build(SIGN_IN_COOKIE).path("/").secure(true))
}

pub fn remove_session_cookie(cookies: CookieJar) -> CookieJar {
    cookies.remove(Cookie::build(SESSION_COOKIE).path("/").secure(true))
}

/// The Resource Isolation Policy of Fetch Metadata.
///
/// The browser sends the header `Sec-Fetch-Site`. It tells where the request comes from, and a page
/// cannot change it. The session cookie is `SameSite=Strict`, but a page on a sibling subdomain is on
/// the same site, so the browser sends the cookie with its requests. A WebSocket has no CORS check.
///
/// This policy refuses a request from another site. It accepts a request that opens a page, as a link
/// does. It accepts a request without the header, because only a browser sends it.
pub async fn resource_isolation_policy(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;

    let headers = request.headers();
    let from_other_site = headers
        .get("sec-fetch-site")
        .is_some_and(|site| site != "same-origin" && site != "none");
    let opens_page = request.method() == axum::http::Method::GET
        && headers
            .get("sec-fetch-mode")
            .is_some_and(|mode| mode == "navigate");

    if from_other_site && !opens_page {
        return ApiError::detail(
            axum::http::StatusCode::FORBIDDEN,
            "the request comes from another site",
        )
        .into_response();
    }

    next.run(request).await
}

impl From<AuthenticateError> for ApiError {
    fn from(error: AuthenticateError) -> Self {
        match error {
            AuthenticateError::SessionEnded => {
                Self::detail(axum::http::StatusCode::UNAUTHORIZED, error.to_string())
            }
            AuthenticateError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::application::identity::errors::AuthenticateError;
    use crate::application::identity::use_cases::MockAuthentication;
    use crate::infrastructure::http::router;
    use crate::testing::http::{self, SESSION_COOKIE};

    #[test]
    fn session_cookie_of_a_token_returns_a_strict_secure_cookie_that_expires_with_the_session() {
        let expires_at = jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_hours(1);

        let cookie = session_cookie("a-token".to_owned(), expires_at);

        assert_eq!(
            cookie.to_string(),
            "__Host-Http-paron_session=a-token; HttpOnly; SameSite=Strict; Secure; Path=/; \
             Expires=Thu, 01 Jan 1970 01:00:00 GMT"
        );
    }

    #[tokio::test]
    async fn resource_isolation_policy_same_origin_or_page_navigation_returns_ok() {
        let api = router(http::deps());

        for (site, mode) in [
            (None, None),
            (Some("same-origin"), Some("cors")),
            (Some("none"), Some("navigate")),
            (Some("cross-site"), Some("navigate")),
        ] {
            let mut request =
                Request::get("/api/auth/session").header(header::COOKIE, SESSION_COOKIE);
            if let Some(site) = site {
                request = request.header("sec-fetch-site", site);
            }
            if let Some(mode) = mode {
                request = request.header("sec-fetch-mode", mode);
            }

            let (status, _) = http::send(&api, request.body(Body::empty()).unwrap()).await;

            assert_eq!(status, StatusCode::OK, "{site:?} {mode:?}");
        }
    }

    #[tokio::test]
    async fn resource_isolation_policy_from_another_site_returns_forbidden() {
        let api = router(http::deps());

        for (method, site, mode) in [
            ("GET", "same-site", "cors"),
            ("GET", "cross-site", "websocket"),
            ("POST", "cross-site", "navigate"),
        ] {
            let request = Request::builder()
                .method(method)
                .uri("/api/auth/session")
                .header(header::COOKIE, SESSION_COOKIE)
                .header("sec-fetch-site", site)
                .header("sec-fetch-mode", mode)
                .body(Body::empty())
                .unwrap();

            let (status, _) = http::send(&api, request).await;

            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {site} {mode}");
        }
    }

    #[tokio::test]
    async fn from_request_parts_reads_the_token_from_the_session_cookie() {
        let mut authentication = MockAuthentication::new();
        authentication
            .expect_authenticate()
            .once()
            .withf(|token| token.expose_secret() == "a-token")
            .returning(|_| Err(AuthenticateError::SessionEnded));
        let api = router(Deps {
            authentication: Arc::new(authentication),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get("/api/auth/session")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["detail"], "session ended");
    }
}
