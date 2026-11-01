//! Bytes really do flow both ways, over every kind of selector PTP/1 defines (§2.3).
//!
//! §11 checks each selector type reaches its service, but every one of those tests speaks first
//! and then waits for an answer. A relay that only pumped the far side after seeing traffic from
//! the near side would pass all of them and still be broken — an SSH server sends its version
//! banner before the client says a word, and that connection would simply hang.
//!
//! So the services here talk first where the protocol allows it, and the bulk test pushes in both
//! directions at once to catch a relay that can only manage one direction at a time.

use std::net::Ipv4Addr;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::Suite;
use crate::{Selector, Selectors};

const GREETING: &[u8] = b"220 service ready\r\n";

/// Long enough to need several round trips through both mappings' flow control, so a relay that
/// only drains one direction at a time deadlocks instead of passing.
const BULK: usize = 512 * 1024;

/// Nothing here should take anywhere near this. It is here so a deadlocked relay fails the test
/// instead of hanging the suite.
const BEFORE_STALLED: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread")]
async fn relay_tcp_port_over_quic_carries_both_directions() {
    must_not_stall(both_directions(Suite::quic(), Service::Tcp)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_tcp_port_over_tcp_carries_both_directions() {
    must_not_stall(both_directions(Suite::tcp(), Service::Tcp)).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn relay_unix_path_over_quic_carries_both_directions() {
    must_not_stall(both_directions(Suite::quic(), Service::Unix)).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn relay_unix_path_over_tcp_carries_both_directions() {
    must_not_stall(both_directions(Suite::tcp(), Service::Unix)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_udp_port_over_quic_carries_both_directions() {
    must_not_stall(datagrams_both_directions(Suite::quic())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_udp_port_over_tcp_carries_both_directions() {
    must_not_stall(datagrams_both_directions(Suite::tcp())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_tcp_port_over_quic_carries_bulk_both_directions_at_once() {
    must_not_stall(bulk_both_directions(Suite::quic())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_tcp_port_over_tcp_carries_bulk_both_directions_at_once() {
    must_not_stall(bulk_both_directions(Suite::tcp())).await;
}

async fn must_not_stall(relaying: impl Future<Output = ()>) {
    tokio::time::timeout(BEFORE_STALLED, relaying)
        .await
        .expect("the relay stalled: one direction is not being carried");
}

/// Which kind of byte-stream service to put behind the tunnel. Both are reached the same way once
/// the stream is open, which is the point — the selector type decides what the agent connects to
/// and nothing after that.
#[derive(Clone, Copy)]
enum Service {
    Tcp,
    #[cfg(unix)]
    Unix,
}

/// The far end speaks first, then the two sides take turns.
async fn both_directions(suite: Suite, kind: Service) {
    let selector = match kind {
        Service::Tcp => Selector::tcp(greeter_on_tcp().await).expect("a bound port"),
        #[cfg(unix)]
        Service::Unix => Selector::unix(greeter_on_unix()).expect("a socket path"),
    };

    let joined = suite
        .joined(Arc::new(Selectors::new([selector.clone().into()])))
        .await;
    let mut stream = joined
        .connected_agents()
        .open(joined.id, &selector)
        .await
        .expect("the agent serves this selector");

    // Nothing has been sent yet: whatever arrives now was the service's own idea. A relay that
    // waits for the consumer to speak before pumping the other direction hangs here.
    let mut greeting = vec![0u8; GREETING.len()];
    stream
        .read_exact(&mut greeting)
        .await
        .expect("the service's greeting must arrive unprompted");
    assert_eq!(greeting, GREETING, "the greeting must arrive intact");

    // And now the other direction, twice, so a relay that manages one exchange and then stops is
    // caught as well.
    for message in [b"first".as_slice(), b"second".as_slice()] {
        stream.write_all(message).await.expect("the request goes");

        let mut echoed = vec![0u8; message.len()];
        stream
            .read_exact(&mut echoed)
            .await
            .expect("and the answer comes back");

        assert_eq!(echoed, message, "the echo must match what was sent");
    }
}

/// A `udp-port` service cannot speak first — it has no address to answer until someone sends it
/// something — so the reverse direction is tested by having it answer once with several datagrams
/// of different lengths, including an empty one (§2.5).
async fn datagrams_both_directions(suite: Suite) {
    let selector = Selector::udp(talkative_on_udp().await).expect("a bound port");
    let joined = suite
        .joined(Arc::new(Selectors::new([selector.clone().into()])))
        .await;
    let mut stream = joined
        .connected_agents()
        .open(joined.id, &selector)
        .await
        .expect("the agent serves this selector");

    // One record out (§2.5: a 2-octet length, then that many octets).
    let question = b"ask";
    stream
        .write_all(&u16::try_from(question.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(question).await.unwrap();

    // Three back, boundaries intact and in order.
    for expected in [b"one".as_slice(), b"".as_slice(), b"three!".as_slice()] {
        let mut length = [0u8; 2];
        stream
            .read_exact(&mut length)
            .await
            .expect("every answering datagram must arrive");

        let mut datagram = vec![0u8; usize::from(u16::from_be_bytes(length))];
        stream.read_exact(&mut datagram).await.unwrap();

        assert_eq!(
            datagram, expected,
            "each datagram must arrive whole and on its own"
        );
    }
}

/// Half a megabyte each way at the same time.
///
/// Neither side reads until it has written everything, so if the relay only drains one direction
/// the other's buffers fill and both sides block for good.
async fn bulk_both_directions(suite: Suite) {
    let selector = Selector::tcp(greeter_on_tcp().await).expect("a bound port");
    let joined = suite
        .joined(Arc::new(Selectors::new([selector.clone().into()])))
        .await;
    let stream = joined
        .connected_agents()
        .open(joined.id, &selector)
        .await
        .expect("the agent serves this selector");

    let (mut from_service, mut to_service) = tokio::io::split(stream);

    // The greeting comes first; the echo of the bulk follows it.
    let mut greeting = vec![0u8; GREETING.len()];
    from_service.read_exact(&mut greeting).await.unwrap();

    let sending = tokio::spawn(async move {
        to_service.write_all(&vec![0xA5; BULK]).await.unwrap();
        to_service.flush().await.unwrap();
    });

    let mut echoed = vec![0u8; BULK];
    from_service
        .read_exact(&mut echoed)
        .await
        .expect("the whole payload must come back while it is still being sent");

    sending.await.unwrap();

    assert!(
        echoed.iter().all(|byte| *byte == 0xA5),
        "the payload must come back unaltered"
    );
}

/// A TCP service that greets every connection before it is spoken to, then echoes.
async fn greeter_on_tcp() -> u16 {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        while let Ok((mut connection, _)) = listener.accept().await {
            tokio::spawn(async move {
                if connection.write_all(GREETING).await.is_err() {
                    return;
                }

                let (mut read, mut write) = connection.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });

    port
}

/// The same, over a UNIX-domain socket.
#[cfg(unix)]
fn greeter_on_unix() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ptp-relay-{}-{}.sock",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();

    tokio::spawn(async move {
        while let Ok((mut connection, _)) = listener.accept().await {
            tokio::spawn(async move {
                if connection.write_all(GREETING).await.is_err() {
                    return;
                }

                let (mut read, mut write) = connection.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });

    path
}

/// A UDP service that answers one datagram with three, one of them empty.
async fn talkative_on_udp() -> u16 {
    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();

    tokio::spawn(async move {
        let mut datagram = vec![0u8; usize::from(u16::MAX)];

        while let Ok((_, from)) = socket.recv_from(&mut datagram).await {
            for answer in [b"one".as_slice(), b"".as_slice(), b"three!".as_slice()] {
                if socket.send_to(answer, from).await.is_err() {
                    return;
                }
            }
        }
    });

    port
}
