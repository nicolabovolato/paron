use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use parking_lot::{Mutex, MutexGuard};
use secrecy::{ExposeSecret as _, SecretSlice};

use crate::{
    Access, AgentId, BrokerPolicy, PublicKey, Rejected, Sequence, SequenceOutcome, StillBound,
    TunnelError,
};

/// An in-memory §8.1 policy, so a test can put a broker in a known state and then reach it over
/// a mapping.
///
/// **Not an example to copy.** It compares tokens with plain equality and stores them in the
/// clear, which POL-2 forbids — a real policy keeps an HMAC-SHA-256 commitment and compares in
/// constant time — and no transport can tell the difference. What it does honour is the part a
/// transport can observe: single use, expiry, and committing the binding before answering.
pub struct Records {
    state: Mutex<HashMap<AgentId, Record>>,
    /// The highest sequence POL-6 has accepted per agent. Separate from `state` because it
    /// outlives a withdrawal: only binding a key clears it (POL-6), so an operator revoking and
    /// re-issuing does not reset the one thing a thief cannot reset.
    sequences: Mutex<HashMap<AgentId, u64>>,
    consulted: AtomicUsize,
}

/// POL-10 in memory. A real agent's is durable (STM-15) — forgetting a sequence freezes an
/// identity, and this forgets at every restart.
pub struct Counting(std::sync::atomic::AtomicU64);

impl Counting {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(std::sync::atomic::AtomicU64::new(0)))
    }
}

impl Sequence for Counting {
    fn next(&self) -> Result<u64, TunnelError> {
        // From 1: 0 is not a sequence (§2.1). `fetch_add` alone would wrap back onto numbers
        // it has already returned, which is the one thing POL-10 must never do.
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                last.checked_add(1)
            })
            .map(|last| last + 1)
            .map_err(|_| TunnelError::unexpected(std::io::Error::other("claim sequence exhausted")))
    }
}

enum Record {
    /// Known, with a token outstanding and no private key bound. Not a wire verdict: a claim
    /// for one of these is answered `Denied` like any other (SEC-7).
    Unbound {
        token: String,
        expired: bool,
    },
    Joined {
        key: PublicKey,
    },
}

impl Records {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HashMap::new()),
            sequences: Mutex::new(HashMap::new()),
            consulted: AtomicUsize::new(0),
        })
    }

    /// An agent the broker knows of, waiting for a token to settle which private key is its
    /// own.
    pub fn unbound(&self, agent: AgentId, token: &str) {
        self.write(agent, token, false);
    }

    /// The same, with a token that ran out.
    pub fn expired(&self, agent: AgentId, token: &str) {
        self.write(agent, token, true);
    }

    /// An agent that already joined, with the private key it joined with.
    pub fn joined(&self, agent: AgentId, key: PublicKey) {
        self.state().insert(agent, Record::Joined { key });
    }

    /// Which private key is bound to an agent, if any: what a join must have persisted, and what
    /// a refusal must not have.
    pub fn bound(&self, agent: AgentId) -> Option<PublicKey> {
        match self.state().get(&agent) {
            Some(Record::Joined { key }) => Some(key.clone()),
            Some(Record::Unbound { .. }) | None => None,
        }
    }

    /// How many times policy was asked anything. Zero is evidence of a negative: an agent that
    /// never identified itself (CNF-5), a frame that never reached policy (CNF-10).
    pub fn consulted(&self) -> usize {
        self.consulted.load(Ordering::Relaxed)
    }

    fn write(&self, agent: AgentId, token: &str, expired: bool) {
        self.state().insert(
            agent,
            Record::Unbound {
                token: token.to_owned(),
                expired,
            },
        );
    }

    /// An operator's withdrawal, which is POL-5 reached from outside the protocol rather than
    /// by a broker answering Stale. Here so CNF-11 can perform one.
    pub fn withdraw(&self, agent: AgentId) {
        self.state().remove(&agent);
    }

    fn state(&self) -> MutexGuard<'_, HashMap<AgentId, Record>> {
        self.state.lock()
    }
}

#[async_trait]
impl BrokerPolicy for Records {
    async fn verify_access(&self, agent: AgentId, authenticated_key: &PublicKey) -> Access {
        self.consulted.fetch_add(1, Ordering::Relaxed);

        match self.state().get(&agent) {
            Some(Record::Joined { key }) if key == authenticated_key => Access::Granted,
            // An id nobody knows, one known but unbound, and one presenting the wrong key are
            // one answer and must stay one (SEC-7).
            _ => Access::Denied,
        }
    }

    async fn assign_identity(
        &self,
        agent: AgentId,
        authenticated_key: &PublicKey,
        token: &SecretSlice<u8>,
    ) -> Result<(), Rejected> {
        self.consulted.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state();

        match state.get(&agent) {
            // Matching only `Unbound` is what refuses a token for an id that already has a
            // private key: a stolen token must not displace a working agent, and unbinding one
            // is `withdraw` and an operator's decision.
            Some(Record::Unbound {
                token: expected,
                expired,
            }) if !expired && expected.as_bytes() == token.expose_secret() => {
                // Committed before the broker can answer, and the token is spent with it. One
                // lock spans checking, spending and binding, so two connections racing the
                // same token cannot both be granted.
                state.insert(
                    agent,
                    Record::Joined {
                        key: authenticated_key.clone(),
                    },
                );

                // POL-6: binding clears the record, or an identity frozen by a holder that
                // presented a sequence near `u64::MAX` could never be recovered.
                self.sequences.lock().remove(&agent);

                Ok(())
            }
            _ => Err(Rejected),
        }
    }

    async fn advance_sequence(&self, agent: AgentId, sequence: u64) -> SequenceOutcome {
        self.consulted.fetch_add(1, Ordering::Relaxed);

        // One lock spans comparing and recording, so two connections racing cannot both be
        // told Current, which would leave the mechanism reporting nothing.
        let mut sequences = self.sequences.lock();

        match sequences.get(&agent) {
            // Strictly greater, so gaps are ordinary: an agent that crashed after recording a
            // sequence it never sent resumes further forward rather than repeating itself.
            Some(last) if sequence > *last => {
                sequences.insert(agent, sequence);

                SequenceOutcome::Current
            }
            Some(_) => SequenceOutcome::Stale,
            // First contact: any sequence is ahead of nothing.
            None => {
                sequences.insert(agent, sequence);

                SequenceOutcome::Current
            }
        }
    }

    async fn unassign_identity(&self, agent: AgentId) -> Result<(), StillBound> {
        self.consulted.fetch_add(1, Ordering::Relaxed);
        self.state().remove(&agent);

        // The sequence outlives the binding on purpose: POL-6 clears it when a key is bound,
        // and not before, so a frozen identity stays frozen until an operator issues a token.
        Ok(())
    }
}

/// Captures what a broker reports to its operator, so STM-12 and STM-16 can be asserted rather
/// than read.
///
/// The buffer is process-wide because the events come from spawned per-connection drivers rather
/// than from the asserting thread. Tests running in parallel therefore see each other's lines, so
/// an assertion names something of its own: an agent id, or a message no other test emits.
pub struct Reported {
    watermark: usize,
}

static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl Reported {
    /// Start recording, remembering where the process-wide buffer stood.
    pub fn watching() -> Self {
        static INSTALLED: std::sync::Once = std::sync::Once::new();

        INSTALLED.call_once(|| {
            // Ignored on purpose: a binary that installed its own subscriber first is not one
            // these assertions can run under, and `said` returning nothing says so.
            let _ = tracing::subscriber::set_global_default(Collector);
        });

        Self {
            watermark: SAID.lock().len(),
        }
    }

    /// What the process has emitted since [`Reported::watching`] — from every thread, not
    /// only the caller's. Tests run concurrently in one process, so an assertion over this
    /// names something of its own, an agent id say, or it answers for whatever else was
    /// running: without that a negative assertion fails on another test's output, and a
    /// positive one passes on it.
    pub fn said(&self) -> Vec<String> {
        SAID.lock()[self.watermark..].to_vec()
    }
}

struct Collector;

impl tracing::Subscriber for Collector {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = Said(String::new());
        event.record(&mut line);

        SAID.lock().push(line.0);
    }

    fn enter(&self, _: &tracing::Id) {}

    fn exit(&self, _: &tracing::Id) {}
}

/// Renders every field of an event into one line, message first, so an assertion can match on
/// the agent id a `warn!` carried as a field.
struct Said(String);

impl tracing::field::Visit for Said {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?} {}", self.0);
        } else {
            self.0 = format!("{} {}={value:?}", self.0, field.name());
        }
    }
}
