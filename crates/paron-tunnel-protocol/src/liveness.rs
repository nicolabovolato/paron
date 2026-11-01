//! Liveness (RFC §6.2, TRN-8).
//!
//! A probe is a `PING` frame on the control stream, answered by `PONG`. §6.2 counts probes that
//! went *unanswered*: a write that succeeds proves nothing, because a dead peer's kernel absorbs
//! writes while it has buffer.
//!
//! Both values belong to one role and neither is exchanged with the peer. Both roles probe and
//! both answer, so what a role concludes rests on its own probing (§6.2).

use std::time::Duration;

/// **K** — the probe interval (§6.2).
pub const PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// **M** — the probes that may go unanswered before a peer is called gone (§6.2).
pub const MISSED_PROBES: usize = 3;

/// The shortest K a deployment may choose (§6.2). Probing is a cost imposed on the peer, and a
/// fleet multiplies a small number.
///
/// The conformance suite goes below it, being the one setting where both roles are under one
/// tester's control (TRN-8).
pub const MINIMUM_PROBE_INTERVAL: Duration = Duration::from_secs(5);

/// The longest K a deployment may choose (§6.2): (M+1)*K is how long a broker may offer a
/// consumer an agent that is gone, so a role choosing K is not choosing only for itself.
///
/// It rarely binds. A NAT need hold a UDP mapping for only two minutes ([RFC 4787] REQ-5), so a
/// metered link saves nothing by probing more slowly than its mapping survives: it pays a whole
/// handshake and a spent sequence each time one expires, against the 3 octets a probe costs.
///
/// [RFC 4787]: https://www.rfc-editor.org/rfc/rfc4787
pub const MAXIMUM_PROBE_INTERVAL: Duration = Duration::from_mins(5);

/// The most unanswered probes a deployment may tolerate (§6.2).
pub const MOST_MISSED_PROBES: usize = 10;

const _: () = assert!(PROBE_INTERVAL.as_nanos() >= MINIMUM_PROBE_INTERVAL.as_nanos());
const _: () = assert!(PROBE_INTERVAL.as_nanos() <= MAXIMUM_PROBE_INTERVAL.as_nanos());
const _: () = assert!(MISSED_PROBES <= MOST_MISSED_PROBES);

/// One role's K and M (§6.2). [`Liveness::default`] is what a role with no reason to choose
/// otherwise uses: a role behind a NAT that expires mappings quickly shortens K, one on a metered
/// link lengthens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Liveness {
    probe_interval: Duration,
    missed_probes: usize,
}

impl Default for Liveness {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Liveness {
    /// What a role with no reason to choose otherwise uses, as a constant, so a deployment can
    /// name it where `Default::default` cannot be called.
    pub const DEFAULT: Self = Self {
        probe_interval: PROBE_INTERVAL,
        missed_probes: MISSED_PROBES,
    };

    /// `None` outside the bounds of §6.2, which requires the rejection at startup rather than on
    /// the connection that first suffers from it. The fields are private and this is the only way
    /// in, so a role cannot hold values outside them.
    pub fn new(probe_interval: Duration, missed_probes: usize) -> Option<Self> {
        let within_bounds = probe_interval >= MINIMUM_PROBE_INTERVAL
            && probe_interval <= MAXIMUM_PROBE_INTERVAL
            && missed_probes <= MOST_MISSED_PROBES;

        within_bounds.then_some(Self {
            probe_interval,
            missed_probes,
        })
    }

    /// The same, below [`MINIMUM_PROBE_INTERVAL`], for the conformance mode of TRN-8. CNF-7 and
    /// CNF-8 wait out a detection bound apiece, which at deployment timings would take minutes.
    #[cfg(test)]
    pub(crate) fn for_conformance(probe_interval: Duration, missed_probes: usize) -> Self {
        Self {
            probe_interval,
            missed_probes,
        }
    }

    pub fn probe_interval(&self) -> Duration {
        self.probe_interval
    }

    pub fn missed_probes(&self) -> usize {
        self.missed_probes
    }

    /// When a peer that has stopped answering is noticed: the M tolerated probes plus the one
    /// that gives up, (M+1)*K (§6.2). §6.2 defines no idle timeout beside it and needs none, a
    /// peer that sends nothing at all being the same case as one that stops answering.
    pub fn detection_bound(&self) -> Duration {
        self.probe_interval * (u32::try_from(self.missed_probes).unwrap_or(u32::MAX) + 1)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn new_at_minimum_probe_interval_returns_liveness() {
        let liveness = Liveness::new(MINIMUM_PROBE_INTERVAL, MISSED_PROBES);

        assert_matches!(liveness, Some(_));
    }

    #[test]
    fn new_at_maximum_probe_interval_returns_liveness() {
        let liveness = Liveness::new(MAXIMUM_PROBE_INTERVAL, MISSED_PROBES);

        assert_matches!(liveness, Some(_));
    }

    #[test]
    fn new_at_most_missed_probes_returns_liveness() {
        let liveness = Liveness::new(PROBE_INTERVAL, MOST_MISSED_PROBES);

        assert_matches!(liveness, Some(_));
    }

    #[test]
    fn new_below_minimum_probe_interval_returns_none() {
        let liveness = Liveness::new(MINIMUM_PROBE_INTERVAL / 2, MISSED_PROBES);

        assert_eq!(liveness, None);
    }

    #[test]
    fn new_above_maximum_probe_interval_returns_none() {
        let liveness = Liveness::new(
            MAXIMUM_PROBE_INTERVAL + Duration::from_secs(1),
            MISSED_PROBES,
        );

        assert_eq!(liveness, None);
    }

    #[test]
    fn new_above_most_missed_probes_returns_none() {
        let liveness = Liveness::new(PROBE_INTERVAL, MOST_MISSED_PROBES + 1);

        assert_eq!(liveness, None);
    }

    #[test]
    fn detection_bound_default_counts_probe_that_gives_up() {
        let bound = Liveness::default().detection_bound();

        assert_eq!(bound, Duration::from_mins(1));
    }
}
