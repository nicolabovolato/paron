use std::sync::Arc;

use crate::application::clock::Clock;

struct FixedClock(jiff::Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> jiff::Timestamp {
        self.0
    }
}

pub fn clock_at(now: jiff::Timestamp) -> Arc<dyn Clock> {
    Arc::new(FixedClock(now))
}

pub fn timestamp(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).unwrap()
}
