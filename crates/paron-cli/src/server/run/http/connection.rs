use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;

const HTTP_MAX_BUFFERED_BYTES: usize = 64 * 1024;
const HTTP2_MAX_STREAMS_PER_CONNECTION: u32 = 32;
const HTTP1_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECTION_MAX_AGE: Duration = Duration::from_hours(1);

pub async fn serve(
    router: &axum::Router,
    tls_handshake: tokio_rustls::server::TlsStream<TcpStream>,
    peer: SocketAddr,
    stop_signal: &tokio_util::sync::CancellationToken,
) {
    let service = tower::ServiceBuilder::new()
        .map_request(move |mut request: hyper::Request<hyper::body::Incoming>| {
            request
                .extensions_mut()
                .insert(axum::extract::connect_info::ConnectInfo(peer));

            request
        })
        .service(router.clone().into_service::<hyper::body::Incoming>());

    let service = hyper_util::service::TowerToHyperService::new(service);

    let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());

    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HTTP1_HEADER_TIMEOUT)
        .max_buf_size(HTTP_MAX_BUFFERED_BYTES);

    builder
        .http2()
        .max_concurrent_streams(HTTP2_MAX_STREAMS_PER_CONNECTION)
        .max_send_buf_size(HTTP_MAX_BUFFERED_BYTES);

    let connection = builder.serve_connection_with_upgrades(
        hyper_util::rt::TokioIo::new(super::deadline::Deadline::new(
            tls_handshake,
            CONNECTION_IDLE_TIMEOUT,
        )),
        service,
    );

    let mut connection = std::pin::pin!(connection.into_owned());
    let max_age = tokio::time::sleep(CONNECTION_MAX_AGE);
    tokio::pin!(max_age);

    let _served = tokio::select! {
        served = connection.as_mut() => served,
        () = stop_signal.cancelled() => {
            connection.as_mut().graceful_shutdown();

            connection.as_mut().await
        }
        () = &mut max_age => {
            connection.as_mut().graceful_shutdown();

            connection.as_mut().await
        }
    };
}
