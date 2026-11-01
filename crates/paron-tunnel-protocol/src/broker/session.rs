//! The broker side of one connection, from the moment a transport hands it over until it ends.
//!
//! This is where §4.2's state machine actually runs: read the version octet, answer the claim,
//! answer a join if one follows, then serve — carrying the limits of TRN-10 and the tunnel streams
//! consumers open along the way.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt as _;
use tokio::time::Instant;

use crate::broker::{Broker, ClaimResponse};
use crate::connected_agents::Tunnel;
use crate::control::{Frames, Probes};
use crate::error::TunnelError;
use crate::identity::{AgentId, PublicKey};
use crate::transport::{BrokerConnection, Closer, Stream, StreamOpener};
use crate::wire::{self, Access, Frame, FrameError, Selector, StreamAnswer};

/// TRN-10: `ACCESS`(Denied) goes out no earlier than this after the octets it answers, fixed at
/// receipt, before any policy is invoked.
pub(crate) const DENIAL_DELAY: Duration = Duration::from_secs(1);

/// TRN-6: how long the broker waits for a stream answer before reporting the open as refused.
const STREAM_ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// The most control frames an admitted connection may deliver in one second before processing is
/// stalled (TRN-10's rate bound). Stalling rather than ending, so the transport's flow control
/// turns the stall into backpressure.
///
/// Derived from what a conforming agent sends: its own `PING` each K, a `PONG` for each of the
/// broker's, and an `ANNOUNCE` when its served set changes. At the 5-second floor on K (§6.2)
/// that is 0.4 frames a second, so this leaves a fortyfold margin for announcement bursts while
/// still cutting an abusive peer off well short of what a loop can emit.
const FRAMES_PER_SECOND: u32 = 16;

/// Why a connection driver stopped.
#[derive(Debug)]
enum Ended {
    /// The transport reported the connection gone, or the peer spoke something that is not this
    /// protocol (STM-3).
    Connection,
    /// The peer stopped answering probes (§6.2).
    PeerSilent,
}

impl From<FrameError> for Ended {
    fn from(_: FrameError) -> Self {
        Self::Connection
    }
}

impl From<std::io::Error> for Ended {
    fn from(_: std::io::Error) -> Self {
        Self::Connection
    }
}

/// One admitted connection, as the consumer surface reaches it. No selector gates the open: the
/// announced set is informational (§3.1) and POL-8 on the agent is the sole authority over what
/// a stream reaches.
pub(crate) struct ConnectionTunnel {
    opener: Arc<dyn StreamOpener>,
    closer: Arc<dyn Closer>,
}

impl ConnectionTunnel {
    /// Open a stream, name the selector, and wait for the agent's answer (§2.5, §4.3).
    async fn open_and_await_answer(
        &self,
        selector: &Selector,
    ) -> Result<Box<dyn Stream>, TunnelError> {
        let mut stream = self.opener.open().await?;
        wire::write_stream_preamble(&mut stream, selector)
            .await
            .map_err(TunnelError::unexpected)?;

        match wire::read_stream_answer(&mut stream).await {
            Ok(StreamAnswer::Accept) => Ok(stream),
            Ok(StreamAnswer::Refuse) => Err(TunnelError::Refused),
            // A stream-level protocol error is the stream's alone (§2.5), reported under STM-8.
            Err(error) => Err(TunnelError::unexpected(error)),
        }
    }
}

#[async_trait]
impl Tunnel for ConnectionTunnel {
    async fn open(&self, selector: &Selector) -> Result<Box<dyn Stream>, TunnelError> {
        // TRN-6 bounds the whole open, not just the answer: a mapping at its stream ceiling makes
        // taking the stream wait, so a bound around the answer alone would leave a consumer
        // waiting on a place that may never free. Giving up early discloses nothing, a refusal
        // already reading the same as a service that is down (SEC-7).
        let opening = self.open_and_await_answer(selector);

        match tokio::time::timeout(STREAM_ANSWER_TIMEOUT, opening).await {
            Err(_elapsed) => Err(TunnelError::Refused),
            Ok(opened) => opened,
        }
    }

    async fn close(&self) {
        self.closer.close().await;
    }
}

/// Speak the broker's half of the protocol on one accepted connection, until it ends.
///
/// This is the whole broker side of the wire protocol and the only thing a transport mapping has
/// to call: accept a connection, hand it over as a [`BrokerConnection`], and this runs §4.2's
/// state machine over it until there is nothing left to run.
///
/// `slot` is the connection's place in the unadmitted-connection bound, taken by the transport
/// when the connection arrived (TRN-10). It is given back here, at admission.
pub async fn serve_connection(
    broker: Arc<Broker>,
    connection: BrokerConnection,
    slot: crate::broker::Slot,
) {
    AgentSession::new(broker, connection).run(slot).await;
}

/// One connection's worth of broker state, from the transport handing it over to the end.
///
/// A type rather than a chain of functions because every step reads the same things: the broker,
/// the control stream and the key the peer proved. The states of §4.2 are the methods, in the
/// order the wire reaches them.
struct AgentSession {
    broker: Arc<Broker>,
    agent_public_key: PublicKey,
    control: Box<dyn Stream>,
    opener: Arc<dyn StreamOpener>,
    closer: Arc<dyn Closer>,
}

impl AgentSession {
    fn new(broker: Arc<Broker>, connection: BrokerConnection) -> Self {
        let BrokerConnection {
            agent_address: _,
            agent_public_key,
            control,
            opener,
            closer,
        } = connection;

        Self {
            broker,
            agent_public_key,
            control,
            opener,
            closer,
        }
    }

    async fn run(mut self, slot: crate::broker::Slot) {
        let broker = Arc::clone(&self.broker);

        // TRN-10: a connection that has proven nothing does not get to exist indefinitely.
        let admitted = tokio::time::timeout(broker.limits().admission_timeout, self.admit()).await;

        let Ok(Ok((agent, granted))) = admitted else {
            self.drain().await;
            self.closer.close().await;

            return;
        };

        // TRN-10 bounds connections that have *not* been admitted, so the place goes back here
        // rather than at the end: an admitted connection is bounded by STM-11 instead, which
        // holds one per agent.
        drop(slot);

        // Held across the serving phase, which consumes the driver: the control stream is
        // split there and does not come back.
        let closer = Arc::clone(&self.closer);

        let ended = match granted {
            Admitted::Serving => self.serve_registered(agent).await,
            Admitted::Joined => self.await_release(agent).await,
        };

        tracing::debug!(%agent, ?ended, "agent connection ended");
        closer.close().await;
    }

    /// POL-3, §6.1: the registration is established at the grant of a `CLAIM`, before
    /// `ACCESS`(Granted) goes out, and replaces whatever connection held the agent's place
    /// (STM-11). It stands until this returns.
    async fn serve_registered(mut self, agent: AgentId) -> Ended {
        let tunnel = Arc::new(ConnectionTunnel {
            opener: Arc::clone(&self.opener),
            closer: Arc::clone(&self.closer),
        });
        let registration = self
            .broker
            .connect_agent(agent, tunnel as Arc<dyn Tunnel>)
            .await;

        let ended = match self.send(&Frame::Access(Access::Granted)).await {
            Ok(()) => self.serve(agent, Admitted::Serving).await,
            Err(ended) => ended,
        };
        drop(registration);

        ended
    }

    /// A grant answering a `JOIN` registers nothing: JOINED awaits the agent's release (§4.2,
    /// §6.1). A conforming agent releases on the grant, so a connection still here after the
    /// unadmitted bound is parked, not joining (TRN-10).
    async fn await_release(mut self, agent: AgentId) -> Ended {
        if let Err(ended) = self.send(&Frame::Access(Access::Granted)).await {
            return ended;
        }

        let released = tokio::time::timeout(
            self.broker.limits().admission_timeout,
            self.serve(agent, Admitted::Joined),
        )
        .await;

        released.unwrap_or_else(|_elapsed| {
            tracing::debug!(%agent, "closed a enrollment connection not terminated by the agent");

            Ended::Connection
        })
    }

    /// LINKED to the grant (§4.2): the version octet, one `CLAIM`, at most one `JOIN`. The
    /// `ACCESS`(Granted) itself is the caller's to send — after POL-3 has registered the
    /// connection where a `CLAIM` was granted (§6.1); every denial is answered here.
    async fn admit(&mut self) -> Result<(AgentId, Admitted), Ended> {
        let version = wire::read_version(&mut self.control).await?;
        // TRN-10 fixes the moment at receipt of the octet, so it is stamped once the read has
        // one: stamping before the read would start the delay while the peer was still sending,
        // and a peer slower than the delay would be answered at once.
        let received = Instant::now();

        if version != wire::VERSION {
            // VER-2, VER-4: denied as this document encodes it, reported to the operator, and
            // the connection then ends. The delay is fixed at receipt of the octet (TRN-10).
            tracing::warn!(
                agent_speaks = version,
                server_speaks = wire::VERSION,
                "agent connection refused: unsupported protocol version"
            );
            self.deny_at(received).await?;

            return Err(Ended::Connection);
        }

        let (Frame::Claim(claim), received) = self.recv().await? else {
            return Err(Ended::Connection);
        };

        match self
            .broker
            .answer_claim(&claim, &self.agent_public_key)
            .await
        {
            ClaimResponse::Granted(agent) => Ok((agent, Admitted::Serving)),
            ClaimResponse::Denied(agent) => self.await_join(agent, received).await,
        }
    }

    /// A denial does not end the connection: the agent may proceed to `JOIN`, answered under
    /// the identifier and proven key of the claim (§4.2).
    async fn await_join(
        &mut self,
        agent: AgentId,
        denied_at: Instant,
    ) -> Result<(AgentId, Admitted), Ended> {
        self.deny_at(denied_at).await?;

        let (Frame::Join(token), received) = self.recv().await? else {
            return Err(Ended::Connection);
        };

        match self
            .broker
            .answer_join(agent, &self.agent_public_key, &token)
            .await
        {
            Access::Granted => Ok((agent, Admitted::Joined)),
            Access::Denied => {
                // A denied `JOIN` is the second and last admission attempt this connection
                // gets (§4.2): nothing further is permitted on it.
                self.deny_at(received).await?;

                Err(Ended::Connection)
            }
        }
    }

    /// SERVING, and JOINED awaiting the agent's release (§4.2): announcements where they are
    /// permitted, probes, and nothing else.
    async fn serve(self, agent: AgentId, state: Admitted) -> Ended {
        let liveness = self.broker.liveness();
        // Consumes the driver: the control stream is split between a reader task and this loop,
        // and nothing after this needs it whole again.
        let (read, mut write) = tokio::io::split(self.control);
        let mut frames = Frames::reading(read);
        let mut probes = Probes::new(liveness);
        let mut throttle = Throttle::new();

        loop {
            let frame = tokio::select! {
                frame = frames.next() => match frame {
                    Some(frame) => frame,
                    // The reader hit end-of-stream or a protocol error (STM-3).
                    None => break Ended::Connection,
                },
                () = probes.tick() => {
                    if probes.peer_is_silent() {
                        break Ended::PeerSilent;
                    }

                    match wire::write_frame(&mut write, &Frame::Ping).await {
                        Ok(()) => probes.sent(),
                        Err(_) => break Ended::Connection,
                    }

                    continue;
                }
            };

            throttle.pace().await;

            match frame {
                Frame::Ping => {
                    if wire::write_frame(&mut write, &Frame::Pong).await.is_err() {
                        break Ended::Connection;
                    }
                }
                Frame::Pong => probes.answered(),
                Frame::Announce(set) if state == Admitted::Serving => {
                    // STM-13: the set, the agent it came from and the host designator serving it
                    // reach the operator, and nothing else is derived from any of them — it is
                    // not answered, and it gates no stream (§3.1).
                    //
                    // The designator is a constant here because PTP/1 defines exactly one and a
                    // frame naming any other is malformed (§2.4), so it is stated rather than
                    // carried: an `AnnounceSet` that exists has already been checked against it.
                    let services: Vec<String> = set.iter().map(ToString::to_string).collect();
                    tracing::debug!(
                        %agent,
                        ?services,
                        host = "the agent's own host",
                        "an agent announced the services it exposes"
                    );
                }
                // A second `CLAIM` or `JOIN`, or anything else, here (STM-3, STM-10).
                _ => break Ended::Connection,
            }
        }
    }

    /// Receive one frame, answering probes transparently, and note when it arrived: denial
    /// delays are fixed at receipt (TRN-10).
    async fn recv(&mut self) -> Result<(Frame, Instant), Ended> {
        loop {
            let frame = wire::read_frame(&mut self.control).await?;
            let received = Instant::now();

            match frame {
                Frame::Ping => wire::write_frame(&mut self.control, &Frame::Pong).await?,
                Frame::Pong => {}
                frame => return Ok((frame, received)),
            }
        }
    }

    async fn send(&mut self, frame: &Frame) -> Result<(), Ended> {
        wire::write_frame(&mut self.control, frame).await?;

        Ok(())
    }

    /// `ACCESS`(Denied), no earlier than [`DENIAL_DELAY`] after `received` (TRN-10). The moment
    /// was fixed before any policy ran, so the answer's timing carries nothing of what the
    /// policy cost, while the policy stayed inside the delay (§12.8).
    async fn deny_at(&mut self, received: Instant) -> Result<(), Ended> {
        tokio::time::sleep_until(received + DENIAL_DELAY).await;
        wire::write_frame(&mut self.control, &Frame::Access(Access::Denied)).await?;

        Ok(())
    }

    /// Let what was written reach the peer before the transport is torn down: an abrupt close
    /// discards data still in flight, and a denial is the last thing this connection says.
    ///
    /// Exits at once when the peer is already gone, so an ordinary disconnect pays nothing.
    async fn drain(&mut self) {
        let _ = self.control.shutdown().await;

        let mut sink = [0u8; 256];
        let draining = async {
            loop {
                match tokio::io::AsyncReadExt::read(&mut self.control, &mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), draining).await;
    }
}

/// Where a grant leaves a connection (§4.2). A `CLAIM` registers it and it serves; a `JOIN`
/// registers nothing and it waits to be released (§6.1).
///
/// It also settles what an `ANNOUNCE` means: reported while SERVING (STM-13), a protocol error in
/// JOINED, where nothing further is permitted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Admitted {
    Serving,
    Joined,
}

/// TRN-10's bound on the rate frames from an admitted connection are processed at.
struct Throttle {
    window: Instant,
    spent: u32,
}

impl Throttle {
    fn new() -> Self {
        Self {
            window: Instant::now(),
            spent: 0,
        }
    }

    /// Waits out the window once a connection has spent its frames for this second.
    async fn pace(&mut self) {
        let now = Instant::now();

        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.spent = 0;
        }

        self.spent += 1;

        if self.spent > FRAMES_PER_SECOND {
            tokio::time::sleep_until(self.window + Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn pace_within_rate_does_not_wait() {
        let mut throttle = Throttle::new();
        let started_at = Instant::now();

        for _ in 0..FRAMES_PER_SECOND {
            throttle.pace().await;
        }

        assert_eq!(started_at.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn pace_past_rate_waits_until_window_ends() {
        let mut throttle = Throttle::new();
        let started_at = Instant::now();
        for _ in 0..FRAMES_PER_SECOND {
            throttle.pace().await;
        }

        throttle.pace().await;

        assert_eq!(started_at.elapsed(), Duration::from_secs(1));
    }
}
