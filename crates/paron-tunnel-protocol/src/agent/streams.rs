//! The agent's half of RFC §4.3: every broker-opened tunnel stream, answered by POL-8 and
//! relayed to the service it names.

use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

use crate::policy::ServedSelectors;
use crate::transport::{IncomingStreams, Stream};
use crate::wire::{self, Selector, StreamAnswer};

/// TRN-6: how long the agent gives its own connection to the target POL-8 returned.
const TARGET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Serve every broker-opened tunnel stream for as long as the connection lasts.
///
/// Every stream is answered by POL-8 in every state (STM-19): `answering` is published before the
/// connection is dialled, so a stream arriving before the `ACCESS`(Granted) that admitted the
/// connection is answered like any other. A connection that publishes nothing — a joining one —
/// refuses every stream.
pub(crate) async fn serve_streams(
    mut incoming: Box<dyn IncomingStreams>,
    answering: tokio::sync::watch::Receiver<Option<Arc<dyn ServedSelectors>>>,
) {
    while let Some(stream) = incoming.next().await {
        let answering = answering.clone();

        // Streams are concurrent, and relaying one must not block another (§4.3, TRN-6).
        tokio::spawn(async move {
            serve_one_stream(stream, &answering).await;
        });
    }
}

async fn serve_one_stream(
    mut stream: Box<dyn Stream>,
    answering: &tokio::sync::watch::Receiver<Option<Arc<dyn ServedSelectors>>>,
) {
    // A preamble that does not decode is a protocol error of the stream alone (§2.5).
    let Ok(selector) = wire::read_stream_preamble(&mut stream).await else {
        return;
    };

    // POL-8 is the only thing that answers a stream, whatever was announced (§3.1, §4.3).
    let served = answering
        .borrow()
        .as_ref()
        .is_some_and(|serves| serves.serves(&selector));

    if !served {
        // A consumer can name any selector, announced or not (§3.1). This occurs in normal
        // use, and the broker controls how often. Debug level keeps it out of the usual log.
        tracing::debug!(%selector, "refused a request for a service this agent is not allowed to reach");

        refuse(stream).await;

        return;
    }

    // Where a served selector terminates is not a choice: PTP/1 names one host, the agent's own
    // (§2.5), so the address is the selector read on loopback. SEC-5 holds because nothing here
    // can name another host.
    match &selector {
        Selector::Tcp(port) => {
            relay_bytes(&selector, stream, connect_loopback_tcp(port.get())).await;
        }
        Selector::Unix(path) => {
            #[cfg(unix)]
            {
                let path = path.path();

                relay_bytes(&selector, stream, tokio::net::UnixStream::connect(path)).await;
            }
            #[cfg(not(unix))]
            {
                let _ = path;

                refuse(stream).await;
            }
        }
        Selector::Udp(port) => {
            relay_datagrams(&selector, stream, connect_loopback_udp(port.get())).await;
        }
    }
}

/// A byte-stream service — `tcp-port` or `unix-path` — carried unframed in both directions (§2.5).
///
/// STM-20: `STREAM_ACCEPT` goes out only once the service is connected. Everything else is
/// `STREAM_REFUSE`, and a service that cannot be reached is not distinguished from a selector
/// POL-8 declined (SEC-7).
///
/// SEC-7 controls what the agent sends to the broker. It does not control this log. The agent
/// writes the reason here, where only the operator of this machine reads it.
async fn relay_bytes<S>(
    selector: &Selector,
    mut stream: Box<dyn Stream>,
    connecting: impl Future<Output = std::io::Result<S>>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut service = match tokio::time::timeout(TARGET_CONNECT_TIMEOUT, connecting).await {
        Ok(Ok(service)) => service,
        Ok(Err(reason)) => {
            tracing::warn!(%selector, %reason, "could not connect to the service the server asked for");

            refuse(stream).await;

            return;
        }
        Err(_elapsed) => {
            tracing::warn!(
                %selector,
                waited_seconds = TARGET_CONNECT_TIMEOUT.as_secs(),
                "the service the server asked for did not accept a connection in time"
            );

            refuse(stream).await;

            return;
        }
    };

    if accept(&mut *stream).await {
        let _ = tokio::io::copy_bidirectional(&mut stream, &mut service).await;
    }
}

/// A `udp-port` service, one length-prefixed record per datagram in both directions, until either
/// side goes (§2.5).
///
/// Buffering is one datagram per direction: a datagram arriving while the stream has no room waits
/// in the socket, and the kernel discards beyond that, which is UDP.
async fn relay_datagrams(
    selector: &Selector,
    mut stream: Box<dyn Stream>,
    connecting: impl Future<Output = std::io::Result<tokio::net::UdpSocket>>,
) {
    let socket = match connecting.await {
        Ok(socket) => socket,
        Err(reason) => {
            tracing::warn!(%selector, %reason, "could not connect to the service the server asked for");

            refuse(stream).await;

            return;
        }
    };

    if !accept(&mut *stream).await {
        return;
    }

    let (mut from_broker, mut to_broker) = tokio::io::split(stream);
    let mut record = Vec::new();
    let mut datagram = vec![0u8; usize::from(u16::MAX)];

    // A direction at a time, rather than two arms of one `select!` over a record and a datagram:
    // reading a record is two reads, its length and then its octets, so an arm dropped between
    // them would resume mid-record and desynchronise the stream for good. Here a direction is
    // dropped only once the other has ended and the relay is going anyway.
    tokio::select! {
        () = records_to_service(&mut from_broker, &socket, &mut record) => {}
        () = datagrams_to_broker(&socket, &mut to_broker, &mut datagram) => {}
    }

    let _ = to_broker.shutdown().await;
}

/// Records off the stream, each one sent as a datagram.
async fn records_to_service(
    from_broker: &mut (impl AsyncRead + Unpin),
    socket: &tokio::net::UdpSocket,
    record: &mut Vec<u8>,
) {
    while wire::read_record(from_broker, record).await.is_ok() {
        if socket.send(record).await.is_err() {
            return;
        }
    }
}

/// Datagrams off the socket, each one written as a record.
async fn datagrams_to_broker(
    socket: &tokio::net::UdpSocket,
    to_broker: &mut (impl AsyncWrite + Unpin),
    datagram: &mut [u8],
) {
    while let Ok(length) = socket.recv(datagram).await {
        if wire::write_record(to_broker, &datagram[..length])
            .await
            .is_err()
        {
            return;
        }
    }
}

/// `STREAM_ACCEPT`, and whether it reached the broker (§2.5).
async fn accept(mut stream: &mut dyn Stream) -> bool {
    wire::write_stream_answer(&mut stream, StreamAnswer::Accept)
        .await
        .is_ok()
}

/// `STREAM_REFUSE`, after which each side closes the stream (§2.5).
async fn refuse(mut stream: Box<dyn Stream>) {
    let _ = wire::write_stream_answer(&mut stream, StreamAnswer::Refuse).await;
}

/// A port selector names a port and no address (§2.3), so both loopbacks are the same service to
/// it: which one a host's service bound is not the consumer's to know. IPv4 first, being the
/// common bind.
async fn connect_loopback_tcp(port: u16) -> std::io::Result<tokio::net::TcpStream> {
    match tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
        Ok(service) => Ok(service),
        Err(over_v4) => tokio::net::TcpStream::connect((Ipv6Addr::LOCALHOST, port))
            .await
            .map_err(|_| over_v4),
    }
}

/// IPv4 loopback only, unlike TCP: connecting a datagram socket sends nothing, so it cannot fail
/// on a port nothing answers and there is no failure to fall back from. A `udp-port` service
/// reachable over this tunnel therefore listens on 127.0.0.1.
///
/// The socket is bound on loopback as well as connected to it. Only loopback is ever on the other
/// end, and SEC-2 has the agent binding nothing anything else can reach: an ephemeral port on
/// every interface would be reachable from off the host, even if the kernel then dropped what
/// arrived there.
async fn connect_loopback_udp(port: u16) -> std::io::Result<tokio::net::UdpSocket> {
    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    socket.connect((Ipv4Addr::LOCALHOST, port)).await?;

    Ok(socket)
}
