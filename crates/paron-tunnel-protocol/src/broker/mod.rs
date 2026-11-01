//! The broker state machine (RFC §4.2), and the limits a broker runs under (TRN-10).

mod session;
mod unadmitted;

pub(crate) use session::DENIAL_DELAY;
pub use session::serve_connection;
pub use unadmitted::Slot;
pub(crate) use unadmitted::UnadmittedConnections;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretSlice;

use crate::connected_agents::{Connected, ConnectedAgents, Tunnel};
use crate::error::TunnelError;
use crate::identity::{AgentId, BrokerIdentity, PublicKey};
use crate::liveness::Liveness;
use crate::policy::{BrokerPolicy, SequenceOutcome};
use crate::transport::{BrokerTransport, Serving};
use crate::wire::{Access, Claim};

/// What agents dial, and what makes them reachable once they have (RFC §4.2).
///
/// [`serve_connection`] calls the steps of §4.2 in the order the wire reaches them — answer a
/// claim, answer a join, register — and decides none of them (STM-9). Unlike [`crate::Agent`]
/// this half cannot read as a sequence: a responder does not choose what arrives next, so its
/// per-connection state lives in the driver's locals (STM-2).
pub struct Broker {
    policy: Arc<dyn BrokerPolicy>,
    connected_agents: Arc<ConnectedAgents>,
    identity: BrokerIdentity,
    liveness: Liveness,
    limits: Limits,
    unadmitted: UnadmittedConnections,
}

impl Broker {
    /// `liveness` and `limits` are this broker's own; [`Default`] on both is what a deployment
    /// with no reason to choose otherwise uses (§6.2, TRN-10).
    pub fn new(
        policy: Arc<dyn BrokerPolicy>,
        connected_agents: Arc<ConnectedAgents>,
        identity: BrokerIdentity,
        liveness: Liveness,
        limits: Limits,
    ) -> Self {
        Self {
            policy,
            connected_agents,
            identity,
            liveness,
            limits,
            unadmitted: UnadmittedConnections::with_capacity(limits.max_unadmitted),
        }
    }

    /// What agents are told to pin (§2.2, POL-7). It goes into join tokens, is fixed for the
    /// deployment's lifetime (§12.4), and authenticates this broker on both mappings alike, so
    /// adding one never re-joins the fleet.
    pub fn identity(&self) -> PublicKey {
        self.identity.root_public_key()
    }

    pub(crate) fn keys(&self) -> &BrokerIdentity {
        &self.identity
    }

    pub(crate) fn liveness(&self) -> Liveness {
        self.liveness
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    pub(crate) fn unadmitted(&self) -> &UnadmittedConnections {
        &self.unadmitted
    }

    /// Bind `transport` to `address`. Nothing is accepted until the [`Serving`] that comes back is
    /// awaited, so the port it bound can be read first, which is the only way to learn one asked
    /// for as zero (TRN-1).
    ///
    /// A broker MAY listen on both mappings at once (§6.3): call this once per transport against
    /// one `Broker`, and STM-11 holds across the pair, enforced by the one [`ConnectedAgents`]
    /// behind it.
    pub async fn bind(
        self: &Arc<Self>,
        transport: &dyn BrokerTransport,
        address: SocketAddr,
    ) -> Result<Serving, TunnelError> {
        transport.bind(address, Arc::clone(self)).await
    }

    /// **CLAIM.** An agent has proven possession of a private key (TRN-3) and named who it is.
    ///
    /// The id comes back with the verdict because the driver needs it for the steps that follow —
    /// including after a denial, a `JOIN` on the same connection being answered under the id the
    /// denied claim named (§4.2).
    pub(crate) async fn answer_claim(
        &self,
        claim: &Claim,
        authenticated_key: &PublicKey,
    ) -> ClaimResponse {
        let agent = claim.agent();

        // POL-1 before POL-6, and the order is load-bearing: an agent identifier is public
        // (§1.2), so a broker weighing the sequence first would let anyone who knows an
        // identifier freeze it by presenting a low one with any private key at all.
        match self.policy.verify_access(agent, authenticated_key).await {
            Access::Denied => ClaimResponse::Denied(agent),
            Access::Granted => match self.policy.advance_sequence(agent, claim.sequence()).await {
                SequenceOutcome::Current => ClaimResponse::Granted(agent),
                SequenceOutcome::Stale => {
                    self.freeze(agent, claim.sequence()).await;

                    ClaimResponse::Denied(agent)
                }
                // Refused, and nothing learned: the agent keeps its binding, and the operator is
                // told about a broken store rather than about a stolen key (POL-6).
                SequenceOutcome::Undetermined => {
                    tracing::warn!(%agent, "agent connection refused: could not read stored sequence number");

                    ClaimResponse::Denied(agent)
                }
            },
        }
    }

    /// Freezing, which §4.2 defines as POL-5 applied to the identifier: the key is unbound, so
    /// every later claim for it is denied by POL-1 without reference to any sequence, and the
    /// freeze survives a restart because the binding does.
    ///
    /// Refusing only the claim that went backwards would hand the identity to whichever party
    /// connected most recently, and in the case this exists to catch that is the thief.
    async fn freeze(&self, agent: AgentId, sequence: u64) {
        // STM-16, and answered like every other denial (SEC-7): that an identity was frozen is
        // the operator's to see and nobody else's.
        tracing::warn!(
            %agent,
            sequence,
            "agent blocked: invalid sequence number"
        );

        if self.policy.unassign_identity(agent).await.is_err() {
            // STM-21. Nothing else is available: the binding is not the broker's to remove, and
            // no answer on the wire distinguishes an identity that was frozen from one that was
            // meant to be.
            tracing::error!(
                %agent,
                "could not block an agent with an invalid sequence number"
            );

            return;
        }

        // POL-5's second half. The claim being answered is not registered — POL-3 runs only
        // after Current — so this ends the registration some earlier connection holds.
        self.connected_agents.disconnect(agent).await;
    }

    /// **JOIN.** The token settles a claim that was denied, which is every claim an agent that
    /// has not joined yet can make (§5.2).
    pub(crate) async fn answer_join(
        &self,
        agent: AgentId,
        authenticated_key: &PublicKey,
        token: &SecretSlice<u8>,
    ) -> Access {
        match self
            .policy
            .assign_identity(agent, authenticated_key, token)
            .await
        {
            Ok(()) => Access::Granted,
            Err(rejected) => rejected.into(),
        }
    }

    /// This connection becomes the way to reach `agent`, and any earlier one is ended (POL-3,
    /// STM-11, §6.1).
    ///
    /// Later connection wins, always. Which of two connections is "later" is simply which one got
    /// here second — never the sequence numbers their claims carried, because POL-6 has already
    /// compared those and admitted this one on the strength of it.
    ///
    /// The broker calls this before it sends `ACCESS`(Granted), which is the order §6.1 requires.
    pub(crate) async fn connect_agent(&self, agent: AgentId, tunnel: Arc<dyn Tunnel>) -> Connected {
        self.connected_agents.connect(agent, tunnel).await
    }

    /// The agents this broker can currently reach. This is the surface a consumer opens streams
    /// through; deciding *which* consumers may do so is the application's job, not this crate's
    /// (§12.3).
    pub fn connected_agents(&self) -> &Arc<ConnectedAgents> {
        &self.connected_agents
    }
}

/// TRN-10's broker-side bounds, beside the two the RFC fixes (the 1-second denial delay and the
/// frame-rate bound, both constants of the driver).
///
/// Neither value is fixed by the RFC; each constrains one role only, and nothing breaks if two
/// deployments choose differently.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most connections a broker holds that have not been admitted.
    pub max_unadmitted: usize,
    /// How long a connection has to reach admission before it is ended. Separate from liveness
    /// (§6.2), which tolerates long silences from a peer that has proven something.
    pub admission_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_unadmitted: 64,
            admission_timeout: Duration::from_secs(30),
        }
    }
}

/// What a claim came to, with the id the driver needs for the steps after it.
///
/// Not [`Access`], which is the wire answer: the id on `Denied` never reaches the agent, and
/// without it a `JOIN` arriving next would have nothing to be answered under (§4.2). Named on
/// `Denied` rather than known — an id nobody has heard of is carried just as a known one is, so
/// the driver cannot phrase a denial differently for them (SEC-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimResponse {
    Granted(AgentId),
    Denied(AgentId),
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use async_trait::async_trait;

    use super::*;
    use crate::conformance::Reported;
    use crate::policy::{Rejected, SequenceOutcome, StillBound};

    struct StaleSequencePolicy {
        unassign_succeeds: bool,
    }

    #[async_trait]
    impl BrokerPolicy for StaleSequencePolicy {
        async fn verify_access(&self, _agent: AgentId, _key: &PublicKey) -> Access {
            Access::Granted
        }

        async fn assign_identity(
            &self,
            _agent: AgentId,
            _key: &PublicKey,
            _token: &SecretSlice<u8>,
        ) -> Result<(), Rejected> {
            unreachable!("freezing reaches no join")
        }

        async fn advance_sequence(&self, _agent: AgentId, _presented: u64) -> SequenceOutcome {
            SequenceOutcome::Stale
        }

        async fn unassign_identity(&self, _agent: AgentId) -> Result<(), StillBound> {
            if self.unassign_succeeds {
                Ok(())
            } else {
                Err(StillBound)
            }
        }
    }

    fn broker_with_policy(policy: StaleSequencePolicy) -> Broker {
        Broker::new(
            Arc::new(policy),
            Arc::new(ConnectedAgents::new()),
            BrokerIdentity::generate().unwrap(),
            Liveness::default(),
            Limits::default(),
        )
    }

    // `said` holds the output of every test in the process, so only the lines that name this
    // agent count.
    fn lines_about(operator_reports: &Reported, agent: AgentId) -> Vec<String> {
        operator_reports
            .said()
            .into_iter()
            .filter(|line| line.contains(&agent.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn freeze_with_unassign_succeeding_reports_agent_blocked() {
        let operator_reports = Reported::watching();
        let agent = AgentId::new(uuid::Uuid::now_v7());
        let broker = broker_with_policy(StaleSequencePolicy {
            unassign_succeeds: true,
        });

        broker.freeze(agent, 7).await;

        assert_matches!(
            lines_about(&operator_reports, agent).as_slice(),
            [blocked] if blocked.starts_with("agent blocked: invalid sequence number")
        );
    }

    #[tokio::test]
    async fn freeze_with_unassign_failing_reports_could_not_block() {
        let operator_reports = Reported::watching();
        let agent = AgentId::new(uuid::Uuid::now_v7());
        let broker = broker_with_policy(StaleSequencePolicy {
            unassign_succeeds: false,
        });

        broker.freeze(agent, 7).await;

        assert_matches!(
            lines_about(&operator_reports, agent).as_slice(),
            [blocked, still_bound]
                if blocked.starts_with("agent blocked: invalid sequence number")
                    && still_bound
                        .starts_with("could not block an agent with an invalid sequence number")
        );
    }
}
