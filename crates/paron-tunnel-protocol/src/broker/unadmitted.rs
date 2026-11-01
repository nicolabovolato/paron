//! How many connections may be waiting to be admitted at once (TRN-10).
//!
//! A connection takes a slot the moment a transport hands it over, and gives it back when it is
//! admitted or when it goes away. Admission is the handover point: after that a connection is
//! bounded by STM-11 instead, which allows one per agent, so it no longer needs a slot here.
//!
//! Without this, anyone who can open TCP or QUIC connections could tie up a broker's memory
//! without ever proving they are anybody.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;

use parking_lot::Mutex;

/// What one source is, for counting: an IPv6 address folded to its /64 prefix, an IPv4 address as
/// it is.
///
/// An ISP assigns at least a /64 to each subscriber, so a peer can use any address within it and
/// counting whole addresses would hand it a new share for each one. IPv4 addresses are assigned
/// individually, so they are counted as they are. Canonicalised first, or an IPv4 address arriving
/// mapped into IPv6 would fold to the prefix every mapped address shares.
fn one_source(source: IpAddr) -> IpAddr {
    match source.to_canonical() {
        IpAddr::V4(address) => IpAddr::V4(address),
        IpAddr::V6(address) => {
            let mut octets = address.octets();

            octets[8..].fill(0);

            IpAddr::V6(Ipv6Addr::from(octets))
        }
    }
}

/// Three limits, which together are what TRN-10 asks for: an overall capacity, a cap on how much
/// of it any one source address may take, and — past halfway — an increasing chance of being
/// turned away regardless.
pub(crate) struct UnadmittedConnections {
    capacity: usize,
    per_source: usize,
    in_use: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl UnadmittedConnections {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            // A quarter of the total. TRN-10 requires that no single source can fill the whole
            // thing, however many connections it opens.
            per_source: (capacity / 4).max(1),
            in_use: Arc::default(),
        }
    }

    /// More than half full.
    ///
    /// Two things start happening at this point: we begin turning some connections away at random
    /// (see [`take_slot`](UnadmittedConnections::take_slot)), and a transport that can check a
    /// peer's address before committing memory to it starts doing so — QUIC Retry, in our case
    /// (TRN-10).
    pub(crate) fn past_half_capacity(&self) -> bool {
        let in_use = self.in_use.lock();
        let total: usize = in_use.values().sum();

        total > self.capacity / 2
    }

    /// Take a slot for a connection from `source`, or `None` to turn it away.
    ///
    /// Past half of the capacity, some connections are refused at random, and the chance of
    /// refusal increases as more connections wait (TRN-10). Without this, the sources that fill
    /// the capacity first keep all of it, and a later connection has no chance.
    ///
    /// One uniform draw from half of the capacity up to the capacity sets the limit: a connection
    /// is refused when more connections than the draw wait. A capacity of 0 gives nothing to
    /// draw, so each connection is refused.
    pub fn take_slot(&self, source: IpAddr) -> Option<Slot> {
        use rand::seq::IteratorRandom as _;

        let refuse_above = (self.capacity / 2..self.capacity).choose(&mut rand::rng())?;

        self.take_slot_unless_above(source, refuse_above)
    }

    /// The lock covers reading the total and taking the slot together. If it did not, two
    /// connections arriving at the same moment could both see room and both be let in. Nothing
    /// inside the lock awaits anything, so it is held for a handful of instructions and cannot
    /// stall other connections.
    fn take_slot_unless_above(&self, source: IpAddr, refuse_above: usize) -> Option<Slot> {
        let source = one_source(source);
        let mut in_use = self.in_use.lock();
        let total: usize = in_use.values().sum();

        if total > refuse_above {
            return None;
        }

        if in_use.get(&source).copied().unwrap_or(0) >= self.per_source {
            return None;
        }

        *in_use.entry(source).or_default() += 1;

        Some(Slot {
            in_use: Arc::clone(&self.in_use),
            source,
        })
    }
}

/// One unadmitted connection's place, given back on drop: at admission, or with the connection.
///
/// Owned rather than borrowed, because a transport takes it the moment a connection arrives and
/// the connection outlives the accept loop that took it.
pub struct Slot {
    in_use: Arc<Mutex<HashMap<IpAddr, usize>>>,
    source: IpAddr,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut in_use = self.in_use.lock();

        if let Some(count) = in_use.get_mut(&self.source) {
            *count -= 1;

            if *count == 0 {
                in_use.remove(&self.source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_address(last_octet: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last_octet])
    }

    #[test]
    fn take_slot_at_capacity_returns_none() {
        let connections = UnadmittedConnections::with_capacity(2);
        let _slots = [1, 2].map(|last_octet| connections.take_slot(source_address(last_octet)));

        let slot = connections.take_slot(source_address(3));

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_with_capacity_zero_returns_none() {
        let connections = UnadmittedConnections::with_capacity(0);

        let slot = connections.take_slot(source_address(1));

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_unless_above_at_half_capacity_with_lowest_draw_returns_slot() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots = [1, 2, 3, 4]
            .map(|last_octet| connections.take_slot_unless_above(source_address(last_octet), 7));

        let slot = connections.take_slot_unless_above(source_address(5), 4);

        assert!(slot.is_some());
    }

    #[test]
    fn take_slot_unless_above_past_half_capacity_with_draw_below_total_returns_none() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots = [1, 2, 3, 4, 5]
            .map(|last_octet| connections.take_slot_unless_above(source_address(last_octet), 7));

        let slot = connections.take_slot_unless_above(source_address(6), 4);

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_unless_above_past_half_capacity_with_draw_at_total_returns_slot() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots = [1, 2, 3, 4, 5]
            .map(|last_octet| connections.take_slot_unless_above(source_address(last_octet), 7));

        let slot = connections.take_slot_unless_above(source_address(6), 5);

        assert!(slot.is_some());
    }

    #[test]
    fn take_slot_unless_above_at_capacity_with_highest_draw_returns_none() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots = [1, 2, 3, 4, 5, 6, 7, 8]
            .map(|last_octet| connections.take_slot_unless_above(source_address(last_octet), 7));

        let slot = connections.take_slot_unless_above(source_address(9), 7);

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_from_one_source_stops_at_a_quarter_of_capacity() {
        let connections = UnadmittedConnections::with_capacity(64);

        let slots: Vec<_> = std::iter::repeat_with(|| connections.take_slot(source_address(1)))
            .take_while(Option::is_some)
            .collect();

        assert_eq!(slots.len(), 16);
    }

    #[test]
    fn take_slot_from_other_source_after_one_source_is_full_returns_slot() {
        let connections = UnadmittedConnections::with_capacity(64);
        let _slots: Vec<_> = std::iter::repeat_with(|| connections.take_slot(source_address(1)))
            .take_while(Option::is_some)
            .collect();

        let slot = connections.take_slot(source_address(2));

        assert!(slot.is_some());
    }

    #[test]
    fn drop_slot_gives_place_back() {
        let connections = UnadmittedConnections::with_capacity(2);
        let [first_slot, _second_slot] =
            [1, 2].map(|last_octet| connections.take_slot(source_address(last_octet)));

        drop(first_slot);

        assert!(connections.take_slot(source_address(3)).is_some());
    }

    #[test]
    fn take_slot_from_same_ipv6_prefix_returns_none() {
        let connections = UnadmittedConnections::with_capacity(4);
        let _slot = connections.take_slot("2001:db8::1".parse().unwrap());

        let slot = connections.take_slot("2001:db8::dead:beef".parse().unwrap());

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_from_other_ipv6_prefix_returns_slot() {
        let connections = UnadmittedConnections::with_capacity(4);
        let _slot = connections.take_slot("2001:db8::1".parse().unwrap());

        let slot = connections.take_slot("2001:db8:0:1::1".parse().unwrap());

        assert!(slot.is_some());
    }

    #[test]
    fn take_slot_from_mapped_ipv4_address_counts_as_the_ipv4_address() {
        let connections = UnadmittedConnections::with_capacity(4);
        let _slot = connections.take_slot("10.0.0.1".parse().unwrap());

        let slot = connections.take_slot("::ffff:10.0.0.1".parse().unwrap());

        assert!(slot.is_none());
    }

    #[test]
    fn take_slot_from_other_mapped_ipv4_address_returns_slot() {
        let connections = UnadmittedConnections::with_capacity(4);
        let _slot = connections.take_slot("10.0.0.1".parse().unwrap());

        let slot = connections.take_slot("::ffff:10.0.0.2".parse().unwrap());

        assert!(slot.is_some());
    }

    #[test]
    fn past_half_capacity_at_half_returns_false() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots =
            [1, 2, 3, 4].map(|last_octet| connections.take_slot(source_address(last_octet)));

        let crowded = connections.past_half_capacity();

        assert!(!crowded);
    }

    #[test]
    fn past_half_capacity_above_half_returns_true() {
        let connections = UnadmittedConnections::with_capacity(8);
        let _slots =
            [1, 2, 3, 4, 5].map(|last_octet| connections.take_slot(source_address(last_octet)));

        let crowded = connections.past_half_capacity();

        assert!(crowded);
    }
}
