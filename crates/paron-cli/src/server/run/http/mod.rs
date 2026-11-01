pub mod connection;
mod deadline;
pub mod frontend;

use std::any::Any;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, LazyLock};

use std::time::Duration;

use axum::http::{HeaderName, Request};

use axum::response::{IntoResponse as _, Response};
use paron_core::infrastructure::http::error::ApiError;
use tower::util::MapRequestLayer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::ResponseBodyTimeoutLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::server::paron::Paron;
use crate::server::run::background_tasks::BackgroundTasks;
use crate::server::run::http::frontend::Frontend;
use crate::server::run::tls::self_signed::SelfSignedCertificate;
use governor::middleware::NoOpMiddleware;
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::{GovernorConfig, GovernorConfigBuilder};
use tower_governor::key_extractor::KeyExtractor;

const RATE_LIMIT_FRONTEND_PER_SECOND: u32 = 25;
const RATE_LIMIT_FRONTEND_BURST: u32 = if cfg!(debug_assertions) { 10_000 } else { 100 };

const RATE_LIMIT_API_PER_SECOND: u32 = 10;
const RATE_LIMIT_API_BURST: u32 = if cfg!(debug_assertions) { 10_000 } else { 25 };

const RATE_LIMIT_API_AUTH_PER_MINUTE: u32 = 5;
const RATE_LIMIT_API_AUTH_BURST: u32 = 25;
const ROUTES_WITH_API_AUTH_RATE_LIMIT: &[(axum::http::Method, &str)] = &[
    (axum::http::Method::POST, "/api/auth/sign-in"),
    (axum::http::Method::POST, "/api/auth/sign-in/verify"),
    (axum::http::Method::POST, "/api/account/credentials"),
];

const RATE_LIMIT_CLEANUP_STALE_INTERVAL: Duration = Duration::from_mins(1);

const MAX_REQUEST_DURATION: Duration = Duration::from_secs(10);
const ROUTES_WITHOUT_MAX_REQUEST_DURATION: &[(axum::http::Method, &str)] = &[
    (axum::http::Method::POST, "/api/machines/test-connection"),
    (
        axum::http::Method::POST,
        "/api/machines/{id}/test-connection",
    ),
];

const MAX_REQUEST_BODY_SIZE_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BODY_DURATION: Duration = Duration::from_secs(10);

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

const ACCESS_LOG_LEVEL: Level = if cfg!(debug_assertions) {
    Level::DEBUG
} else {
    Level::INFO
};

pub fn build(
    server: &Paron,
    trusted_proxies: &crate::server::run::proxy::L7Proxies,
    background: &mut BackgroundTasks,
    self_signed: SelfSignedCertificate,
    #[cfg(debug_assertions)] dev_server: bool,
) -> crate::Result<axum::Router> {
    let frontend = Frontend::embedded();

    #[cfg(debug_assertions)]
    let frontend = dev_server
        .then(Frontend::start_dev_server)
        .transpose()?
        .unwrap_or(frontend);

    let rate_limit_key_extractor = ProxiedIpKeyExtractor(trusted_proxies.clone());
    let l7_proxies = trusted_proxies.clone();

    let api_rate_limit = Arc::new(
        GovernorConfigBuilder::default()
            .period(Duration::from_secs(1) / RATE_LIMIT_API_PER_SECOND)
            .burst_size(RATE_LIMIT_API_BURST)
            .key_extractor(rate_limit_key_extractor.clone())
            .finish()
            .ok_or_else(|| "could not configure the API rate limit".to_owned())?,
    );
    let api_auth_rate_limit = Arc::new(
        GovernorConfigBuilder::default()
            .period(Duration::from_mins(1) / RATE_LIMIT_API_AUTH_PER_MINUTE)
            .burst_size(RATE_LIMIT_API_AUTH_BURST)
            .key_extractor(rate_limit_key_extractor.clone())
            .finish()
            .ok_or_else(|| "could not configure the API auth rate limit".to_owned())?,
    );
    let frontend_rate_limit = Arc::new(
        GovernorConfigBuilder::default()
            .period(Duration::from_secs(1) / RATE_LIMIT_FRONTEND_PER_SECOND)
            .burst_size(RATE_LIMIT_FRONTEND_BURST)
            .key_extractor(rate_limit_key_extractor)
            .finish()
            .ok_or_else(|| "could not configure the assets rate limit".to_owned())?,
    );

    background.add({
        let rate_limiters = [
            Arc::clone(api_rate_limit.limiter()),
            Arc::clone(api_auth_rate_limit.limiter()),
            Arc::clone(frontend_rate_limit.limiter()),
        ];

        async move {
            loop {
                tokio::time::sleep(RATE_LIMIT_CLEANUP_STALE_INTERVAL).await;

                for rate_limit in &rate_limiters {
                    rate_limit.retain_recent();
                    rate_limit.shrink_to_fit();
                }
            }
        }
    });

    let api = server
        .http
        .clone()
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let rate_limit = Arc::clone(&api_auth_rate_limit);

                async move { limit_api_auth(&rate_limit, request, next).await }
            },
        ))
        .layer(GovernorLayer::new(api_rate_limit));
    let frontend = frontend
        .into_router()
        .layer(GovernorLayer::new(frontend_rate_limit));

    let router = api
        .fallback_service(frontend)
        .route(
            "/paron-ca.crt",
            axum::routing::get(move || async move {
                let certificate = self_signed.ca_certificate();

                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/x-x509-ca-cert",
                    )],
                    pem::encode(&pem::Pem::new("CERTIFICATE", certificate.to_vec())),
                )
            }),
        )
        .route(
            "/health",
            axum::routing::get(|| async { axum::http::StatusCode::OK }),
        );

    Ok(router
        .layer(axum::middleware::from_fn(time_limit))
        .layer(ResponseBodyTimeoutLayer::new(MAX_RESPONSE_BODY_DURATION))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_REQUEST_BODY_SIZE_BYTES,
        ))
        // The body limit and the rate limits answer in plain text; the API promises an ApiError.
        .layer(axum::middleware::map_response(
            paron_core::infrastructure::http::error::normalise_error_response,
        ))
        .layer(CatchPanicLayer::custom(panic_handler))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(DefaultOnResponse::new().level(ACCESS_LOG_LEVEL)),
        )
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, UuidV7))
        .layer(MapRequestLayer::new(move |request| {
            strip_untrusted_headers(request, &l7_proxies)
        })))
}

/// Set a hard floor if a request takes too long.
async fn time_limit(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let route = request.extensions().get::<axum::extract::MatchedPath>();
    if route.is_some_and(|route| {
        ROUTES_WITHOUT_MAX_REQUEST_DURATION.contains(&(request.method().clone(), route.as_str()))
    }) {
        return next.run(request).await;
    }

    tokio::time::timeout(MAX_REQUEST_DURATION, next.run(request))
        .await
        .unwrap_or_else(|_| {
            ApiError::detail(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "the request took too long",
            )
            .into_response()
        })
}

#[allow(clippy::needless_pass_by_value)]
fn panic_handler(panic: Box<dyn Any + Send>) -> Response {
    let cause = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned());

    ApiError::internal(cause).into_response()
}

/// Like SmartIpKeyExtractor, but actually safe
#[derive(Clone)]
struct ProxiedIpKeyExtractor(crate::server::run::proxy::L7Proxies);

impl KeyExtractor for ProxiedIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, request: &Request<T>) -> Result<Self::Key, GovernorError> {
        let address = self
            .0
            .client_from_headers(request.headers())
            .or_else(|| request_ip(request))
            .ok_or(GovernorError::UnableToExtractKey)?;

        Ok(crate::server::run::listener::admission::normalize_for_rate_limiting(address))
    }
}

async fn limit_api_auth(
    rate_limit: &GovernorConfig<ProxiedIpKeyExtractor, NoOpMiddleware>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use tower::ServiceExt as _;

    let route = request.extensions().get::<axum::extract::MatchedPath>();
    let is_auth_route = route.is_some_and(|route| {
        ROUTES_WITH_API_AUTH_RATE_LIMIT.contains(&(request.method().clone(), route.as_str()))
    });

    if !is_auth_route {
        return next.run(request).await;
    }

    let Ok(response) = tower_governor::governor::Governor::new(next, rate_limit)
        .oneshot(request)
        .await;

    response
}

fn strip_untrusted_headers(
    mut request: axum::extract::Request,
    l7_proxies: &crate::server::run::proxy::L7Proxies,
) -> axum::extract::Request {
    static TRUSTED_PROXY_HEADERS: LazyLock<Vec<HeaderName>> = LazyLock::new(|| {
        crate::server::run::proxy::PROXY_FORWARDING_HEADERS
            .into_iter()
            .chain([REQUEST_ID_HEADER])
            .collect()
    });

    let ip = request_ip(&request);

    if ip.is_some_and(|address| l7_proxies.contains(address)) {
        return request;
    }

    let mut headers_stripped = false;

    for header in TRUSTED_PROXY_HEADERS.iter() {
        headers_stripped |= request.headers_mut().remove(header).is_some();
    }

    if headers_stripped && let Some(ip) = ip {
        report_headers_stripped(ip);
    }

    request
}

fn report_headers_stripped(peer: IpAddr) {
    static HEADERS_STRIPPED_REPORTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    const MESSAGE: &str =
        "not a trusted proxy, forwarding headers stripped and rate limited as one caller";

    if HEADERS_STRIPPED_REPORTED.swap(true, Relaxed) {
        tracing::debug!(%peer, "{MESSAGE}");
    } else {
        tracing::warn!(%peer, "{MESSAGE}");
    }
}

/// Set by the tcp listener
/// no address = test or forged http request (refused by governor)
fn request_ip<T>(request: &Request<T>) -> Option<IpAddr> {
    request
        .extensions()
        .get::<axum::extract::connect_info::ConnectInfo<SocketAddr>>()
        .map(|connected| connected.0.ip())
}

#[derive(Clone, Copy)]
struct UuidV7;

impl MakeRequestId for UuidV7 {
    #[expect(clippy::expect_used)]
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        let id = uuid::Uuid::now_v7()
            .to_string()
            .parse()
            .expect("a UUIDv7 is a valid request id");

        Some(RequestId::new(id))
    }
}

fn request_span(request: &axum::extract::Request) -> tracing::Span {
    let id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|id| id.to_str().ok())
        .unwrap_or_default();

    tracing::info_span!("request", %id, method = %request.method(), path = %request.uri().path())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::{StatusCode, header};
    use tower::ServiceExt as _;

    use super::*;
    use crate::server::paron::Limits;
    use crate::server::run::proxy::L7Proxies;
    use crate::server::run::tls::self_signed;
    use crate::testing::{TempDataDir, ip_address, parse_each};

    const TRUSTED_PROXY_PEER: SocketAddr =
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 1000));
    const CLIENT_PEER: SocketAddr =
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 20000));

    async fn router_on_fresh_data_dir(name: &str) -> (TempDataDir, Paron, axum::Router) {
        let data_dir = TempDataDir::new(name);

        let contents = crate::server::data_dir::init(data_dir.path()).unwrap();
        let server = Paron::new(
            contents,
            Limits::default(),
            &["https://paron.example.com".parse().unwrap()],
            8443,
        )
        .await
        .unwrap();
        let self_signed =
            self_signed::configure(data_dir.path(), &[], &mut BackgroundTasks::new()).unwrap();

        let router = build(
            &server,
            &L7Proxies::new(&["10.0.0.0/8".parse().unwrap()]),
            &mut BackgroundTasks::new(),
            self_signed,
            #[cfg(debug_assertions)]
            false,
        )
        .unwrap();

        (data_dir, server, router)
    }

    fn extract_client_ip(trusted_networks: &[&str], headers: &[(&str, &str)]) -> IpAddr {
        let request = headers
            .iter()
            .fold(Request::builder(), |request, (name, value)| {
                request.header(*name, *value)
            })
            .extension(ConnectInfo(SocketAddr::from(([198, 51, 100, 9], 40000))))
            .body(())
            .unwrap();

        ProxiedIpKeyExtractor(L7Proxies::new(&parse_each(trusted_networks)))
            .extract(&request)
            .unwrap()
    }

    #[test]
    fn drain_timeout_is_longer_than_the_max_request_duration() {
        assert!(crate::lifecycle::DRAIN_TIMEOUT > MAX_REQUEST_DURATION);
    }

    #[test]
    fn extract_returns_the_nearest_address_no_trusted_proxy_vouched_for() {
        const X_FORWARDED_FOR_WITH_THREE_HOPS: (&str, &str) =
            ("x-forwarded-for", "203.0.113.9, 192.0.2.5, 10.0.0.7");

        type ExtractCase = (
            &'static str,
            &'static [&'static str],
            &'static [(&'static str, &'static str)],
            &'static str,
        );

        let cases: [ExtractCase; 9] = [
            (
                "one trusted hop",
                &["10.0.0.7"],
                &[("x-forwarded-for", "1.2.3.4, 203.0.113.9")],
                "203.0.113.9",
            ),
            (
                "every hop is trusted",
                &["10.0.0.0/8", "192.0.2.0/24"],
                &[X_FORWARDED_FOR_WITH_THREE_HOPS],
                "203.0.113.9",
            ),
            (
                "a hop is not trusted",
                &["10.0.0.0/8"],
                &[X_FORWARDED_FOR_WITH_THREE_HOPS],
                "192.0.2.5",
            ),
            (
                "the chain holds only trusted hops",
                &["10.0.0.0/8"],
                &[("x-forwarded-for", "10.0.0.3, 10.0.0.5, 10.0.0.7")],
                "10.0.0.3",
            ),
            (
                "an entry does not parse",
                &["10.0.0.0/8"],
                &[("x-forwarded-for", "203.0.113.9, unknown, 10.0.0.7")],
                "10.0.0.7",
            ),
            (
                "no entry parses",
                &["10.0.0.0/8"],
                &[("x-forwarded-for", "unknown")],
                "198.51.100.9",
            ),
            (
                "forwarded",
                &["10.0.0.0/8"],
                &[("forwarded", "for=203.0.113.9, for=192.0.2.5, for=10.0.0.7")],
                "192.0.2.5",
            ),
            (
                "x-real-ip",
                &[],
                &[("x-real-ip", "203.0.113.9")],
                "203.0.113.9",
            ),
            ("no forwarding header", &[], &[], "198.51.100.9"),
        ];

        for (case, trusted_networks, headers, client_ip) in cases {
            assert_eq!(
                extract_client_ip(trusted_networks, headers),
                ip_address(client_ip),
                "{case}"
            );
        }
    }

    #[test]
    fn extract_forwarded_ipv6_address_returns_its_rate_limit_block() {
        let ip_from_forwarded =
            extract_client_ip(&[], &[("forwarded", "for=\"[2001:db8:1:2:3:4:5:6]:443\"")]);
        let ip_from_x_forwarded_for =
            extract_client_ip(&[], &[("x-forwarded-for", "2001:db8:1:2:aaaa::1")]);

        assert_eq!(ip_from_forwarded, ip_address("2001:db8:1:2::"));
        assert_eq!(ip_from_x_forwarded_for, ip_address("2001:db8:1:2::"));
    }

    #[tokio::test]
    async fn panic_handler_returns_internal_error_without_the_cause() {
        let response = panic_handler(Box::new("a handler panicked"));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let problem = String::from_utf8_lossy(&body);
        assert!(problem.contains(r#""status":500"#), "{problem}");
        assert!(!problem.contains("a handler panicked"), "{problem}");
    }

    #[tokio::test]
    async fn build_oversized_body_with_content_length_returns_payload_too_large() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("oversized-declared").await;
        let request = Request::builder()
            .method("POST")
            .uri("https://paron.example.com/api/auth/sign-in/verify")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, MAX_REQUEST_BODY_SIZE_BYTES + 1)
            .extension(ConnectInfo(CLIENT_PEER))
            .body(Body::from(vec![b'x'; MAX_REQUEST_BODY_SIZE_BYTES + 1]))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        paron.close().await;
    }

    #[tokio::test]
    async fn build_oversized_body_without_content_length_returns_payload_too_large() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("oversized-streamed").await;
        let request = Request::builder()
            .method("POST")
            .uri("https://paron.example.com/api/auth/sign-in/verify")
            .header(header::COOKIE, "__Host-Http-paron_sign_in=a-token")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(CLIENT_PEER))
            .body(Body::from(vec![b'x'; MAX_REQUEST_BODY_SIZE_BYTES + 1]))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        paron.close().await;
    }

    #[tokio::test]
    async fn build_sign_in_past_the_burst_returns_too_many_requests() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("api-auth-rate-limit").await;

        for attempt in 0..=RATE_LIMIT_API_AUTH_BURST {
            let request = Request::builder()
                .method("POST")
                .uri("https://paron.example.com/api/auth/sign-in/verify")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(CLIENT_PEER))
                .body(Body::from(
                    r#"{"kind":"password","username":"nobody","password":"wrong"}"#,
                ))
                .unwrap();

            let response = router.clone().oneshot(request).await.unwrap();

            let expected = if attempt < RATE_LIMIT_API_AUTH_BURST {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            };
            assert_eq!(response.status(), expected, "attempt {attempt}");
        }

        paron.close().await;
    }

    #[tokio::test]
    async fn build_sets_the_document_headers_on_the_frontend() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("documents-frontend").await;
        let request = Request::builder()
            .uri("https://paron.example.com/")
            .extension(ConnectInfo(TRUSTED_PROXY_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert!(response.headers().contains_key(header::REFERRER_POLICY));
        assert!(
            response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        paron.close().await;
    }

    #[tokio::test]
    async fn build_sets_no_document_headers_on_the_api() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("documents-api").await;
        let request = Request::builder()
            .uri("https://paron.example.com/api/auth/session")
            .extension(ConnectInfo(TRUSTED_PROXY_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert!(!response.headers().contains_key(header::REFERRER_POLICY));
        assert!(
            !response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        paron.close().await;
    }

    #[tokio::test]
    async fn build_serves_the_authority_certificate_inside_the_layers() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("authority").await;
        let request = Request::builder()
            .uri("/paron-ca.crt")
            .extension(ConnectInfo(TRUSTED_PROXY_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/x-x509-ca-cert"
        );
        assert!(
            response.headers().contains_key(REQUEST_ID_HEADER),
            "the route was added after the layers"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let certificate_pem = String::from_utf8_lossy(&body);
        assert!(
            certificate_pem.starts_with("-----BEGIN CERTIFICATE-----"),
            "{certificate_pem}"
        );
        assert!(
            !certificate_pem.contains("PRIVATE KEY"),
            "{certificate_pem}"
        );
        paron.close().await;
    }

    #[tokio::test]
    async fn build_request_id_from_a_trusted_proxy_is_kept() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("request-id-proxy").await;
        let request = Request::builder()
            .uri("https://paron.example.com/api/auth/session")
            .header(REQUEST_ID_HEADER, "set-by-a-proxy")
            .extension(ConnectInfo(TRUSTED_PROXY_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.headers()[REQUEST_ID_HEADER], "set-by-a-proxy");
        paron.close().await;
    }

    #[tokio::test]
    async fn build_request_id_from_a_client_is_replaced() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("request-id-client").await;
        let request = Request::builder()
            .uri("https://paron.example.com/api/auth/session")
            .header(REQUEST_ID_HEADER, "set-by-a-client")
            .extension(ConnectInfo(CLIENT_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_ne!(response.headers()[REQUEST_ID_HEADER], "set-by-a-client");
        paron.close().await;
    }

    // The listener puts the peer address on every request. The rate limiter refuses a request
    // without one.
    #[tokio::test]
    async fn build_request_without_an_address_returns_internal_error() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("address").await;
        let request = Request::builder()
            .uri("https://paron.example.com/")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        paron.close().await;
    }

    #[tokio::test]
    async fn build_api_under_another_host_returns_misdirected_request() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("host-api").await;
        let request = Request::builder()
            .method("POST")
            .uri("/api/auth/sign-in")
            .header(header::HOST, "evil.example")
            .extension(ConnectInfo(CLIENT_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
        paron.close().await;
    }

    #[tokio::test]
    async fn build_health_under_another_host_returns_ok() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("host-health").await;
        let request = Request::builder()
            .uri("/health")
            .header(header::HOST, "10.0.0.5")
            .extension(ConnectInfo(CLIENT_PEER))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        paron.close().await;
    }

    #[tokio::test]
    async fn build_local_host_on_the_listen_or_the_public_port_reaches_the_api() {
        let (_data_dir, paron, router) = router_on_fresh_data_dir("host-local").await;

        for host in [
            "localhost",
            "localhost:8443",
            "127.0.0.1",
            "127.0.0.1:8443",
            "[::1]",
            "[::1]:8443",
        ] {
            let request = Request::builder()
                .uri("/api/auth/session")
                .header(header::HOST, host)
                .extension(ConnectInfo(CLIENT_PEER))
                .body(Body::empty())
                .unwrap();

            let response = router.clone().oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{host}");
        }
        paron.close().await;
    }
}
