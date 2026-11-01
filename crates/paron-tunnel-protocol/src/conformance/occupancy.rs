//! What one source may hold of the unadmitted-connection bound, driven through a transport
//! (TRN-10).
//!
//! Not a member of §11 — the conformance table has no requirement for a limit — but the bound is
//! the only thing standing between an unproven peer and every agent in the fleet, and the unit
//! tests beside [`UnadmittedConnections`](crate::broker::UnadmittedConnections) cannot see the
//! stage a flood actually arrives at. A peer that completes the handshake and opens nothing is
//! holding a place, and these check that it is counted as one.

use std::future::IntoFuture as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::task::AbortOnDropHandle;

use super::records::Records;
use crate::quic::{QuicBrokerTransport, broker_streams, transport_config};
use crate::tcp::TcpBrokerTransport;
use crate::{
    Broker, BrokerIdentity, BrokerPolicy, Fingerprint, Identity, InMemoryRollbackIndex, Limits,
    Liveness, tls,
};

/// Eight, so a quarter of it is two: few enough that the share is reached in two connections and
/// the random refusal past half capacity never comes into it.
const CAPACITY: usize = 8;
const PER_SOURCE: usize = CAPACITY / 4;

#[derive(Clone, Copy)]
enum Mapping {
    Quic,
    Tcp,
}

#[tokio::test(flavor = "multi_thread")]
async fn handshakes_from_one_source_over_quic_stop_at_its_share() {
    one_source_stops_at_its_share(Mapping::Quic).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn handshakes_from_one_source_over_tcp_stop_at_its_share() {
    one_source_stops_at_its_share(Mapping::Tcp).await;
}

/// A peer that completes handshakes and opens no stream takes its share and no more.
///
/// The regression this exists for: the pre-admission counter used to be reached only once the
/// control stream had arrived, so a peer that stopped just short of that was in no counter at
/// all and one address could fill the whole bound.
async fn one_source_stops_at_its_share(mapping: Mapping) {
    let (broker, address, _accepting) = broker(mapping).await;
    let key = broker.identity().fingerprint();

    // Held: dropping one gives its place back, which is not what is under test here.
    let mut holding = Vec::new();

    for taken in 0..PER_SOURCE {
        holding.push(
            handshake_only(mapping, address, &key)
                .await
                .unwrap_or_else(|error| {
                    panic!("connection {taken} is within this source's share: {error}")
                }),
        );
    }

    let past_the_share = handshake_only(mapping, address, &key).await;

    assert!(
        past_the_share.is_err(),
        "a source holding its whole share of {PER_SOURCE} was given another place"
    );
}

/// One connection, handshake completed and no stream opened — which is what the bound exists to
/// count and what [`AgentTransport::dial`](crate::AgentTransport::dial) cannot produce, because
/// it opens the control stream as part of connecting.
///
/// The returned value is the connection: holding it holds the place.
async fn handshake_only(
    mapping: Mapping,
    address: SocketAddr,
    broker_key: &Fingerprint,
) -> Result<Box<dyn Send>, crate::TunnelError> {
    let identity = Identity::generate().expect("an agent identity");

    match mapping {
        Mapping::Quic => {
            let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls::agent_config(
                &identity,
                broker_key,
                Arc::new(InMemoryRollbackIndex::default()),
            )?)
            .map_err(crate::TunnelError::unexpected)?;
            let mut config = quinn::ClientConfig::new(Arc::new(crypto));
            config.transport_config(transport_config(broker_streams(), None));

            let mut endpoint = quinn::Endpoint::client((Ipv4Addr::LOCALHOST, 0).into())
                .map_err(crate::TunnelError::unexpected)?;
            endpoint.set_default_client_config(config);

            let host = address.ip().to_string();
            let connection = endpoint
                .connect(address, &host)
                .map_err(crate::TunnelError::unexpected)?
                .await
                .map_err(crate::TunnelError::unexpected)?;

            // The endpoint goes with it: dropping it would close the connection under us.
            Ok(Box::new((endpoint, connection)))
        }
        Mapping::Tcp => {
            let tcp = tokio::net::TcpStream::connect(address)
                .await
                .map_err(crate::TunnelError::unexpected)?;

            let connector = tokio_rustls::TlsConnector::from(tls::agent_config(
                &identity,
                broker_key,
                Arc::new(InMemoryRollbackIndex::default()),
            )?);
            let name = tls::server_name(&address.ip().to_string())?;

            let tls = connector
                .connect(name, tcp)
                .await
                .map_err(crate::TunnelError::unexpected)?;

            Ok(Box::new(tls))
        }
    }
}

/// A broker bound on loopback with a bound small enough to reach, accepting.
async fn broker(mapping: Mapping) -> (Arc<Broker>, SocketAddr, AbortOnDropHandle<()>) {
    let records = Records::new();
    let broker = Arc::new(Broker::new(
        Arc::clone(&records) as Arc<dyn BrokerPolicy>,
        Arc::new(crate::ConnectedAgents::new()),
        BrokerIdentity::generate().expect("a broker identity"),
        Liveness::default(),
        Limits {
            max_unadmitted: CAPACITY,
            // Long, so nothing under test is reached by a connection timing out instead.
            admission_timeout: Duration::from_secs(30),
        },
    ));

    let serving = match mapping {
        Mapping::Quic => {
            broker
                .bind(&QuicBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
                .await
        }
        Mapping::Tcp => {
            broker
                .bind(&TcpBrokerTransport, (Ipv4Addr::LOCALHOST, 0).into())
                .await
        }
    }
    .expect("the broker must bind");

    let address = serving.address();
    let accepting = AbortOnDropHandle::new(tokio::spawn(serving.into_future()));

    (broker, address, accepting)
}
