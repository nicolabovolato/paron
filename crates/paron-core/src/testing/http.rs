use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::Value;
use tower::ServiceExt as _;

use crate::application::audit::MockAuditReader;
use crate::application::identity::authorization::actor::{Actor, AuthenticatedUser};
use crate::application::identity::use_cases::{
    MockAccountRecovery, MockAuthentication, MockCredentialManagement, MockUserManagement,
};
use crate::application::machine::{
    MockMachineConnections, MockMachineEnrollment, MockMachineManagement,
};
use crate::domain::user::User;
use crate::infrastructure::http::Config;
use crate::infrastructure::http::public_origins::PublicOrigins;
use crate::infrastructure::http::state::Deps;
use crate::testing::identity;

pub const CONFIG: Config = Config {
    expose_docs: true,
    websocket_keepalive_interval: Duration::from_secs(30),
};
pub const ORIGIN: &str = "https://paron.example:8443";
pub const HOST: &str = "paron.example:8443";
pub const OTHER_ORIGIN: &str = "https://paron.local";
pub const SESSION_COOKIE: &str = "__Host-Http-paron_session=a-token";
pub const SESSION_ID: uuid::Uuid = uuid::Uuid::from_u128(1);

pub fn signed_in_user() -> User {
    identity::user_builder()
        .id(uuid::Uuid::from_u128(2))
        .build()
}

pub fn is_signed_in_user(actor: &Actor) -> bool {
    actor.user().is_some_and(|authenticated| {
        authenticated.session_id == SESSION_ID && authenticated.user.id == signed_in_user().id
    })
}

/// Any session cookie authenticates as [`signed_in_user`].
pub fn authentication() -> MockAuthentication {
    let mut authentication = MockAuthentication::new();
    authentication.expect_authenticate().returning(|_| {
        Ok(AuthenticatedUser {
            session_id: SESSION_ID,
            recently_authenticated_until: jiff::Timestamp::MAX,
            user: signed_in_user(),
        })
    });

    authentication
}

/// Each use case is a mock with no expectation, except [`authentication`].
pub fn deps() -> Deps {
    Deps {
        machines: Arc::new(MockMachineManagement::new()),
        machine_connections: Arc::new(MockMachineConnections::new()),
        machine_enrollment: Arc::new(MockMachineEnrollment::new()),
        authentication: Arc::new(authentication()),
        credentials: Arc::new(MockCredentialManagement::new()),
        account_recovery: Arc::new(MockAccountRecovery::new()),
        users: Arc::new(MockUserManagement::new()),
        audit: Arc::new(MockAuditReader::new()),
        public_origins: Arc::new(PublicOrigins::new(
            url::Url::parse(ORIGIN).unwrap().origin(),
            [url::Url::parse(OTHER_ORIGIN).unwrap().origin()],
        )),
        config: CONFIG,
    }
}

/// Sends the request to the router, under the host of [`ORIGIN`] when it names none. The body is
/// `Value::Null` when it is not JSON.
pub async fn send(api: &Router, mut request: Request<Body>) -> (StatusCode, Value) {
    if !request.headers().contains_key(header::HOST) {
        request
            .headers_mut()
            .insert(header::HOST, HOST.parse().unwrap());
    }
    let response = api.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();

    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}
