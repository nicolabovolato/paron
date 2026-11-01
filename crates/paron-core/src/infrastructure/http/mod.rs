pub mod auth;
pub mod error;
pub mod pagination;
pub mod public_origins;
pub mod routes;
pub mod state;

use axum::{Json, Router};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable as _};

use auth::{RecoveryTokenSecurityScheme, SessionCookieSecurityScheme};
use error::ApiError;
use state::Deps;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Paron API",
        version = env!("CARGO_PKG_VERSION"),
        license(
            name = "PolyForm Shield License 1.0.0",
            url = "https://polyformproject.org/licenses/shield/1.0.0",
        ),
    ),
    components(schemas(ApiError), responses(ApiError)),
    modifiers(&SessionCookieSecurityScheme, &RecoveryTokenSecurityScheme),
)]
struct OpenApiSpec;

#[derive(Clone, Copy)]
pub struct Config {
    pub expose_docs: bool,
    pub websocket_keepalive_interval: std::time::Duration,
}

pub fn router(deps: Deps) -> Router {
    let (router, document) = api().split_for_parts();

    let router = if deps.config.expose_docs {
        router
            .route(
                "/api/openapi.json",
                axum::routing::get({
                    let document = document.clone();
                    async move || Json(document)
                }),
            )
            .merge(Scalar::with_url("/api/docs", document))
    } else {
        router
    };

    router
        .layer(axum::middleware::map_response(
            error::normalise_error_response,
        ))
        .layer(axum::middleware::from_fn(auth::resource_isolation_policy))
        .layer(axum::middleware::from_fn_with_state(
            std::sync::Arc::clone(&deps.public_origins),
            public_origins::match_public_origin,
        ))
        .with_state(deps)
}

pub fn openapi() -> utoipa::openapi::OpenApi {
    api().into_openapi()
}

fn api() -> OpenApiRouter<Deps> {
    let mut api = OpenApiRouter::with_openapi(OpenApiSpec::openapi()).nest(
        "/api",
        OpenApiRouter::new()
            .merge(routes::auth::routes())
            .merge(routes::account::routes())
            .merge(routes::audit::routes())
            .merge(routes::machines::routes())
            .merge(routes::users::routes())
            .fallback(async || ApiError::detail(axum::http::StatusCode::NOT_FOUND, "")),
    );

    error::declare_error_responses(api.get_openapi_mut());

    api
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU16;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::*;
    use crate::application::identity::authentication::methods::passkey::PasskeyName;
    use crate::application::identity::authentication::methods::password::Password;
    use crate::application::identity::authorization::actor::Denied;
    use crate::application::identity::errors::ListCredentialsError;
    use crate::application::identity::use_cases::{
        MockAccountRecovery, MockCredentialManagement, MockUserManagement,
    };
    use crate::application::machine::{
        MockMachineConnections, MockMachineEnrollment, MockMachineManagement,
    };
    use crate::domain::machine::{Host, MachineName, ssh};
    use crate::domain::user::{DisplayName, Permission, Username};
    use crate::testing::http::{self, CONFIG, SESSION_COOKIE};
    use crate::testing::keys;

    const PUBLIC_OPERATIONS: [(&str, &str); 4] = [
        ("post", "/api/auth/sign-in"),
        ("post", "/api/auth/sign-in/verify"),
        ("get", "/api/auth/recovery"),
        ("post", "/api/auth/recovery"),
    ];

    // These act only on the session of the user, or filter what they return. The terminal
    // denies with a close frame: see `websocket.rs`.
    const OPERATIONS_THAT_DENY_NOTHING: [(&str, &str); 11] = [
        ("post", "/api/auth/sign-in"),
        ("post", "/api/auth/sign-in/verify"),
        ("get", "/api/auth/recovery"),
        ("post", "/api/auth/recovery"),
        ("delete", "/api/auth/session"),
        ("get", "/api/auth/session"),
        ("get", "/api/account"),
        ("get", "/api/account/credentials"),
        ("post", "/api/account/credentials"),
        ("get", "/api/audit"),
        ("get", "/api/machines/{id}/terminal"),
    ];

    fn openapi_document() -> Value {
        serde_json::to_value(openapi()).unwrap()
    }

    fn openapi_operations() -> impl Iterator<Item = (String, String, Value)> {
        const HTTP_METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

        let paths = openapi_document()["paths"].as_object().unwrap().clone();

        paths.into_iter().flat_map(|(path, item)| {
            HTTP_METHODS.into_iter().filter_map(move |method| {
                let operation = item.get(method)?.clone();

                Some((method.to_owned(), path.clone(), operation))
            })
        })
    }

    // An optional field is `oneOf [null, ...]`, and its bounds are in the branch that is not null.
    fn resolve_property<'a>(document: &'a Value, property: &'a Value) -> &'a Value {
        let property = property["oneOf"]
            .as_array()
            .and_then(|branches| branches.iter().find(|branch| branch["type"] != "null"))
            .unwrap_or(property);

        match property["$ref"].as_str().and_then(|target| {
            target
                .strip_prefix("#/components/schemas/")
                .map(|name| &document["components"]["schemas"][name])
        }) {
            Some(component) => component,
            None => property,
        }
    }

    fn length_bounds(document: &Value, schema: &str, field: &str) -> (usize, usize) {
        let property = resolve_property(
            document,
            &document["components"]["schemas"][schema]["properties"][field],
        );
        let min = property["minLength"].as_u64().unwrap();
        let max = property["maxLength"].as_u64().unwrap();

        (usize::try_from(min).unwrap(), usize::try_from(max).unwrap())
    }

    #[test]
    fn openapi_operations_require_a_session_except_the_public_ones() {
        let security_schemes = openapi_document()["components"]["securitySchemes"].clone();

        let unguarded_operations: Vec<String> = openapi_operations()
            .filter(|(method, path, _)| {
                !PUBLIC_OPERATIONS.contains(&(method.as_str(), path.as_str()))
            })
            .filter(|(_, _, operation)| {
                // A scheme name that the document does not declare guards nothing.
                let is_guarded = operation["security"]
                    .as_array()
                    .is_some_and(|requirements| {
                        requirements.iter().any(|requirement| {
                            requirement.as_object().is_some_and(|scopes_by_scheme| {
                                scopes_by_scheme
                                    .keys()
                                    .any(|scheme_name| !security_schemes[scheme_name].is_null())
                            })
                        })
                    });

                !is_guarded
            })
            .map(|(method, path, _)| format!("{} {path}", method.to_uppercase()))
            .collect();

        assert_eq!(unguarded_operations, Vec::<String>::new());
    }

    #[test]
    fn openapi_operations_declare_the_error_envelope() {
        let operations_without_error_envelope: Vec<String> = openapi_operations()
            .filter(|(_, _, operation)| {
                ["4XX", "5XX"].iter().any(|range| {
                    operation["responses"][range]["$ref"] != "#/components/responses/ApiError"
                })
            })
            .map(|(method, path, _)| format!("{} {path}", method.to_uppercase()))
            .collect();

        assert_eq!(operations_without_error_envelope, Vec::<String>::new());
    }

    #[test]
    fn openapi_operation_ids_are_unique() {
        let mut operations_by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for (method, path, operation) in openapi_operations() {
            let id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{} {path} has no operationId", method.to_uppercase()))
                .to_owned();

            operations_by_id
                .entry(id)
                .or_default()
                .push(format!("{} {path}", method.to_uppercase()));
        }
        operations_by_id.retain(|_, operations| operations.len() > 1);

        assert_eq!(operations_by_id, BTreeMap::new());
    }

    #[test]
    fn openapi_request_bodies_list_their_required_fields() {
        let document = openapi_document();

        for (schema, expected_fields) in [
            (
                "CreateUserRequest",
                &["display_name", "roles", "username"][..],
            ),
            ("MachineRequest", &["connectivity", "name", "protocols"][..]),
            ("PartialUpdateUserRequest", &[][..]),
        ] {
            let mut required_fields: Vec<&str> =
                document["components"]["schemas"][schema]["required"]
                    .as_array()
                    .map_or_else(Vec::new, |names| {
                        names.iter().filter_map(Value::as_str).collect()
                    });
            required_fields.sort_unstable();

            assert_eq!(required_fields, expected_fields, "{schema}");
        }
    }

    // `RawString` and `RawNumber` are internal readers of a plain `String` or `u64`. A field
    // that uses one must state its shape inline.
    #[test]
    fn openapi_names_no_reader_as_a_component() {
        let document = openapi_document();
        let schemas = document["components"]["schemas"].as_object().unwrap();

        for reader in ["RawString", "RawNumber"] {
            assert!(
                !schemas.contains_key(reader),
                "{reader} reached the document: {:?}",
                schemas.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn openapi_references_all_resolve() {
        let document = openapi_document();
        let mut unresolved_references = Vec::new();
        let mut values_to_visit = vec![&document];

        while let Some(value) = values_to_visit.pop() {
            match value {
                Value::Object(fields) => {
                    if let Some(Value::String(target)) = fields.get("$ref")
                        && document.pointer(target.trim_start_matches('#')).is_none()
                    {
                        unresolved_references.push(target.clone());
                    }

                    values_to_visit.extend(fields.values());
                }
                Value::Array(items) => values_to_visit.extend(items),
                _ => {}
            }
        }
        unresolved_references.sort();
        unresolved_references.dedup();

        assert_eq!(unresolved_references, Vec::<String>::new());
    }

    #[test]
    fn openapi_username_bounds_match_the_parser() {
        let document = openapi_document();

        for (schema, field) in [
            ("CreateUserRequest", "username"),
            ("PartialUpdateUserRequest", "username"),
        ] {
            let (min, max) = length_bounds(&document, schema, field);

            assert!(Username::parse(&"a".repeat(min)).is_ok(), "{schema}");
            assert!(Username::parse(&"a".repeat(min - 1)).is_err(), "{schema}");
            assert!(Username::parse(&"a".repeat(max)).is_ok(), "{schema}");
            assert!(Username::parse(&"a".repeat(max + 1)).is_err(), "{schema}");
        }
    }

    #[test]
    fn openapi_limited_text_bounds_match_the_parser() {
        type Parses = fn(String) -> bool;

        let document = openapi_document();
        let parsers: [(&str, Parses); 4] = [
            ("DisplayName", |text| DisplayName::parse(text).is_ok()),
            ("PasskeyName", |text| PasskeyName::parse(text).is_ok()),
            ("MachineName", |text| MachineName::parse(text).is_ok()),
            ("SshUser", |text| ssh::User::parse(text).is_ok()),
        ];

        for (schema, parses) in parsers {
            let text = &document["components"]["schemas"][schema];
            let bound = |keyword: &str| usize::try_from(text[keyword].as_u64().unwrap()).unwrap();
            let (min, max) = (bound("minLength"), bound("maxLength"));

            assert!(parses("a".repeat(min)), "{schema}");
            assert!(!parses("a".repeat(min - 1)), "{schema}");
            assert!(parses("a".repeat(max)), "{schema}");
            assert!(!parses("a".repeat(max + 1)), "{schema}");
        }
    }

    #[test]
    fn openapi_host_bound_matches_the_parser() {
        let document = openapi_document();
        let max = document["components"]["schemas"]["Host"]["maxLength"]
            .as_u64()
            .unwrap();
        let max = usize::try_from(max).unwrap();

        assert!(Host::parse(&"a".repeat(max)).is_ok());
        assert!(Host::parse(&"a".repeat(max + 1)).is_err());
    }

    #[test]
    fn openapi_password_bounds_match_the_parser() {
        type Parses = fn(SecretString) -> bool;

        let document = openapi_document();
        let parsers: [(&str, Parses); 2] = [
            ("Password", |password| Password::parse(password).is_ok()),
            ("SshPassword", |password| {
                ssh::Password::parse(password).is_ok()
            }),
        ];

        for (schema, parses) in parsers {
            let password = &document["components"]["schemas"][schema];
            let bound =
                |keyword: &str| usize::try_from(password[keyword].as_u64().unwrap()).unwrap();
            let (min, max) = (bound("minLength"), bound("maxLength"));
            let password_of_length = |length: usize| SecretString::from("a".repeat(length));

            assert!(parses(password_of_length(min)), "{schema}");
            assert!(!parses(password_of_length(min - 1)), "{schema}");
            assert!(parses(password_of_length(max)), "{schema}");
            assert!(!parses(password_of_length(max + 1)), "{schema}");
        }
    }

    #[test]
    fn openapi_property_names_are_snake_case() {
        let document = openapi_document();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        let property_names = |properties: Option<&serde_json::Map<String, Value>>| {
            properties
                .into_iter()
                .flat_map(|properties| properties.keys())
                .cloned()
                .collect::<Vec<_>>()
        };

        for (schema, body) in schemas {
            // A tagged enum keeps its fields under `oneOf`, not at the top level.
            let variants = body["oneOf"].as_array().map_or(&[][..], Vec::as_slice);
            let properties = property_names(body["properties"].as_object())
                .into_iter()
                .chain(
                    variants
                        .iter()
                        .flat_map(|variant| property_names(variant["properties"].as_object())),
                );

            for property in properties {
                assert!(
                    property
                        .chars()
                        .all(|character| character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || character == '_'),
                    "{schema}.{property} is not the snake_case name that a validation error uses",
                );
            }
        }
    }

    #[test]
    fn openapi_port_range_matches_the_parser() {
        let document = openapi_document();
        let port = resolve_property(
            &document,
            &document["components"]["schemas"]["SshRequest"]["properties"]["port"],
        );

        let min = u16::try_from(port["minimum"].as_u64().unwrap()).unwrap();
        let max = port["maximum"].as_u64().unwrap();

        assert!(NonZeroU16::new(min).is_some());
        assert!(NonZeroU16::new(min - 1).is_none());
        assert_eq!(max, u64::from(u16::MAX));
    }

    #[test]
    fn openapi_username_pattern_accepts_what_the_parser_does() {
        let document = openapi_document();
        let username = resolve_property(
            &document,
            &document["components"]["schemas"]["CreateUserRequest"]["properties"]["username"],
        );
        let pattern = regex::Regex::new(username["pattern"].as_str().unwrap()).unwrap();

        for candidate in [
            "admin",
            "Admin",
            "  admin  ",
            "a.b",
            "a-b",
            "a_b",
            "a1.b2-c3_d4",
            "abc",
            "Not A Username",
            ".admin",
            "admin.",
            "a..b",
            "a b",
            "adm!n",
            "ab",
        ] {
            let is_valid_username = Username::parse(candidate).is_ok();
            let matches_pattern = pattern.is_match(candidate);

            assert!(
                !is_valid_username || matches_pattern,
                "{candidate:?} is accepted by the parser and refused by the pattern"
            );
        }
    }

    #[tokio::test]
    async fn router_guarded_operation_without_cookie_returns_unauthorized() {
        let api = router(http::deps());
        let path_parameter = regex::Regex::new(r"\{[^}]+\}").unwrap();
        let guarded_operations: Vec<_> = openapi_operations()
            .filter(|(method, path, _)| {
                !PUBLIC_OPERATIONS.contains(&(method.as_str(), path.as_str()))
            })
            .collect();

        for (method, path, _) in guarded_operations {
            let concrete_path = path_parameter.replace_all(&path, uuid::Uuid::now_v7().to_string());

            let (status, _) = http::send(
                &api,
                Request::builder()
                    .method(method.to_uppercase().as_str())
                    .uri(&*concrete_path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn router_operation_denied_for_the_session_user_returns_forbidden() {
        let denied = || Denied::MissingPermission(Permission::UserWrite);
        let mut users = MockUserManagement::new();
        users
            .expect_list()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        users
            .expect_get_by_id()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        users
            .expect_create()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        users
            .expect_update()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        users
            .expect_delete()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        let mut credentials = MockCredentialManagement::new();
        credentials
            .expect_list()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(ListCredentialsError::Denied(denied())));
        let mut account_recovery = MockAccountRecovery::new();
        account_recovery
            .expect_start_recovery()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        let mut machines = MockMachineManagement::new();
        machines
            .expect_list()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        machines
            .expect_get()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        machines
            .expect_create()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        machines
            .expect_update()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        machines
            .expect_delete()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        let mut machine_connections = MockMachineConnections::new();
        machine_connections
            .expect_test()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        let mut machine_enrollment = MockMachineEnrollment::new();
        machine_enrollment
            .expect_issue_enrollment_token()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        machine_enrollment
            .expect_unenroll()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(move |_, _| Err(denied().into()));
        let api = router(Deps {
            account_recovery: Arc::new(account_recovery),
            credentials: Arc::new(credentials),
            users: Arc::new(users),
            machines: Arc::new(machines),
            machine_connections: Arc::new(machine_connections),
            machine_enrollment: Arc::new(machine_enrollment),
            ..http::deps()
        });
        let host_key = ssh_key::PublicKey::from_bytes(keys::SSH_PUBLIC_KEY)
            .unwrap()
            .to_openssh()
            .unwrap();
        let machine = json!({
            "name": "web",
            "connectivity": { "kind": "tunneled" },
            "protocols": {
                "ssh": {
                    "user": "deploy",
                    "port": 22,
                    "authentication": {
                        "host_key": host_key,
                        "method": { "kind": "password", "password": "hunter2" },
                    },
                    "connect_timeout_seconds": 30,
                },
            },
        });
        let operations = [
            ("get", "/api/users", None),
            (
                "post",
                "/api/users",
                Some(json!({
                    "username": "someone",
                    "display_name": "Someone",
                    "roles": [],
                })),
            ),
            ("get", "/api/users/{id}", None),
            (
                "patch",
                "/api/users/{id}",
                Some(json!({ "display_name": "Someone" })),
            ),
            ("delete", "/api/users/{id}", None),
            ("get", "/api/users/{id}/credentials", None),
            ("post", "/api/users/{id}/recovery", None),
            ("get", "/api/machines", None),
            ("post", "/api/machines", Some(machine.clone())),
            ("get", "/api/machines/{id}", None),
            (
                "patch",
                "/api/machines/{id}",
                Some(json!({ "name": "web" })),
            ),
            ("delete", "/api/machines/{id}", None),
            (
                "post",
                "/api/machines/{id}/test-connection?protocol=ssh",
                None,
            ),
            (
                "post",
                "/api/machines/test-connection?protocol=ssh",
                Some(machine),
            ),
            ("post", "/api/machines/{id}/enrollment", None),
            ("delete", "/api/machines/{id}/enrollment", None),
        ];

        let mut checked_operations: Vec<(String, String)> = operations
            .iter()
            .map(|(method, uri, _)| {
                let path = uri.split_once('?').map_or(*uri, |(path, _)| path);

                ((*method).to_owned(), path.to_owned())
            })
            .chain(
                OPERATIONS_THAT_DENY_NOTHING
                    .iter()
                    .map(|(method, path)| ((*method).to_owned(), (*path).to_owned())),
            )
            .collect();
        let mut documented_operations: Vec<(String, String)> = openapi_operations()
            .map(|(method, path, _)| (method, path))
            .collect();
        checked_operations.sort();
        documented_operations.sort();
        assert_eq!(checked_operations, documented_operations);

        for (method, path, body) in operations {
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(path.replace("{id}", &uuid::Uuid::now_v7().to_string()))
                .header(header::COOKIE, SESSION_COOKIE);
            let request = match body {
                Some(body) => request
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string())),
                None => request.body(Body::empty()),
            };

            let (status, body) = http::send(&api, request.unwrap()).await;

            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
        }
    }

    #[tokio::test]
    async fn router_unknown_api_path_returns_not_found() {
        let (status, _) = http::send(
            &router(http::deps()),
            Request::get("/api/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn router_request_under_a_host_that_is_not_configured_returns_misdirected_request() {
        let api = router(http::deps());

        for host in [
            "evil.example",
            "paron.example",
            "paron.example:9443",
            "10.0.0.5",
        ] {
            let (status, body) = http::send(
                &api,
                Request::get("/api/auth/session")
                    .header(header::HOST, host)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

            assert_eq!(status, StatusCode::MISDIRECTED_REQUEST, "{host}");
            assert_eq!(body["status"], 421, "{host}: {body}");
        }
    }

    #[tokio::test]
    async fn router_invalid_body_returns_validation_errors_in_the_envelope() {
        let api = router(http::deps());

        let (status, body) = http::send(
            &api,
            Request::post("/api/users")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "username": "Not A Username",
                        "display_name": "Someone",
                        "roles": [],
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body["validation_errors"]["username"].is_array(), "{body}");
        assert!(body["detail"].is_string(), "{body}");
    }

    #[tokio::test]
    async fn router_malformed_json_returns_bad_request_in_the_envelope() {
        let api = router(http::deps());

        let (status, body) = http::send(
            &api,
            Request::post("/api/users")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{ not json"))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["status"], 400, "{body}");
    }

    #[tokio::test]
    async fn router_body_that_is_not_json_returns_unsupported_media_type() {
        let api = router(http::deps());

        let (status, body) = http::send(
            &api,
            Request::post("/api/users")
                .header(header::COOKIE, SESSION_COOKIE)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("someone"))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body["status"], 415, "{body}");
    }

    #[tokio::test]
    async fn router_with_exposed_docs_serves_the_document() {
        let api = router(http::deps());

        for path in ["/api/openapi.json", "/api/docs"] {
            let (status, _) =
                http::send(&api, Request::get(path).body(Body::empty()).unwrap()).await;

            assert_eq!(status, StatusCode::OK, "{path}");
        }
    }

    #[tokio::test]
    async fn router_without_exposed_docs_does_not_serve_the_document() {
        let api = router(Deps {
            config: Config {
                expose_docs: false,
                ..CONFIG
            },
            ..http::deps()
        });

        for path in ["/api/openapi.json", "/api/docs"] {
            let (status, _) =
                http::send(&api, Request::get(path).body(Body::empty()).unwrap()).await;

            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }
}
