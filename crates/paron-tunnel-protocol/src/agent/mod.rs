//! The agent side of the protocol: the state machine of RFC §4.1 and the join and serve
//! procedures of §5.
//!
//! The agent dials out and never listens. Every connection in this module is one it opened itself,
//! and it binds no port that anything could connect to. That is STM-17, and SEC-2 — "an agent has
//! no listening surface" — is just the consequence of it.
//!
//! There is no code here for VER-3's version retry, because this crate implements exactly one
//! version and there is nothing to retry with. If a second version is ever added, the rules are:
//! only retry after a denial from a broker whose identity was verified, never after a transport
//! failure, and never try the same version twice in one call.

mod race;
mod streams;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::control::{self, Frames, Probes};
use crate::error::TunnelError;
use crate::identity::{AgentId, Identity};
use crate::liveness::Liveness;
use crate::policy::{Sequence, ServedSelectors};
use crate::transport::{AgentConnection, AgentTransport, Closer, PinnedBroker, Stream};
use crate::wire::{self, Access, Claim, Frame};

/// How long the agent waits for the answer to an admission attempt. Longer than the broker's
/// denial delay (TRN-10), and shorter than a broker that will never answer.
const ANSWER_WAIT: Duration = Duration::from_secs(30);

/// STM-14's reconnection bound B as it starts, and as a stable connection returns it.
const RECONNECTION_BOUND: Duration = Duration::from_secs(5);

/// STM-14's ceiling on the doubling of B.
const RECONNECTION_CEILING: Duration = Duration::from_secs(60);

/// One agent's side of one broker (RFC §4.1, §5).
///
/// Unlike [`crate::Broker`] this half reads as a sequence, an initiator choosing what happens
/// next: the states of §4.1 are positions inside [`join`](Agent::join) and [`serve`](Agent::serve),
/// which are the only things that drive a connection (STM-1). No partly-advanced connection is
/// public, so a caller cannot take the states out of order.
pub struct Agent {
    /// The transport mappings, in the order §6.3 races them.
    transports: Vec<Box<dyn AgentTransport>>,
    broker: PinnedBroker,
    identity: AgentIdentity,
    /// POL-8, held so it can be replaced: a connection that is serving announces the change
    /// (§3.1) and every later connection dials with the new answer.
    served: watch::Sender<Arc<dyn ServedSelectors>>,
    sequence: Arc<dyn Sequence>,
    liveness: Liveness,
    /// Serialises drawing a sequence and sending it, so two procedures running at once cannot
    /// present them out of the order they were drawn (POL-10). Out-of-order arrival would look to
    /// POL-6 like a sequence going backwards and freeze the agent's own identity (SEC-6). The
    /// common path — one [`serve_continuously`](Agent::serve_continuously) loop — never contends.
    claim_order: tokio::sync::Mutex<()>,
}

impl Agent {
    /// Every argument is a value §8.2 names: `broker` is POL-7 and where to reach it, `identity`
    /// is POL-9, `served` decides what a stream may reach (POL-8), and `sequence` supplies the
    /// number each claim carries (POL-10).
    ///
    /// [`Selectors`](crate::Selectors) is the ordinary `served` for an agent with a fixed list.
    pub fn new(
        transports: Vec<Box<dyn AgentTransport>>,
        broker: PinnedBroker,
        identity: AgentIdentity,
        served: Arc<dyn ServedSelectors>,
        sequence: Arc<dyn Sequence>,
        liveness: Liveness,
    ) -> Self {
        Self {
            transports,
            broker,
            identity,
            served: watch::Sender::new(served),
            sequence,
            liveness,
            claim_order: tokio::sync::Mutex::new(()),
        }
    }

    /// Serve a different set from now on, which is what an agent reloading its configuration
    /// calls.
    ///
    /// A connection already serving answers streams from the new set at once and sends another
    /// `ANNOUNCE` without dropping anything (§4.1); one not serving picks the set up when it next
    /// dials. Streams already open are unaffected, a stream being answered when it is opened.
    ///
    /// A set whose encoding exceeds one `ANNOUNCE` frame is refused and nothing changes (§2.4).
    /// Taking it would leave the agent unable to announce what it serves, and §4.1 has no state
    /// for an agent that serves without announcing: it would either have to lie about the set or
    /// stop serving over a change to a set that decides nothing (§3.1).
    pub fn reconfigure(&self, served: Arc<dyn ServedSelectors>) -> Result<(), TunnelError> {
        if !served.announced().fits_one_frame() {
            return Err(TunnelError::fault(
                "the served set exceeds one ANNOUNCE frame",
            ));
        }

        // Send failure means no live `serve` is listening, which the next dial fixes by reading
        // the value rather than the notification.
        let _ = self.served.send(served);

        Ok(())
    }

    pub fn id(&self) -> AgentId {
        self.identity.id
    }

    /// The next claim this agent can make (§2.4). Spends a sequence, so no two calls agree:
    /// [`Sequence::next`] records before it returns (STM-15), and a claim built but never sent has
    /// spent its number all the same.
    fn next_claim(&self) -> Result<Claim, TunnelError> {
        Claim::new(self.identity.id, self.sequence.next()?)
            .ok_or_else(|| TunnelError::fault("a sequence is never 0"))
    }

    /// Bind this agent's private key to its id, using a join token (§5.2).
    ///
    /// Idempotent on purpose (STM-7): the postcondition is that the key is bound, so a broker
    /// that already grants the bare claim ends this successfully without the token being sent.
    /// That covers an agent that died after the broker committed the binding — the token is
    /// spent, and without the shortcut the agent would need a new one from an operator.
    ///
    /// The token goes out after a denial rather than after an invitation, a broker never
    /// signalling that one is wanted (SEC-7).
    pub async fn join(&self, token: &SecretString) -> Result<(), TunnelError> {
        // A joining connection serves nothing, so every stream that arrives on it is refused
        // (STM-19).
        let (serves_nothing, _) = watch::channel(None);
        let mut connection = self.dial(serves_nothing.subscribe()).await?;

        let granted = match self.attempt_admission(&mut *connection.control).await? {
            Access::Granted => Ok(()),
            Access::Denied => {
                let token = SecretSlice::from(token.expose_secret().as_bytes().to_vec());
                control::send_frame(&mut connection.control, &Frame::Join(token)).await?;

                match await_verdict(&mut *connection.control).await? {
                    Access::Granted => Ok(()),
                    Access::Denied => Err(TunnelError::Refused),
                }
            }
        };

        // Dropped rather than closed politely: a conforming broker commits the binding before it
        // grants, so nothing is lost (§5.2, SEC-3).
        connection.closer.close().await;

        granted
    }

    /// Hold this agent reachable until the connection ends or `stopping` is cancelled (§5.1).
    ///
    /// Returning at all means the tunnel is gone: `Ok` means it reached SERVING and has since
    /// ended, and carries how long it stayed there, `Err` that it never got there. Either way
    /// STM-14 bounds the next race, and [`serve_continuously`](Agent::serve_continuously) is that
    /// pacing already written.
    ///
    /// Stopping this way still says the goodbye Appendix B asks for, which dropping the future
    /// would not.
    ///
    /// A denial here is final rather than an invitation to join: tokens belong to
    /// [`join`](Agent::join) alone and are not kept for reconnections (STM-6).
    pub async fn serve(&self, stopping: impl Future<Output = ()>) -> Result<Duration, TunnelError> {
        let mut reconfigured = self.served.subscribe();
        let current = reconfigured.borrow_and_update().clone();

        // The one set that reaches here unchecked is the one `new` was built with;
        // [`reconfigure`](Agent::reconfigure) refuses any later one that cannot be announced.
        let set = current.announced();
        if !set.fits_one_frame() {
            return Err(TunnelError::fault(
                "the served set exceeds one ANNOUNCE frame",
            ));
        }

        // STM-19: every stream is answered by POL-8 in every state, and a stream can arrive
        // before the grant does (TRN-7), so what answers them is published before dialling.
        let (answering, _) = watch::channel(Some(Arc::clone(&current)));
        let mut connection = self.dial(answering.subscribe()).await?;

        match self.attempt_admission(&mut *connection.control).await? {
            Access::Granted => {}
            Access::Denied => {
                connection.closer.close().await;

                return Err(TunnelError::Refused);
            }
        }
        let serving_since = Instant::now();

        // §4.1: on reaching SERVING the agent announces every selector it expects to serve.
        // Informational and unanswered (§3.1).
        control::send_frame(&mut connection.control, &Frame::Announce(set)).await?;

        let Established { control, closer } = connection;
        self.serving(control, &answering, &mut reconfigured, stopping)
            .await;
        let served = serving_since.elapsed();
        closer.close().await;

        Ok(served)
    }

    /// The SERVING loop of §4.1: probes, announcements, and the connection's end. Every way out
    /// of it is the connection ending, which is why it reports nothing.
    async fn serving(
        &self,
        control: Box<dyn Stream>,
        answering: &watch::Sender<Option<Arc<dyn ServedSelectors>>>,
        reconfigured: &mut watch::Receiver<Arc<dyn ServedSelectors>>,
        stopping: impl Future<Output = ()>,
    ) {
        tokio::pin!(stopping);

        let (read, mut write) = tokio::io::split(control);
        let mut frames = Frames::reading(read);
        let mut probes = Probes::new(self.liveness);

        loop {
            tokio::select! {
                frame = frames.next() => {
                    let Some(frame) = frame else {
                        // The connection ended, or the peer left the protocol (STM-3). Either
                        // way the tunnel served and is gone.
                        break;
                    };

                    match frame {
                        Frame::Ping => {
                            if control::send_frame(&mut write, &Frame::Pong).await.is_err() {
                                break;
                            }
                        }
                        Frame::Pong => probes.answered(),
                        // Anything else in SERVING is a protocol error (STM-3); the tunnel
                        // served, so the connection just ends.
                        _ => break,
                    }
                }
                changed = reconfigured.changed() => {
                    if changed.is_err() {
                        // An Agent outlives its serve calls, so the sender never drops; holding
                        // the tunnel is still the right answer.
                        continue;
                    }

                    let next = reconfigured.borrow_and_update().clone();
                    let set = next.announced();

                    // POL-8 answers from the new set at once; the announcement informs the
                    // broker's operator and nothing else (§3.1).
                    let _ = answering.send(Some(next));

                    if control::send_frame(&mut write, &Frame::Announce(set)).await.is_err() {
                        break;
                    }
                }
                () = probes.tick() => {
                    // §6.2: the peer stopped answering, so the connection is treated as ended.
                    if probes.peer_is_silent() {
                        break;
                    }

                    if control::send_frame(&mut write, &Frame::Ping).await.is_err() {
                        break;
                    }

                    probes.sent();
                }
                () = &mut stopping => break,
            }
        }
    }

    /// Hold a tunnel open until `stopping`, dialling again whenever one ends.
    ///
    /// Returns only when asked to stop: an agent that has stopped trying is an agent nobody can
    /// reach, and there is no failure here an operator could fix faster than the agent can retry,
    /// so what went wrong goes to `tracing` instead.
    ///
    /// The pacing is STM-14. Every race waits a delay drawn from [B, 2B), where B starts at 5
    /// seconds, returns there after a stable connection (one that stayed in SERVING for this
    /// agent's K), and otherwise doubles to at most 60 seconds: after a failed race, a denial,
    /// or a connection that ended sooner. The delay is drawn against B as it stood, so the
    /// doubling reaches only later races. An agent cannot learn why it was denied (SEC-7) or
    /// when an unreachable broker returns, so it polls at a rate that decays to a floor.
    pub async fn serve_continuously(&self, stopping: impl Future<Output = ()>) {
        tokio::pin!(stopping);

        let k = self.liveness.probe_interval();
        let mut bound = ReconnectionBound::default();

        loop {
            let served = self.serve(stopping.as_mut()).await;

            // `serve` also returns when it is asked to stop.
            if stopping.as_mut().now_or_never().is_some() {
                return;
            }

            let stable = match served {
                Ok(served) => {
                    tracing::info!(agent = %self.identity.id, "lost the tunnel connection, retrying");

                    served >= k
                }
                Err(error) => {
                    tracing::warn!(agent = %self.identity.id, %error, "could not keep a tunnel connection open");

                    false
                }
            };

            let delay = bound.delay_after(stable);

            // Cancellable, or a stop during a redial delay waits out the whole of it — up to
            // two minutes on an agent that has been failing to connect.
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = &mut stopping => return,
            }
        }
    }

    /// LINKED to CLAIMED (§4.1): the version octet, the one `CLAIM`, and the verdict answering
    /// it.
    async fn attempt_admission(&self, mut control: &mut dyn Stream) -> Result<Access, TunnelError> {
        // Draw and send the claim under one lock so its sequence reaches the broker in the order
        // it was drawn, even if another procedure on this agent is claiming at the same time
        // (POL-10, SEC-6). Held only across the version octet and the CLAIM frame.
        {
            let _order = self.claim_order.lock().await;
            let claim = self.next_claim()?;

            wire::write_version(&mut control)
                .await
                .map_err(TunnelError::unexpected)?;
            control::send_frame(&mut control, &Frame::Claim(claim)).await?;
        }

        await_verdict(control).await
    }

    /// One race over the mappings and the broker's addresses (§6.3). Broker-opened streams are
    /// served from here on, by `answering` in whatever state the connection reaches (STM-19).
    async fn dial(
        &self,
        answering: watch::Receiver<Option<Arc<dyn ServedSelectors>>>,
    ) -> Result<Established, TunnelError> {
        let AgentConnection {
            control,
            incoming,
            closer,
        } = race::race(&self.transports, &self.broker, &self.identity.keys).await?;
        tokio::spawn(streams::serve_streams(incoming, answering));

        Ok(Established { control, closer })
    }
}

/// This agent's own identity (§1.2, POL-9): the identifier it claims, and the keypair it proves
/// possession of. The identifier is public; the private half never leaves the host.
pub struct AgentIdentity {
    pub id: AgentId,
    pub keys: Identity,
}

/// A connection the agent holds: its control stream, and the handle that ends it. Tunnel streams
/// are not here, `dial` having handed them to the task that serves them.
struct Established {
    control: Box<dyn Stream>,
    closer: Box<dyn Closer>,
}

/// The `ACCESS` answering an admission attempt, with probes answered meanwhile and the wait
/// bounded (§3.2).
async fn await_verdict(control: &mut dyn Stream) -> Result<Access, TunnelError> {
    match tokio::time::timeout(ANSWER_WAIT, control::recv_frame(control)).await {
        Err(_elapsed) => Err(TunnelError::fault("no answer to an admission attempt")),
        Ok(Ok(Frame::Access(access))) => Ok(access),
        // A read failure is the connection ending; any other frame is a protocol error (STM-3).
        // Either way this returns and the caller closes the connection.
        Ok(Err(_) | Ok(_)) => Err(TunnelError::NotConnected),
    }
}

/// STM-14's reconnection bound B, which paces the races of
/// [`serve_continuously`](Agent::serve_continuously).
struct ReconnectionBound(Duration);

impl Default for ReconnectionBound {
    fn default() -> Self {
        Self(RECONNECTION_BOUND)
    }
}

impl ReconnectionBound {
    /// The wait before the next race, drawn uniformly from `[B, 2B)`. A stable connection returns
    /// B to its start before the draw; anything else doubles B after it, to at most the ceiling,
    /// so the doubling reaches only later races.
    ///
    /// Randomised rather than flat because B starts at the same value on every agent. A fixed
    /// wait would make a whole fleet redial in step after a broker restart, and the agents with
    /// most reason to reconnect are the ones that would arrive together.
    fn delay_after(&mut self, stable: bool) -> Duration {
        use rand::RngExt as _;

        if stable {
            self.0 = RECONNECTION_BOUND;
        }
        let delay = rand::rng().random_range(self.0..self.0 * 2);
        if !stable {
            self.0 = (self.0 * 2).min(RECONNECTION_CEILING);
        }

        delay
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::identity::InMemoryRollbackIndex;
    use crate::policy::Selectors;
    use crate::wire::Selector;

    fn agent_serving(served: Arc<dyn ServedSelectors>) -> Agent {
        struct UnusedSequence;

        impl Sequence for UnusedSequence {
            fn next(&self) -> Result<u64, TunnelError> {
                unreachable!("this agent never claims")
            }
        }

        Agent::new(
            Vec::new(),
            PinnedBroker {
                address: "broker.invalid:8443".parse().unwrap(),
                root: Identity::generate().unwrap().public_key().fingerprint(),
                rollback: Arc::new(InMemoryRollbackIndex::default()),
            },
            AgentIdentity {
                id: AgentId::new(uuid::Uuid::now_v7()),
                keys: Identity::generate().unwrap(),
            },
            served,
            Arc::new(UnusedSequence),
            Liveness::default(),
        )
    }

    fn long_unix_selectors(count: usize) -> Arc<Selectors> {
        Arc::new(Selectors::new((0..count).map(|path_number| {
            Selector::unix(format!("/{}/{path_number}", "x".repeat(4000)))
                .unwrap()
                .into()
        })))
    }

    #[test]
    fn reconfigure_with_set_within_one_frame_returns_ok() {
        let agent = agent_serving(long_unix_selectors(1));

        let reconfigured = agent.reconfigure(long_unix_selectors(2));

        assert_matches!(reconfigured, Ok(()));
    }

    // 17 paths of 4000 octets do not fit in the 65535 octets of one frame.
    #[test]
    fn reconfigure_with_set_beyond_one_frame_returns_error() {
        let agent = agent_serving(long_unix_selectors(1));

        let reconfigured = agent.reconfigure(long_unix_selectors(17));

        assert_matches!(reconfigured, Err(TunnelError::Unexpected(_)));
    }

    #[test]
    fn delay_after_consecutive_failures_doubles_up_to_ceiling() {
        let mut bound = ReconnectionBound::default();

        let delays: Vec<Duration> = (0..6).map(|_| bound.delay_after(false)).collect();

        for (delay, from) in delays.into_iter().zip([5, 10, 20, 40, 60, 60]) {
            let from = Duration::from_secs(from);
            assert!(
                (from..from * 2).contains(&delay),
                "{delay:?} not in [{from:?}, 2·{from:?})"
            );
        }
    }

    #[test]
    fn delay_after_stable_connection_returns_to_start() {
        let mut bound = ReconnectionBound::default();
        for _ in 0..3 {
            bound.delay_after(false);
        }

        let delay = bound.delay_after(true);

        assert!((RECONNECTION_BOUND..RECONNECTION_BOUND * 2).contains(&delay));
    }
}
