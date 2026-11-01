use std::net::IpAddr;
use std::sync::Arc;

use rand::seq::IteratorRandom as _;

/// Truncates an IPv6 address to its /64 prefix. IPv4 addresses are returned unchanged.
///
/// An ISP assigns at least a /64 to each subscriber, so a client can use any address within it.
/// Keying on the full address would let one client obtain a new rate limit key at will. IPv4
/// addresses are assigned individually, so they are used as they are.
pub fn normalize_for_rate_limiting(peer: IpAddr) -> IpAddr {
    // A dual-stack listener reports IPv4 clients as `::ffff:a.b.c.d`. Truncating those would map
    // every IPv4 client to the same key.
    let peer = peer.to_canonical();

    let IpAddr::V6(address) = peer else {
        return peer;
    };

    let mut prefix = address.octets();

    prefix[8..].fill(0);

    IpAddr::V6(prefix.into())
}

pub enum TotalSemaphore {
    Shedding(LoadSheddingSemaphore),
    Hard(Arc<tokio::sync::Semaphore>),
}

impl TotalSemaphore {
    fn try_acquire_owned(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        match self {
            Self::Shedding(total) => total.try_acquire_owned(),
            Self::Hard(total) => Arc::clone(total).try_acquire_owned().ok(),
        }
    }
}

pub struct AdmissionSemaphore {
    total: TotalSemaphore,
    per_source: PerClientSemaphore,
}

impl AdmissionSemaphore {
    pub fn new(total: TotalSemaphore, per_source: usize) -> Self {
        Self {
            total,
            per_source: PerClientSemaphore::new(per_source),
        }
    }

    pub fn try_acquire_owned(
        &self,
        source: Option<std::net::IpAddr>,
    ) -> Result<AdmissionPermit, AdmissionRefused> {
        let total = self
            .total
            .try_acquire_owned()
            .ok_or(AdmissionRefused::Total)?;

        let source = match source {
            Some(peer) => Some(
                self.per_source
                    .try_acquire(peer)
                    .ok_or(AdmissionRefused::Source)?,
            ),
            None => None,
        };

        Ok(AdmissionPermit {
            _total: total,
            _source: source,
        })
    }
}

pub struct AdmissionPermit {
    _total: tokio::sync::OwnedSemaphorePermit,
    _source: Option<PerClientPermit>,
}

pub enum AdmissionRefused {
    Total,
    Source,
}

/// Once the semaphore is half full, randomly refuses with increased odds as the semaphore gets fuller.
pub struct LoadSheddingSemaphore {
    capacity: usize,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl LoadSheddingSemaphore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            semaphore: Arc::new(tokio::sync::Semaphore::new(capacity)),
        }
    }

    fn try_acquire_owned(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let refuse_above = (self.capacity / 2..self.capacity).choose(&mut rand::rng())?;

        self.try_acquire_owned_unless_above(refuse_above)
    }

    fn try_acquire_owned_unless_above(
        &self,
        refuse_above: usize,
    ) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let taken = self.capacity - self.semaphore.available_permits();

        if taken > refuse_above {
            return None;
        }

        Arc::clone(&self.semaphore).try_acquire_owned().ok()
    }
}

/// Holds semaphores keyed on connection source
struct PerClientSemaphore {
    counts: Arc<parking_lot::Mutex<std::collections::HashMap<std::net::IpAddr, usize>>>,
    capacity: usize,
}

impl PerClientSemaphore {
    fn new(capacity: usize) -> Self {
        Self {
            counts: Arc::default(),
            capacity,
        }
    }

    fn try_acquire(&self, peer: std::net::IpAddr) -> Option<PerClientPermit> {
        let peer = normalize_for_rate_limiting(peer);
        let mut held = self.counts.lock();
        let holding = held.get(&peer).copied().unwrap_or(0);

        if holding >= self.capacity {
            return None;
        }

        held.insert(peer, holding + 1);

        Some(PerClientPermit {
            counts: Arc::clone(&self.counts),
            source: peer,
        })
    }
}

struct PerClientPermit {
    counts: Arc<parking_lot::Mutex<std::collections::HashMap<std::net::IpAddr, usize>>>,
    source: std::net::IpAddr,
}

impl Drop for PerClientPermit {
    fn drop(&mut self) {
        let mut held = self.counts.lock();

        let Some(holding) = held.get_mut(&self.source) else {
            return;
        };

        *holding -= 1;

        if *holding == 0 {
            held.remove(&self.source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ip_address;

    #[test]
    fn normalize_for_rate_limiting_separates_mapped_ipv4_callers() {
        let first_ipv4 = normalize_for_rate_limiting(ip_address("::ffff:1.2.3.4"));
        let second_ipv4 = normalize_for_rate_limiting(ip_address("::ffff:203.0.113.9"));

        assert_eq!(first_ipv4, ip_address("1.2.3.4"));
        assert_eq!(second_ipv4, ip_address("203.0.113.9"));
    }

    #[test]
    fn normalize_for_rate_limiting_joins_one_ipv6_prefix() {
        assert_eq!(
            normalize_for_rate_limiting(ip_address("2001:db8:1:2:3:4:5:6")),
            normalize_for_rate_limiting(ip_address("2001:db8:1:2:aaaa::1"))
        );
        assert_ne!(
            normalize_for_rate_limiting(ip_address("2001:db8:1:2::1")),
            normalize_for_rate_limiting(ip_address("2001:db8:1:3::1"))
        );
    }

    const PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));
    const OTHER_PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 8));

    fn acquire_permits(semaphore: &LoadSheddingSemaphore, count: usize) -> Vec<impl Drop> {
        (0..count)
            .map(|_| {
                Arc::clone(&semaphore.semaphore)
                    .try_acquire_owned()
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn try_acquire_past_capacity_of_a_source_refuses_it() {
        let sources = PerClientSemaphore::new(2);
        let _permits = [sources.try_acquire(PEER), sources.try_acquire(PEER)];

        let refused = sources.try_acquire(PEER);

        assert!(refused.is_none());
    }

    #[test]
    fn try_acquire_past_capacity_of_a_source_admits_another_source() {
        let sources = PerClientSemaphore::new(2);
        let _permits = [sources.try_acquire(PEER), sources.try_acquire(PEER)];

        let admitted = sources.try_acquire(OTHER_PEER);

        assert!(admitted.is_some());
    }

    #[test]
    fn try_acquire_after_permits_drop_admits_the_source_again() {
        let sources = PerClientSemaphore::new(2);
        drop([sources.try_acquire(PEER), sources.try_acquire(PEER)]);

        let admitted = sources.try_acquire(PEER);

        assert!(admitted.is_some());
    }

    #[test]
    fn try_acquire_forgets_a_source_after_its_last_permit_drops() {
        let sources = PerClientSemaphore::new(2);

        drop(sources.try_acquire(PEER).unwrap());

        assert!(sources.counts.lock().is_empty());
    }

    #[test]
    fn try_acquire_owned_at_half_admits() {
        let semaphore = LoadSheddingSemaphore::new(8);
        let _permits = acquire_permits(&semaphore, 4);

        let admitted = semaphore.try_acquire_owned();

        assert!(admitted.is_some());
    }

    #[test]
    fn try_acquire_owned_at_capacity_refuses() {
        let semaphore = LoadSheddingSemaphore::new(8);
        let _permits = acquire_permits(&semaphore, 8);

        let refused = semaphore.try_acquire_owned();

        assert!(refused.is_none());
    }

    #[test]
    fn try_acquire_owned_unless_above_past_half_with_draw_below_taken_refuses() {
        let semaphore = LoadSheddingSemaphore::new(8);
        let _permits = acquire_permits(&semaphore, 5);

        let refused = semaphore.try_acquire_owned_unless_above(4);

        assert!(refused.is_none());
    }

    #[test]
    fn try_acquire_owned_unless_above_past_half_with_draw_at_taken_admits() {
        let semaphore = LoadSheddingSemaphore::new(8);
        let _permits = acquire_permits(&semaphore, 5);

        let admitted = semaphore.try_acquire_owned_unless_above(5);

        assert!(admitted.is_some());
    }
}
