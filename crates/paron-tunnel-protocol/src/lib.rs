//! PTP/1, the Paron Tunnel Protocol.
//!
//! An agent runs on a host the broker has no way to dial — behind NAT, behind a firewall, on
//! someone's home network. So the agent dials the broker instead, and keeps that one connection
//! open. Everything after that runs backwards down it: the broker opens byte streams to the agent,
//! and each one lands on a local service — an SSH port, a UDP port, a UNIX socket. The connection
//! goes one way and the service goes the other, which is the whole idea.
//!
//! The normative specification is `paron-tunnel-protocol-rfc.md`, beside this crate's manifest.
//! Every `§n` in this crate cites it, as does every label: `TRN-n` transport, `POL-n` policy,
//! `STM-n` state machines, `VER-n` versioning, `CNF-n` conformance, `SEC-n` security. Appendix D
//! lists them all.
//!
//! | module | RFC | implemented by |
//! |---|---|---|
//! | `wire`, `identity` | §2 encodings, §3 vocabulary | the wire forms themselves |
//! | `agent`, `broker`, `control` | §4 state machines, TRN-10 limits | this crate |
//! | `connected_agents` | §6.1 registration | this crate |
//! | `liveness` | §6.2 liveness | this crate |
//! | `transport`, `tls` | §7, TRN-n | this crate, once per mapping |
//! | `quic`, `tcp` | appendices A and B | this crate |
//! | `policy` | §8, POL-n | the embedding application |
//! | `error` | STM-8 error classification | this crate |
//!
//! The state machines decide nothing on their own (STM-9). Anything that amounts to a judgement —
//! may this agent connect, is this token good, may this stream reach that port — is a call into
//! [`BrokerPolicy`] or [`ServedSelectors`], which the application implements. The transport
//! mappings are just as narrow: they hand up byte streams and the peer's TLS key, nothing more.
//! If you change one thing in this crate, keep that split.
//!
//! Every module is private and re-exported here, so there is one path to each item — except the
//! two transport mappings, which stay modules of their own so that a deployment names the one it
//! is using. This crate speaks PTP/1 and nothing else ([`VERSION`]).
//!
//! # Surfaces
//!
//! **Applications** run an [`Agent`] or a [`Broker`], implement [`BrokerPolicy`] and
//! [`ServedSelectors`] to answer the judgements, and open streams to agents through
//! [`ConnectedAgents`] — which is also where you cut one off again, with
//! [`ConnectedAgents::disconnect`]. An application never sees a frame.
//!
//! **Transport mappings** are the other seam. A mapping implements [`AgentTransport`] or
//! [`BrokerTransport`]; on the broker side it hands every accepted connection to
//! [`serve_connection`] and that speaks the protocol over it. [`quic`] and [`tcp`] are the two the
//! RFC specifies for PTP/1 (VER-5), and they are written against this seam like anyone else's
//! would be.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod agent;
mod broker;
mod connected_agents;
mod control;
mod error;
mod identity;
mod liveness;
mod policy;
mod tls;
mod transport;
mod wire;

pub mod quic;
pub mod tcp;

#[cfg(test)]
mod conformance;

pub use agent::{Agent, AgentIdentity};
pub use broker::{Broker, Limits, serve_connection};
pub use connected_agents::{Connected, ConnectedAgents, Tunnel};
pub use error::TunnelError;
pub use identity::{
    AgentId, BrokerIdentity, Fingerprint, Identity, InMemoryRollbackIndex, NotAFingerprint,
    NotAKey, NotAKeypair, PublicKey, RollbackIndex,
};
pub use liveness::{
    Liveness, MAXIMUM_PROBE_INTERVAL, MINIMUM_PROBE_INTERVAL, MISSED_PROBES, MOST_MISSED_PROBES,
    PROBE_INTERVAL,
};
pub use policy::{
    BrokerPolicy, Rejected, Selectors, Sequence, SequenceOutcome, ServedSelectors, StillBound,
};
pub use transport::{
    AgentConnection, AgentTransport, BrokerAddress, BrokerConnection, BrokerTransport, Closer,
    IncomingStreams, PinnedBroker, Serving, Stream, StreamOpener,
};
pub use wire::{Access, AnnounceSet, Announced, PortRange, Selector, UnixPath, VERSION};
