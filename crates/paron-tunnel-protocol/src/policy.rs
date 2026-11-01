//! Judgements the protocol leaves to the embedder (RFC §8).
//!
//! [`BrokerPolicy`] is §8.1: what the broker's records say. [`ServedSelectors`] is §8.2's POL-8.
//! Which agent ids exist, where records live and how tokens reach an operator are all outside
//! this crate (STM-9). POL-7 and POL-9 have no trait; they are values an agent is constructed
//! with.

use std::collections::BTreeSet;

use async_trait::async_trait;
use secrecy::SecretSlice;

use crate::error::TunnelError;
use crate::identity::{AgentId, PublicKey};
use crate::wire::{Access, AnnounceSet, Announced, Selector};

/// §8.1, asked of whatever holds the broker's records.
///
/// POL-4, issuance, is not here either, and for a plainer reason: nothing in this protocol issues
/// a join token. A broker only ever verifies one, so the entropy, the expiry, the commitment and
/// the one identifier a token is issued for are all requirements on whatever an operator gets
/// tokens from.
///
/// POL-5, withdrawal, is [`unassign_identity`](BrokerPolicy::unassign_identity). No message of
/// this protocol requests it, but a broker invokes it on its own where a sequence has gone
/// backwards (§4.2), which is the whole of freezing. An embedder exposes it to its operators as
/// well, on an interface of its own: [`assign_identity`](BrokerPolicy::assign_identity) refuses
/// any key but the bound one, so without withdrawal a lost private key leaves an agent
/// unreachable for good, and recovery is withdraw, issue a token, join.
#[async_trait]
pub trait BrokerPolicy: Send + Sync {
    /// **POL-1.** Is `authenticated_key` the key bound to `agent`?
    ///
    /// Pure: no writes. The transport has already proven possession (TRN-3), so this only
    /// compares against what is stored. Byte equality over the §2.2 encoding is enough, since
    /// public keys are public.
    ///
    /// Answer `Denied` for an unknown id, for a known id with no key bound, and for a known id
    /// presenting the wrong key. Those three must be indistinguishable to the caller, both in
    /// what is returned and in how long it takes (SEC-7).
    async fn verify_access(&self, agent: AgentId, authenticated_key: &PublicKey) -> Access;

    /// **POL-2.** Bind `authenticated_key` to `agent` if `token` authorises it.
    ///
    /// The token is the octets the `JOIN` frame carried (§2.4). It is not required to be text:
    /// an implementation that issues text tokens decodes them here, and one that cannot decode
    /// this token has a token that authorises nothing.
    ///
    /// An implementation must:
    ///
    /// - compare the token in constant time against a stored one-way commitment, and never store
    ///   the token itself;
    /// - reject an expired token, and one issued for a different agent;
    /// - reject any token if a key is already bound, so a stolen one cannot replace a working
    ///   agent's key. Unbinding one is withdrawal (POL-5), which an operator invokes out of band;
    /// - invalidate the token on success, so it is single-use (SEC-4);
    /// - commit the binding before returning. `Ok` is what makes the broker answer `Granted`,
    ///   and an agent that dies immediately afterwards must be joined, not half-joined (SEC-3).
    ///
    /// Compare, invalidate and bind in one atomic step. Two connections presenting the same
    /// token at once must produce at most one binding.
    async fn assign_identity(
        &self,
        agent: AgentId,
        authenticated_key: &PublicKey,
        token: &SecretSlice<u8>,
    ) -> Result<(), Rejected>;

    /// **POL-6.** Record `agent`'s claim sequence, and say how it compares.
    ///
    /// Return `Current` and record `sequence` if it exceeds the last recorded for this agent;
    /// with nothing recorded, any sequence is `Current`. Return `Stale` if it does not exceed
    /// it. Return `Undetermined` if the record can be neither read nor written.
    ///
    /// Compare and record in one atomic step (STM-18). Two connections racing must not both be
    /// told `Current`.
    ///
    /// Recording is all this does. `Stale` is the broker's cue to freeze, and freezing is
    /// [`unassign_identity`](BrokerPolicy::unassign_identity) — do not perform it here, and do
    /// not invent a frozen state of your own. It is the binding that is durable, so a freeze
    /// held anywhere else would not survive a restart.
    ///
    /// `Undetermined` is not `Stale`. Both refuse the claim, and only `Stale` unbinds anything:
    /// answering `Stale` for a store that could not be reached would let an outage of the
    /// broker's own storage cost an agent its binding, and spend on it the one alarm SEC-6
    /// rests on.
    ///
    /// Binding a key under [`assign_identity`](BrokerPolicy::assign_identity) clears the record
    /// for that agent. Without that a frozen identity cannot be recovered: a holder that
    /// presented a sequence near `u64::MAX` leaves nothing an honest agent could exceed.
    ///
    /// The protocol calls this only after [`verify_access`](BrokerPolicy::verify_access) has
    /// granted (§4.2). Agent ids are public, so a sequence weighed before the key is proven
    /// would let anyone shut an agent down.
    async fn advance_sequence(&self, agent: AgentId, sequence: u64) -> SequenceOutcome;

    /// **POL-5.** Unbind whatever key `agent` has, so that nothing but a fresh token admits it.
    ///
    /// The broker calls this where [`advance_sequence`](BrokerPolicy::advance_sequence) answered
    /// `Stale`: one key is in two places, and freezing the identity for both is the only answer
    /// available to a party that cannot tell them apart (§4.2). An operator invokes the same
    /// operation out of band for a key that was lost.
    ///
    /// Unbinding an agent that has no key bound is the postcondition already, and succeeds.
    ///
    /// `Err` is a fault of the environment and not a verdict — the key is still bound, and the
    /// next claim to exceed the recorded sequence will be admitted. A broker reports that to its
    /// operator (STM-21) and can do nothing else about it, the binding not being the broker's to
    /// remove.
    async fn unassign_identity(&self, agent: AgentId) -> Result<(), StillBound>;
}

/// What [`BrokerPolicy::advance_sequence`] made of a claim's sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceOutcome {
    /// Ahead of everything recorded, and now recorded.
    Current,
    /// Not ahead. One key is in two places, and the broker freezes the identity for both.
    Stale,
    /// The record could not be reached, so nothing was established about the claim. Refused
    /// like `Stale` and acted on unlike it: a fault of the broker's own storage is not evidence
    /// against an agent (POL-6).
    Undetermined,
}

/// A withdrawal did not happen (POL-5).
///
/// Carries no reason, because the only party that can act on one has it already: whatever
/// refused the write knows why, and the broker knows only that an identity it meant to freeze
/// is still bound and still reachable (STM-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StillBound;

/// A policy said no.
///
/// Carries no reason. The broker's only use for it is to answer [`Access::Denied`], and an
/// agent told why its token failed (expired, wrong, already spent) learns what a prober wants
/// to know (SEC-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected;

impl From<Rejected> for Access {
    fn from(_: Rejected) -> Self {
        Self::Denied
    }
}

/// **POL-10, §8.2.** The sequence an agent presents in its next claim.
///
/// Each one must be greater than the last, and must be written to durable storage *before* it
/// is returned (STM-15). Recording first is what makes a crash harmless: a broker requires each
/// claim to exceed the last it accepted, not to follow it, so a number that was recorded but
/// never sent is spent.
///
/// An agent that loses this state — restored from backup, or copied to a second host — presents
/// a sequence the broker has already seen and freezes its own identity. A broker cannot tell
/// that from a stolen key, and does not try.
pub trait Sequence: Send + Sync {
    /// The next sequence, already recorded. An agent that gets `Err` must not connect.
    fn next(&self) -> Result<u64, TunnelError>;
}

/// **POL-8, §8.2.** Which selectors this agent serves.
///
/// A trait because the answer can come from anywhere: a config file, a service inventory, the
/// time of day. Most agents want [`Selectors`].
///
/// Answering is the whole of the judgement, and *where* a served selector terminates is not a
/// choice: PTP/1 has one host designator, the agent's own host (§2.3, §2.5), so a `tcp-port`
/// reaches that port on loopback, a `udp-port` likewise, and a `unix-path` the socket it names.
/// SEC-5 then holds by construction — an agent cannot front another host, because nothing here
/// can name one.
pub trait ServedSelectors: Send + Sync {
    /// The set one `ANNOUNCE` carries (§3.1): what this agent expects to serve.
    /// Informational; [`serves`](ServedSelectors::serves) is the sole authority over what a
    /// stream reaches.
    ///
    /// A member may name no service, which announces a set without enumerating it (§2.3). Such a
    /// member still stands for the selectors it identifies, so [`serves`](ServedSelectors::serves)
    /// has to answer for each of them.
    fn announced(&self) -> AnnounceSet;

    /// Whether a stream naming `selector` is served, or refused.
    ///
    /// `false` is a normal answer, not an error: the broker is told `STREAM_REFUSE` and the
    /// connection carries on. It reads the same as a service that is announced but not
    /// answering, which SEC-7 requires.
    ///
    /// `selector` names one service, because that is what a [`Selector`] is (§2.3). A set-naming
    /// type reaches this nowhere: it has its own type, [`Announced`], and no preamble carries one.
    fn serves(&self, selector: &Selector) -> bool;
}

/// A fixed announced set, and nothing served beyond what its members identify.
///
/// The set is announced as it stands. It answers each stream by what its members identify (§2.3).
/// A `tcp-port-range` of 1 to 65535 is therefore how an agent serves every TCP port: one member
/// to announce, and every port served.
pub struct Selectors(BTreeSet<Announced>);

impl Selectors {
    pub fn new(members: impl IntoIterator<Item = Announced>) -> Self {
        Self(members.into_iter().collect())
    }
}

impl ServedSelectors for Selectors {
    fn announced(&self) -> AnnounceSet {
        AnnounceSet::new(self.0.iter().cloned())
    }

    fn serves(&self, selector: &Selector) -> bool {
        self.0.iter().any(|member| member.identifies(selector))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::PortRange;

    #[test]
    fn serves_selector_a_member_identifies_returns_true() {
        let selectors = Selectors::new([
            Announced::TcpRange(PortRange::new(8000, 8080).unwrap()),
            Selector::tcp(22).unwrap().into(),
        ]);

        let served = [Selector::tcp(22), Selector::tcp(8080)]
            .map(|selector| selectors.serves(&selector.unwrap()));

        assert_eq!(served, [true, true]);
    }

    #[test]
    fn serves_selector_no_member_identifies_returns_false() {
        let selectors = Selectors::new([
            Announced::TcpRange(PortRange::new(8000, 8080).unwrap()),
            Selector::tcp(22).unwrap().into(),
        ]);

        let served = [Selector::tcp(23), Selector::udp(22)]
            .map(|selector| selectors.serves(&selector.unwrap()));

        assert_eq!(served, [false, false]);
    }
}
