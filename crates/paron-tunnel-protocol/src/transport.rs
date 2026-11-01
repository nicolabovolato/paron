//! What a transport mapping owes the protocol (RFC §7, TRN-1 to TRN-12).
//!
//! Both mappings realise the identical protocol: the frames of §2 are encoded by [`crate::wire`]
//! and driven by the two state machines, so a mapping supplies streams and TLS identity proof and
//! nothing else. It decides nothing (STM-9).

use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::sync::Arc;

use async_trait::async_trait;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::broker::Broker;
use crate::error::TunnelError;
use crate::identity::{Fingerprint, Identity, PublicKey, RollbackIndex};

/// Where an agent dials its broker: a DNS name or an IP literal, an IPv6 literal bracketed as
/// `[::1]:8443`, and a port.
///
/// Parsed where an operator configures it rather than on every dial, so a malformed address
/// costs a startup failure and not a tunnel that dies on each attempt. The host is validated as
/// the TLS server name a mapping will present it as, so no mapping re-checks it.
#[derive(Debug, Clone)]
pub struct BrokerAddress {
    host: String,
    port: NonZeroU16,
    server_name: ServerName<'static>,
}

impl BrokerAddress {
    /// The host, as TLS names it: no brackets, whatever the address was written with.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port.get()
    }

    pub(crate) fn server_name(&self) -> ServerName<'static> {
        self.server_name.clone()
    }

    /// Every address this broker resolves to, in the order the resolver gave them (TRN-1).
    ///
    /// Resolved per race, so an agent follows a broker that moves. All of them rather than the
    /// first: a dual-stack host commonly resolves to an AAAA an agent cannot reach and an A it
    /// can, and §6.3 races one connection attempt per mapping and address.
    pub async fn resolve(&self) -> Result<Vec<SocketAddr>, TunnelError> {
        let addresses: Vec<SocketAddr> = tokio::net::lookup_host((self.host(), self.port()))
            .await
            .map_err(TunnelError::unexpected)?
            .collect();

        if addresses.is_empty() {
            return Err(TunnelError::fault("the broker host resolves to no address"));
        }

        Ok(addresses)
    }
}

impl std::str::FromStr for BrokerAddress {
    type Err = TunnelError;

    fn from_str(address: &str) -> Result<Self, Self::Err> {
        let (host, port) = split_host_port(address)?;

        Ok(Self {
            port: port
                .parse::<u16>()
                .ok()
                .and_then(NonZeroU16::new)
                .ok_or_else(|| TunnelError::fault("a broker port is 1 to 65535"))?,
            // Rejects an empty or unusable host, which is the whole of what TLS asks of it.
            server_name: crate::tls::server_name(host)?,
            host: host.to_owned(),
        })
    }
}

impl std::fmt::Display for BrokerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

/// The host and port of `host:port`, with an IPv6 literal bracketed. Unbracketed, an IPv6
/// literal cannot be told from a host and a port, so it is refused rather than guessed at.
fn split_host_port(address: &str) -> Result<(&str, &str), TunnelError> {
    if let Some(rest) = address.strip_prefix('[') {
        let (host, rest) = rest
            .split_once(']')
            .ok_or_else(|| TunnelError::fault("a bracketed broker host closes its bracket"))?;

        return rest
            .strip_prefix(':')
            .map(|port| (host, port))
            .ok_or_else(|| TunnelError::fault("a broker address is host:port"));
    }

    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| TunnelError::fault("a broker address is host:port"))?;

    if host.contains(':') {
        return Err(TunnelError::fault(
            "an IPv6 broker address is written [::1]:8443",
        ));
    }

    Ok((host, port))
}

/// A byte pipe multiplexed over a connection: the control stream, or a tunnel stream. A trait
/// alias, a stream being a different thing per mapping.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

/// How many tunnel streams one connection may carry at once.
///
/// TRN-6 and TRN-9 require a bound but do not say what it should be, so this is our choice, and
/// both mappings use it. 256 concurrent forwards to a single machine is far more than an operator
/// is likely to want and small enough that a connection cannot cost the broker much.
///
/// The two mappings account for it differently, which is worth knowing if you change it:
///
/// - QUIC counts each direction separately, so the agent grants the broker exactly this many and
///   the control stream — which the agent opens — is not counted against it.
/// - yamux has one budget for the whole session in both directions, so its limit is this plus one,
///   the extra being the control stream.
///
/// Either way a connection carries this many tunnel streams and no more.
pub(crate) const TUNNEL_STREAMS: usize = 256;

/// A tunnel stream holding its place in a mapping's per-connection ceiling until it drops.
///
/// Both mappings bound the streams one connection carries (TRN-6, TRN-9) and answer an open past
/// the bound as a refusal rather than waiting for a place, which §6.1 requires: a consumer
/// waiting on a place is a consumer waiting on another consumer to close.
pub(crate) struct Leased<S> {
    stream: S,
    _place: tokio::sync::OwnedSemaphorePermit,
}

impl<S> Leased<S> {
    pub(crate) fn new(stream: S, place: tokio::sync::OwnedSemaphorePermit) -> Self {
        Self {
            stream,
            _place: place,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Leased<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Leased<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// The broker an agent belongs to: where to dial, whose key to accept there, and the rollback
/// index it has accepted under that key (POL-7, TRN-2). The address and the key arrive by the
/// same out-of-band handover as the join token; the index is the agent's own record.
pub struct PinnedBroker {
    pub address: BrokerAddress,
    pub root: Fingerprint,
    pub rollback: Arc<dyn RollbackIndex>,
}

/// The agent half of a mapping: one connection attempt (TRN-1, §6.3).
#[async_trait]
pub trait AgentTransport: Send + Sync {
    /// Establish a connection to `broker` at `address`, one of the addresses its name resolves
    /// to, refusing any peer whose certificate was not signed by the key it pins (TRN-2), and
    /// proving possession of `identity`'s private key in the handshake (TRN-3). Nothing of the
    /// protocol is transmitted before the broker's key is verified.
    async fn dial(
        &self,
        broker: &PinnedBroker,
        address: SocketAddr,
        identity: &Identity,
    ) -> Result<AgentConnection, TunnelError>;
}

/// One established connection, from the agent's side, before any protocol has been spoken.
///
/// The parts are public because the agent moves each of them out separately and holds no
/// invariant over the set; construction goes through [`new`](AgentConnection::new) so that a
/// mapping outside this crate is not broken by a part being added.
#[non_exhaustive]
pub struct AgentConnection {
    /// The control stream, opened by the agent (§2.4). The protocol driver writes the version
    /// octet and every frame here.
    pub control: Box<dyn Stream>,
    /// Broker-opened tunnel streams, as they arrive. `None` when the connection has ended.
    pub incoming: Box<dyn IncomingStreams>,
    /// Ends the connection when dropped or invoked (STM-5).
    pub closer: Box<dyn Closer>,
}

impl AgentConnection {
    pub fn new(
        control: Box<dyn Stream>,
        incoming: Box<dyn IncomingStreams>,
        closer: Box<dyn Closer>,
    ) -> Self {
        Self {
            control,
            incoming,
            closer,
        }
    }
}

/// The agent's accept side: broker-opened tunnel streams, in arrival order (§4.3, STM-19).
///
/// A trait rather than a channel or a `futures::Stream` because each mapping already has this
/// loop and only needs it exposed — `accept_bi` on a QUIC connection, the inbound half of a yamux
/// session — so an adapter task and its buffering would be pure overhead.
#[async_trait]
pub trait IncomingStreams: Send {
    /// The next stream the broker opened, or `None` once the connection has ended.
    async fn next(&mut self) -> Option<Box<dyn Stream>>;
}

/// Ends the whole connection, not one stream.
///
/// Separate from the control stream for two reasons. Shutting a stream is not ending a
/// connection: closing the control stream leaves a QUIC connection and its tunnel streams alive,
/// and closing a yamux substream leaves the session running. And the two have different owners:
/// the driver owns the control stream exclusively and splits it to read and write, while a
/// replacement ends a displaced connection from another task entirely (STM-11).
#[async_trait]
pub trait Closer: Send + Sync {
    async fn close(&self);
}

/// The broker half of a mapping: listen, and hand every established connection to the
/// per-connection driver.
#[async_trait]
pub trait BrokerTransport: Send + Sync {
    /// Bind `address`. Nothing is accepted until what comes back is awaited (TRN-1). Every
    /// accepted connection is driven through the broker state machine of §4.2 against `broker`.
    async fn bind(&self, address: SocketAddr, broker: Arc<Broker>) -> Result<Serving, TunnelError>;
}

/// One accepted connection, from the broker's side, before any protocol has been spoken.
///
/// Public parts and a constructor, for the reasons under [`AgentConnection`].
#[non_exhaustive]
pub struct BrokerConnection {
    /// The address the agent dialled from. TRN-10 counts unadmitted connections per source, and
    /// this is the source.
    pub agent_address: SocketAddr,
    /// The key the peer proved possession of in the handshake (TRN-3). Its significance is
    /// POL-1's to judge.
    pub agent_public_key: PublicKey,
    /// The control stream the agent opened.
    pub control: Box<dyn Stream>,
    /// Opens a tunnel stream toward the agent (TRN-6).
    pub opener: Arc<dyn StreamOpener>,
    /// Ends the connection (STM-5, STM-11).
    pub closer: Arc<dyn Closer>,
}

impl BrokerConnection {
    pub fn new(
        agent_address: SocketAddr,
        agent_public_key: PublicKey,
        control: Box<dyn Stream>,
        opener: Arc<dyn StreamOpener>,
        closer: Arc<dyn Closer>,
    ) -> Self {
        Self {
            agent_address,
            agent_public_key,
            control,
            opener,
            closer,
        }
    }
}

#[async_trait]
pub trait StreamOpener: Send + Sync {
    async fn open(&self) -> Result<Box<dyn Stream>, TunnelError>;
}

/// A broker bound to an address, not yet accepting. Awaiting it accepts until it is dropped;
/// [`Serving::with_graceful_shutdown`] accepts until a signal and then closes what it opened.
pub struct Serving {
    address: SocketAddr,
    stopping: tokio_util::sync::CancellationToken,
    accepting: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
}

impl Serving {
    pub fn new(
        address: SocketAddr,
        stopping: tokio_util::sync::CancellationToken,
        accepting: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Self {
        Self {
            address,
            stopping,
            accepting: Box::pin(accepting),
        }
    }

    /// Where it bound, which is the only way to learn the port after asking for zero (TRN-1).
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Accepts until `signal` resolves, then closes the sessions it opened.
    ///
    /// The accept loop does not end on its own, so reaching the end of it first is a failure
    /// however it happened: nothing restarts it, and agents can no longer connect.
    pub async fn with_graceful_shutdown(
        mut self,
        signal: impl std::future::Future<Output = ()> + Send,
    ) -> Result<(), TunnelError> {
        tokio::select! {
            () = &mut self.accepting => Err(TunnelError::fault(
                "the accept loop stopped, so agents can no longer connect",
            )),
            () = signal => {
                self.stopping.cancel();

                self.accepting.await;

                Ok(())
            }
        }
    }
}

impl std::future::IntoFuture for Serving {
    type Output = ();
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        self.accepting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_broker_address_with_name_returns_host_and_port() {
        let address: BrokerAddress = "broker.example.com:8443".parse().unwrap();

        assert_eq!(
            (address.host(), address.port()),
            ("broker.example.com", 8443)
        );
    }

    #[test]
    fn parse_broker_address_with_bracketed_ipv6_returns_host_without_brackets() {
        let address: BrokerAddress = "[::1]:8443".parse().unwrap();

        assert_eq!((address.host(), address.port()), ("::1", 8443));
    }

    #[test]
    fn round_trip_broker_address_keeps_text() {
        let texts = ["broker.example.com:8443", "[::1]:8443"];

        let written = texts.map(|text| text.parse::<BrokerAddress>().unwrap().to_string());

        assert_eq!(written, texts);
    }

    #[test]
    fn parse_broker_address_invalid_returns_error() {
        let invalid_texts = [
            "::1",
            "fe80::1:8443",
            "broker:99999",
            "broker:0",
            "broker:",
            "broker",
            ":8443",
            "[]:8443",
        ];

        let accepted: Vec<_> = invalid_texts
            .into_iter()
            .filter(|text| text.parse::<BrokerAddress>().is_ok())
            .collect();

        assert_eq!(accepted, Vec::<&str>::new());
    }
}
