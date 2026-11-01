//! Scaffolding both benchmarks need: a policy that decides nothing, a sequence counter, and an
//! echo service to point agents at.
//!
//! None of it is an example to copy. POL-1 is what decides who may connect and POL-10 is durable
//! on a real agent; neither holds here, because what is under measurement is the protocol
//! carrying traffic rather than the records behind it.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};

use paron_tunnel_protocol::{
    Access, AgentId, BrokerPolicy, PublicKey, Rejected, Sequence, SequenceOutcome, StillBound,
    TunnelError,
};

/// Grants every claim, so a measurement carries no record store's cost.
pub struct GrantAll;

#[async_trait::async_trait]
impl BrokerPolicy for GrantAll {
    async fn verify_access(&self, _agent: AgentId, _key: &PublicKey) -> Access {
        Access::Granted
    }

    async fn assign_identity(
        &self,
        _agent: AgentId,
        _key: &PublicKey,
        _token: &secrecy::SecretSlice<u8>,
    ) -> Result<(), Rejected> {
        Ok(())
    }

    async fn advance_sequence(&self, _agent: AgentId, _presented: u64) -> SequenceOutcome {
        SequenceOutcome::Current
    }

    async fn unassign_identity(&self, _agent: AgentId) -> Result<(), StillBound> {
        Ok(())
    }
}

/// POL-10 in memory. Starts at 1, since 0 is not a sequence (§2.1).
#[derive(Default)]
pub struct Counting(AtomicU64);

impl Sequence for Counting {
    fn next(&self) -> Result<u64, TunnelError> {
        Ok(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

/// Deterministic ids, so a broker process and an agent process agree on them without exchanging
/// any.
pub fn agent_id(n: usize) -> AgentId {
    AgentId::parse(&format!("00000000-0000-7000-8000-{n:012x}")).unwrap()
}

/// One TCP echo on loopback, which is where every agent's selectors point.
pub async fn tcp_echo() -> u16 {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        while let Ok((mut connection, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = connection.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });

    port
}
