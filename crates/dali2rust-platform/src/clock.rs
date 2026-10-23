pub trait UnixTimeMs: Send + Sync {
    fn unix_millis(&self) -> u64;
}

pub trait Clock: Send + Sync {
    fn monotonic_ms(&self) -> u64;

    fn monotonic_duration(&self) -> core::time::Duration {
        core::time::Duration::from_millis(self.monotonic_ms())
    }
}

#[derive(Clone)]
pub struct Uptime {
    clock: std::sync::Arc<dyn Clock>,
    started_ms: u64,
}

impl Uptime {
    #[must_use]
    pub fn start(clock: std::sync::Arc<dyn Clock>) -> Self {
        let started_ms = clock.monotonic_ms();
        Self { clock, started_ms }
    }

    #[must_use]
    pub fn elapsed_ms(&self) -> u64 {
        self.clock.monotonic_ms().saturating_sub(self.started_ms)
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use super::*;

    struct SteppedClock(AtomicU64);

    impl Clock for SteppedClock {
        fn monotonic_ms(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn uptime_counts_from_the_clock_reading_it_started_at() {
        let clock = Arc::new(SteppedClock(AtomicU64::new(4_000)));
        let uptime = Uptime::start(Arc::clone(&clock) as Arc<dyn Clock>);
        assert_eq!(uptime.elapsed_ms(), 0);
        clock.0.store(4_750, Ordering::Relaxed);
        assert_eq!(uptime.elapsed_ms(), 750);
    }

    #[test]
    fn every_copy_of_an_uptime_reads_the_same_start() {
        let clock = Arc::new(SteppedClock(AtomicU64::new(100)));
        let first = Uptime::start(Arc::clone(&clock) as Arc<dyn Clock>);
        clock.0.store(160, Ordering::Relaxed);
        let copy = first.clone();
        clock.0.store(400, Ordering::Relaxed);
        assert_eq!((first.elapsed_ms(), copy.elapsed_ms()), (300, 300));
    }

    #[test]
    fn a_clock_that_reads_earlier_than_the_start_is_zero_not_a_wrap() {
        let clock = Arc::new(SteppedClock(AtomicU64::new(900)));
        let uptime = Uptime::start(Arc::clone(&clock) as Arc<dyn Clock>);
        clock.0.store(100, Ordering::Relaxed);
        assert_eq!(uptime.elapsed_ms(), 0);
    }
}
