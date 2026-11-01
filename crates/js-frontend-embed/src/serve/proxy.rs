use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// Forwards to a running dev server instead of answering from the bundle, so a browser talks only
/// to this binary and gets the same origin behaviour while developing as in a deployment.
///
/// The hot-reload upgrade is forwarded like anything else, as long as nothing tells the bundler's
/// client to connect somewhere absolute — in Vite, `server.hmr.clientPort` and friends.
pub fn proxying(dev: crate::DevServer) -> axum::routing::MethodRouter {
    // The route holds the dev server, because dropping that handle stops the process this
    // forwards to. Both live in one allocation so the handler stays `Clone`, which axum needs.
    let running = std::sync::Arc::new((Proxy::to(dev.origin()), dev));

    axum::routing::any(move |request: Request<Body>| {
        let running = std::sync::Arc::clone(&running);

        async move { running.0.forward(request).await }
    })
}

#[derive(Clone)]
struct Proxy {
    origin: String,
    client: hyper_util::client::legacy::Client<
        hyper_util::client::legacy::connect::HttpConnector,
        Body,
    >,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Proxy")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Proxy {
    fn to(origin: &str) -> Self {
        Self {
            origin: origin.to_owned(),
            client: hyper_util::client::legacy::Client::builder(
                hyper_util::rt::TokioExecutor::new(),
            )
            .build_http(),
        }
    }

    async fn forward(&self, mut request: Request<Body>) -> Response {
        // Claimed before the request is consumed, or the upgraded halves cannot be joined.
        let upgrading = request
            .headers()
            .get(header::UPGRADE)
            .is_some()
            .then(|| hyper::upgrade::on(&mut request));

        let (mut parts, body) = request.into_parts();

        // The origin comes first and carries the authority, so what follows can only be a path on
        // the dev server. The other way round, a path beginning `//` would supply its own.
        let tail = parts.uri.path_and_query().map_or("/", |path| path.as_str());

        let Ok(uri) = format!("{}{tail}", self.origin).parse::<Uri>() else {
            return unreachable(&self.origin);
        };

        parts.uri = uri;
        // The caller may have arrived over HTTP/2; the client below is HTTP/1 only.
        parts.version = axum::http::Version::HTTP_11;
        // The caller's Host names the deployment, and a dev server checks it against the hosts it
        // allows. Removed rather than rewritten: the client fills one in from the URI.
        parts.headers.remove(header::HOST);

        // Not when the request is an upgrade: there `Connection` and `Upgrade` are the handshake
        // rather than a description of the hop, the same exception the 101 gets below.
        if upgrading.is_none() {
            strip_hop_by_hop(&mut parts.headers);
        }

        let Ok(response) = self.client.request(Request::from_parts(parts, body)).await else {
            return unreachable(&self.origin);
        };

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            let mut response = response.map(Body::new);

            // HTTP/2 has no connection headers, so a browser that negotiated it would be within
            // its rights to call the response malformed.
            strip_hop_by_hop(response.headers_mut());

            return response;
        }

        // Past here the answer is the handshake itself, and the headers stripped above are what
        // makes it one, so they go back untouched.
        let Some(downstream) = upgrading else {
            return unreachable(&self.origin);
        };

        let mut answer = Response::new(Body::empty());
        *answer.status_mut() = response.status();
        *answer.headers_mut() = response.headers().clone();

        tokio::spawn(async move {
            let (Ok(mut downstream), Ok(mut upstream)) =
                (downstream.await, hyper::upgrade::on(response).await)
            else {
                return;
            };

            let mut downstream = hyper_util::rt::TokioIo::new(&mut downstream);
            let mut upstream = hyper_util::rt::TokioIo::new(&mut upstream);

            let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
        });

        answer
    }
}

/// Answered when the dev server is not there to answer for itself: usually it stopped, otherwise
/// it is serving TLS, which this plaintext hop cannot read.
fn unreachable(origin: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!(
            "the frontend's dev server at {origin} did not answer.\n\n\
             It has probably stopped — the terminal it was started from will say why. If it is \
             still running, check that it is not serving TLS: this forwards plain HTTP to it, \
             and a bundler told to use HTTPS accepts the connection and then answers nothing \
             that can be read.\n"
        ),
    )
        .into_response()
}

/// Takes off the headers describing the connection a message arrived on rather than the message
/// itself: the browser is on one connection and the dev server on another.
fn strip_hop_by_hop(headers: &mut axum::http::HeaderMap) {
    // `Connection` may name further headers belonging to that connection alone, so what it lists
    // has to come off before it does.
    let listed: Vec<String> = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(|name| name.trim().to_owned())
                .collect()
        })
        .unwrap_or_default();

    for name in listed {
        headers.remove(name.as_str());
    }

    for name in [
        header::CONNECTION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
    ] {
        headers.remove(name);
    }

    // Neither is a constant in `http`. `Proxy-Connection` was never standardised and is still
    // sent.
    for name in ["keep-alive", "proxy-connection"] {
        headers.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    const OK_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi";

    /// Accepts one connection on `listener`, reads the request head, and writes `response`. The
    /// task returns the request head.
    fn answer_once(
        listener: TcpListener,
        response: &'static [u8],
    ) -> tokio::task::JoinHandle<String> {
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = vec![0u8; 2048];
            let bytes_read = stream.read(&mut head).await.unwrap();

            stream.write_all(response).await.unwrap();
            stream.flush().await.unwrap();

            String::from_utf8_lossy(&head[..bytes_read]).into_owned()
        })
    }

    /// A dev server on the IPv4 loopback that answers one request with `response`. Returns its
    /// origin, and a task that returns the request head.
    async fn dev_server_that_answers(
        response: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());

        (origin, answer_once(listener, response))
    }

    // `Dev::spawn` accepts an answer from one address of the name. The proxy dials the name, so
    // its connector must try each address.
    #[tokio::test]
    async fn forward_to_a_name_reaches_a_dev_server_on_either_loopback() {
        let listener = TcpListener::bind("[::1]:0")
            .await
            .expect("the test needs an IPv6 loopback");
        let port = listener.local_addr().unwrap().port();
        let _request_head = answer_once(listener, OK_RESPONSE);

        let response = Proxy::to(&format!("http://localhost:{port}"))
            .forward(Request::get("/").body(Body::empty()).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn forward_keeps_the_path_and_query() {
        let (origin, request_head) = dev_server_that_answers(OK_RESPONSE).await;

        let response = Proxy::to(&origin)
            .forward(Request::get("/users?page=2").body(Body::empty()).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        let head = request_head.await.unwrap();
        assert!(head.starts_with("GET /users?page=2 "), "{head}");
    }

    #[tokio::test]
    async fn forward_keeps_the_authority_whatever_the_path() {
        for path in [
            "//evil.example/x",
            "/x@evil.example",
            "/..%2f..%2fetc/passwd",
            "/a?next=http://evil.example",
            "/%2f%2fevil.example",
        ] {
            let (origin, request_head) = dev_server_that_answers(OK_RESPONSE).await;
            let authority = origin.trim_start_matches("http://").to_owned();

            let response = Proxy::to(&origin)
                .forward(Request::get(path).body(Body::empty()).unwrap())
                .await;

            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let head = request_head.await.unwrap().to_lowercase();
            assert!(
                head.contains(&format!("host: {authority}")),
                "{path}: {head}"
            );
            assert!(!head.contains("evil.example\r\n"), "{path}: {head}");
        }
    }

    #[tokio::test]
    async fn forward_replaces_the_callers_host() {
        let (origin, request_head) = dev_server_that_answers(OK_RESPONSE).await;
        let authority = origin.trim_start_matches("http://").to_owned();

        Proxy::to(&origin)
            .forward(
                Request::get("/")
                    .header(header::HOST, "paron.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

        let head = request_head.await.unwrap().to_lowercase();
        assert!(head.contains(&format!("host: {authority}")), "{head}");
        assert!(!head.contains("paron.test"), "{head}");
    }

    #[tokio::test]
    async fn forward_an_http2_request_speaks_http1_upstream() {
        let (origin, request_head) = dev_server_that_answers(OK_RESPONSE).await;
        let request = Request::get("/")
            .version(axum::http::Version::HTTP_2)
            .body(Body::empty())
            .unwrap();

        let response = Proxy::to(&origin).forward(request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let head = request_head.await.unwrap();
        assert!(head.starts_with("GET / HTTP/1.1\r\n"), "{head}");
    }

    #[tokio::test]
    async fn forward_strips_hop_by_hop_headers_from_the_request() {
        let (origin, request_head) = dev_server_that_answers(OK_RESPONSE).await;
        let mut request = Request::get("/").body(Body::empty()).unwrap();
        for (name, value) in [
            ("connection", "keep-alive, x-of-my-own"),
            ("x-of-my-own", "named by the connection header"),
            ("keep-alive", "timeout=5"),
            ("proxy-connection", "keep-alive"),
            ("te", "trailers"),
            ("trailer", "expires"),
            ("proxy-authenticate", "basic"),
            ("proxy-authorization", "basic abc"),
            ("transfer-encoding", "chunked"),
            ("accept", "text/html"),
        ] {
            request.headers_mut().insert(name, value.parse().unwrap());
        }

        Proxy::to(&origin).forward(request).await;

        let head = request_head.await.unwrap().to_ascii_lowercase();
        let mut header_names: Vec<&str> = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, _)| name.trim())
            .collect();
        header_names.sort_unstable();
        assert_eq!(header_names, ["accept", "host"], "{head}");
    }

    #[tokio::test]
    async fn forward_strips_hop_by_hop_headers_from_the_answer() {
        const RESPONSE_WITH_HOP_BY_HOP_HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\
              connection: keep-alive, x-dev-only\r\nx-dev-only: secret\r\n\
              keep-alive: timeout=5\r\nproxy-connection: keep-alive\r\n\
              transfer-encoding: chunked\r\nupgrade: h2c\r\nte: trailers\r\n\
              trailer: expires\r\nproxy-authenticate: basic\r\n\
              proxy-authorization: basic abc\r\ncontent-type: text/html\r\n\r\nhi";
        let (origin, _request_head) =
            dev_server_that_answers(RESPONSE_WITH_HOP_BY_HOP_HEADERS).await;

        let response = Proxy::to(&origin)
            .forward(Request::get("/").body(Body::empty()).unwrap())
            .await;

        let mut header_names: Vec<&str> = response
            .headers()
            .keys()
            .map(header::HeaderName::as_str)
            .collect();
        header_names.sort_unstable();
        assert_eq!(header_names, ["content-length", "content-type"]);
    }

    #[tokio::test]
    async fn forward_sends_the_request_body_to_the_dev_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let received_request = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request_bytes = Vec::new();
            // The head and the body can arrive in two reads.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);

            while !String::from_utf8_lossy(&request_bytes).contains("wanted-in-the-body") {
                let mut chunk = [0u8; 1024];

                match tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await {
                    Ok(Ok(0) | Err(_)) | Err(_) => break,
                    Ok(Ok(bytes_read)) => request_bytes.extend_from_slice(&chunk[..bytes_read]),
                }
            }

            stream.write_all(OK_RESPONSE).await.unwrap();
            stream.flush().await.unwrap();

            String::from_utf8_lossy(&request_bytes).into_owned()
        });
        let request = Request::post("/refresh")
            .header(header::HOST, "paron.test")
            .body(Body::from("wanted-in-the-body"))
            .unwrap();

        let response = Proxy::to(&origin).forward(request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let request_text = received_request.await.unwrap();
        assert!(
            request_text.ends_with("\r\n\r\nwanted-in-the-body"),
            "{request_text}"
        );
    }

    // The dev server sends the second half of the body only after `forward` returns. A proxy
    // that buffers the body does not return, and the timeout fails the test.
    #[tokio::test]
    async fn forward_returns_the_answer_before_the_body_is_finished() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (send_second_half, second_half_wanted) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = vec![0u8; 2048];
            let _request_length = stream.read(&mut head).await.unwrap();

            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nfirst-")
                .await
                .unwrap();
            stream.flush().await.unwrap();
            second_half_wanted.await.unwrap();
            stream.write_all(b"half").await.unwrap();
            stream.flush().await.unwrap();
        });

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            Proxy::to(&origin).forward(Request::get("/").body(Body::empty()).unwrap()),
        )
        .await
        .expect("the answer should arrive while its body is still open");

        send_second_half.send(()).unwrap();
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(body, "first-half");
    }

    #[tokio::test]
    async fn forward_an_upgrade_keeps_the_handshake_headers() {
        const SWITCHING_PROTOCOLS_RESPONSE: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\n\
              upgrade: websocket\r\n\
              connection: Upgrade\r\nsec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
              sec-websocket-protocol: vite-hmr\r\n\r\n";
        let (origin, request_head) = dev_server_that_answers(SWITCHING_PROTOCOLS_RESPONSE).await;
        let request = Request::get("/")
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_PROTOCOL, "vite-hmr")
            .body(Body::empty())
            .unwrap();

        let response = Proxy::to(&origin).forward(request).await;

        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let head = request_head.await.unwrap().to_ascii_lowercase();
        for line in ["connection: upgrade", "upgrade: websocket"] {
            assert!(head.contains(line), "the handshake did not arrive: {head}");
        }
        for (name, value) in [
            (header::UPGRADE, "websocket"),
            (header::CONNECTION, "Upgrade"),
            (header::SEC_WEBSOCKET_ACCEPT, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            (header::SEC_WEBSOCKET_PROTOCOL, "vite-hmr"),
        ] {
            assert_eq!(
                response
                    .headers()
                    .get(&name)
                    .map(|found| found.to_str().unwrap()),
                Some(value),
                "{name}"
            );
        }
    }

    // An upgrade joins the connection that the request arrived on, so this test needs a real one.
    #[tokio::test]
    async fn forward_after_the_upgrade_carries_bytes_both_ways() {
        const SWITCHING_PROTOCOLS_RESPONSE: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\n\
              upgrade: websocket\r\nconnection: Upgrade\r\n\r\n";
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", upstream.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.unwrap();
            let mut head = vec![0u8; 2048];
            let _request_length = stream.read(&mut head).await.unwrap();
            stream
                .write_all(SWITCHING_PROTOCOLS_RESPONSE)
                .await
                .unwrap();
            stream.flush().await.unwrap();
            let mut echo_buffer = vec![0u8; 64];

            while let Ok(bytes_read) = stream.read(&mut echo_buffer).await {
                if bytes_read == 0 || stream.write_all(&echo_buffer[..bytes_read]).await.is_err() {
                    break;
                }
            }
        });
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = proxy_listener.accept().await.unwrap();
            let proxy = Proxy::to(&origin);
            let forward_service =
                hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                    let proxy = proxy.clone();

                    async move {
                        Ok::<_, std::convert::Infallible>(
                            proxy.forward(request.map(Body::new)).await,
                        )
                    }
                });

            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), forward_service)
                .with_upgrades()
                .await
                .unwrap();
        });
        let mut browser = tokio::net::TcpStream::connect(proxy_address).await.unwrap();
        browser
            .write_all(
                b"GET / HTTP/1.1\r\nHost: paron.test\r\nConnection: Upgrade\r\n\
                  Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                  Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response_bytes = Vec::new();
        while !response_bytes
            .windows(4)
            .any(|window| window == b"\r\n\r\n")
        {
            let mut byte = [0u8; 1];

            match tokio::time::timeout(std::time::Duration::from_secs(5), browser.read(&mut byte))
                .await
            {
                Ok(Ok(1)) => response_bytes.push(byte[0]),
                _ => panic!(
                    "the handshake never finished: {:?}",
                    String::from_utf8_lossy(&response_bytes)
                ),
            }
        }
        let handshake = String::from_utf8_lossy(&response_bytes).into_owned();
        assert!(handshake.starts_with("HTTP/1.1 101"), "{handshake}");

        browser.write_all(b"a change on disk").await.unwrap();

        let mut echoed_bytes = vec![0u8; 16];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            browser.read_exact(&mut echoed_bytes),
        )
        .await
        .expect("nothing came back through the tunnel, so hot reload would never arrive")
        .unwrap();
        assert_eq!(echoed_bytes, b"a change on disk");
    }

    #[tokio::test]
    async fn forward_a_non_http_answer_returns_bad_gateway() {
        const TLS_SERVER_HELLO_START: &[u8] = &[0x16, 0x03, 0x03, 0x00, 0x2a, 0x02];
        let (origin, _request_head) = dev_server_that_answers(TLS_SERVER_HELLO_START).await;

        let response = Proxy::to(&origin)
            .forward(Request::get("/").body(Body::empty()).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn forward_to_a_closed_port_returns_bad_gateway() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);

        let response = Proxy::to(&origin)
            .forward(Request::get("/").body(Body::empty()).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxying_forwards_a_write_method() {
        use tower::ServiceExt as _;

        let lease = crate::testing::lease_port();
        let frontend =
            crate::testing::bun_frontend(&crate::testing::node_dev_server_package_json("dev"));
        let dev_server = crate::Dev::new(frontend.path())
            .port(lease.port())
            .spawn()
            .unwrap();
        let router = proxying(dev_server);

        for method in [
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::PATCH,
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(&method)
                        .uri("/anything")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK, "{method}");
        }
    }
}
