//! The race of RFC §6.3: one connection attempt per mapping and address, in order, each started
//! 250 ms after the one before it or as soon as that one fails to establish.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::time::Instant;

use crate::error::TunnelError;
use crate::identity::Identity;
use crate::transport::{AgentConnection, AgentTransport, PinnedBroker};

/// §6.3's time between the starts of two connection attempts, the default of RFC 8305.
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// The first connection attempt to be ready, past TRN-2 with its control stream open: §6.3's
/// selected attempt. Or the last failure to establish, when every attempt fails (a failed race,
/// which STM-14 backs off from).
///
/// Returning drops every attempt still running, which ends it before it carries a protocol
/// message (§6.3). A dropped attempt that had also completed its handshake is an overtaken
/// connection; the broker reaps it as a connection that was never admitted (TRN-10).
pub async fn race(
    transports: &[Box<dyn AgentTransport>],
    broker: &PinnedBroker,
    identity: &Identity,
) -> Result<AgentConnection, TunnelError> {
    let addresses = alternate_families(broker.address.resolve().await?);
    let mut attempts = Vec::with_capacity(transports.len() * addresses.len());
    for transport in transports {
        for &address in &addresses {
            attempts.push((transport, address));
        }
    }
    let mut waiting = attempts.into_iter().enumerate();
    let mut running = FuturesUnordered::new();
    let mut exhausted = false;
    let mut latest = 0;
    let mut next_start = Instant::now();
    let mut refused = TunnelError::NotConnected;

    loop {
        if !exhausted && Instant::now() >= next_start {
            match waiting.next() {
                Some((index, (transport, address))) => {
                    running.push(async move {
                        (index, transport.dial(broker, address, identity).await)
                    });
                    latest = index;
                    next_start = Instant::now() + ATTEMPT_DELAY;
                }
                None => exhausted = true,
            }
        }

        if exhausted && running.is_empty() {
            return Err(refused);
        }

        tokio::select! {
            Some((index, dialed)) = running.next() => match dialed {
                Ok(connection) => return Ok(connection),
                Err(error) => {
                    refused = error;

                    // Only a failure of the attempt started last brings the next one forward.
                    if index == latest {
                        next_start = Instant::now();
                    }
                }
            },
            () = tokio::time::sleep_until(next_start), if !exhausted => {}
        }
    }
}

/// The resolver's order, with address families taking turns from the family of the first
/// address (RFC 8305 §4).
fn alternate_families(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_is_ipv6 = addresses.first().is_some_and(SocketAddr::is_ipv6);
    let (first_family, other_family): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|address| address.is_ipv6() == first_is_ipv6);
    let mut other_family = other_family.into_iter();

    let mut alternated = Vec::new();
    for address in first_family {
        alternated.push(address);
        alternated.extend(other_family.next());
    }
    alternated.extend(other_family);

    alternated
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::net::{TcpListener, UdpSocket};

    use super::*;
    use crate::identity::InMemoryRollbackIndex;
    use crate::quic::QuicAgentTransport;
    use crate::tcp::TcpAgentTransport;

    #[test]
    fn alternate_families_with_mixed_families_alternates_from_first() {
        let addresses: Vec<SocketAddr> = [
            "[::1]:1",
            "[::2]:1",
            "[::3]:1",
            "127.0.0.1:1",
            "127.0.0.2:1",
        ]
        .map(|address| address.parse().unwrap())
        .into();

        let alternated = alternate_families(addresses);

        let expected: Vec<SocketAddr> = [
            "[::1]:1",
            "127.0.0.1:1",
            "[::2]:1",
            "127.0.0.2:1",
            "[::3]:1",
        ]
        .map(|address| address.parse().unwrap())
        .into();
        assert_eq!(alternated, expected);
    }

    #[test]
    fn alternate_families_with_one_family_keeps_order() {
        let addresses: Vec<SocketAddr> = ["127.0.0.2:1", "127.0.0.1:1"]
            .map(|address| address.parse().unwrap())
            .into();

        let alternated = alternate_families(addresses.clone());

        assert_eq!(alternated, addresses);
    }

    // A UDP socket that reads nothing is a network that drops QUIC without an error, whose
    // attempt would otherwise end only at its 10 second handshake timeout.
    #[tokio::test]
    async fn race_with_silent_first_mapping_starts_next_after_attempt_delay() {
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = tcp.local_addr().unwrap();
        let _silent_udp = UdpSocket::bind(address).await.unwrap();
        let transports: Vec<Box<dyn AgentTransport>> =
            vec![Box::new(QuicAgentTransport), Box::new(TcpAgentTransport)];
        let broker = PinnedBroker {
            address: address.to_string().parse().unwrap(),
            root: Identity::generate().unwrap().public_key().fingerprint(),
            rollback: Arc::new(InMemoryRollbackIndex::default()),
        };
        let started = Instant::now();

        let racing = tokio::spawn(async move {
            race(&transports, &broker, &Identity::generate().unwrap()).await
        });
        let _socket = tcp.accept().await.unwrap();
        let tcp_started = started.elapsed();
        racing.abort();

        assert!(
            (ATTEMPT_DELAY..Duration::from_secs(2)).contains(&tcp_started),
            "the TCP attempt started after {tcp_started:?}"
        );
    }
}
