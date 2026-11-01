//! Reading and writing the control stream, which both roles do the same way.
//!
//! Every connection has exactly one control stream, and it carries two kinds of traffic: the
//! frames of RFC §2.4, and the `PING`/`PONG` probes that tell each side the other is still there
//! (§6.2). Both roles need both, so both live here rather than being written twice.

use tokio::sync::mpsc;

use crate::error::TunnelError;
use crate::liveness::Liveness;
use crate::transport::Stream;
use crate::wire::{self, Frame, FrameError};

/// How many whole frames a reader may buffer ahead of the loop it feeds. Small on purpose: a peer
/// that outruns the loop fills this and then blocks the reader, which the transport turns into
/// backpressure.
const FRAMES_AHEAD: usize = 16;

/// The frames arriving on one control stream, read by a task of its own.
///
/// The task exists for cancel-safety: `read_frame` awaits several times, so a loop that raced it
/// against a timer would drop a partially-read frame and end a healthy connection. Receiving from
/// the channel instead is cancel-safe, and the task keeps the partial-read state.
///
/// It ends with the connection, or when this is dropped — including a drop the code did not spell
/// out, such as a `serve` loop abandoned by a timeout.
pub(crate) struct Frames {
    frames: mpsc::Receiver<Frame>,
    reader: tokio::task::JoinHandle<()>,
}

impl Frames {
    pub(crate) fn reading(mut read: tokio::io::ReadHalf<Box<dyn Stream>>) -> Self {
        let (sender, frames) = mpsc::channel(FRAMES_AHEAD);

        let reader = tokio::spawn(async move {
            // Ends, dropping its sender, on the first read error or malformed frame (STM-3), or
            // once the loop it feeds has gone.
            while let Ok(frame) = wire::read_frame(&mut read).await {
                if sender.send(frame).await.is_err() {
                    break;
                }
            }
        });

        Self { frames, reader }
    }

    /// The next frame, or `None` once the connection has ended or the peer left the protocol
    /// (STM-3). Cancel-safe, so it belongs in a `select!`.
    pub(crate) async fn next(&mut self) -> Option<Frame> {
        self.frames.recv().await
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// §6.2's probe bookkeeping for one role: a `PING` each K, and the connection called gone once
/// more than M of them have gone unanswered.
pub(crate) struct Probes {
    interval: tokio::time::Interval,
    tolerated: usize,
    unanswered: usize,
}

impl Probes {
    pub(crate) fn new(liveness: Liveness) -> Self {
        let mut interval = tokio::time::interval(liveness.probe_interval());
        // The first tick of a tokio interval is immediate; probing starts one K in.
        interval.reset();
        // Delay, not Burst: a loop held up past a tick would otherwise be handed the missed
        // ticks back to back, sending a flurry of probes that count as unanswered together and
        // trip the detection bound on a peer that is merely slow. Delay keeps them K apart.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        Self {
            interval,
            tolerated: liveness.missed_probes(),
            unanswered: 0,
        }
    }

    /// Resolves when the next probe is due.
    pub(crate) async fn tick(&mut self) {
        self.interval.tick().await;
    }

    /// Whether more than M probes have gone unanswered, which is the (M+1)*K detection bound of
    /// §6.2 reached: the peer is treated as gone.
    pub(crate) fn peer_is_silent(&self) -> bool {
        self.unanswered > self.tolerated
    }

    /// Account for a probe just sent. Only a `PONG` clears the count: §6.2 determines liveness
    /// from received messages, a write that succeeds proving nothing.
    pub(crate) fn sent(&mut self) {
        self.unanswered += 1;
    }

    pub(crate) fn answered(&mut self) {
        self.unanswered = 0;
    }
}

/// The next frame that is not a probe, answering any `PING` that arrives meanwhile (§6.2). Used
/// where a role waits for one specific answer rather than serving a loop.
pub(crate) async fn recv_frame(mut control: &mut dyn Stream) -> Result<Frame, FrameError> {
    loop {
        match wire::read_frame(&mut control).await? {
            Frame::Ping => wire::write_frame(&mut control, &Frame::Pong).await?,
            Frame::Pong => {}
            frame => return Ok(frame),
        }
    }
}

pub(crate) async fn send_frame(
    control: &mut (impl tokio::io::AsyncWrite + Unpin),
    frame: &Frame,
) -> Result<(), TunnelError> {
    wire::write_frame(control, frame)
        .await
        .map_err(|_| TunnelError::NotConnected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn peer_is_silent_at_missed_probes_returns_false() {
        let liveness = Liveness::default();
        let mut probes = Probes::new(liveness);
        for _ in 0..liveness.missed_probes() {
            probes.sent();
        }

        let silent = probes.peer_is_silent();

        assert!(!silent);
    }

    #[tokio::test(start_paused = true)]
    async fn peer_is_silent_past_missed_probes_returns_true() {
        let liveness = Liveness::default();
        let mut probes = Probes::new(liveness);
        for _ in 0..=liveness.missed_probes() {
            probes.sent();
        }

        let silent = probes.peer_is_silent();

        assert!(silent);
    }

    #[tokio::test(start_paused = true)]
    async fn answered_after_missed_probes_clears_silence() {
        let liveness = Liveness::default();
        let mut probes = Probes::new(liveness);
        for _ in 0..=liveness.missed_probes() {
            probes.sent();
        }

        probes.answered();

        assert!(!probes.peer_is_silent());
    }
}
