use core::sync::atomic::{AtomicU64, Ordering};

use dali2rust_platform::clock::Clock;

pub struct StoppedClock {
    now_ms: u64,
}

impl StoppedClock {
    pub const fn at(now_ms: u64) -> Self {
        Self { now_ms }
    }
}

impl Clock for StoppedClock {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms
    }
}

pub struct AdvancingClock {
    now_ms: AtomicU64,
    step_ms: u64,
}

impl AdvancingClock {
    pub const fn new(step_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(0),
            step_ms,
        }
    }
}

impl Clock for AdvancingClock {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms.fetch_add(self.step_ms, Ordering::SeqCst)
    }
}

pub struct SettableClock {
    now_ms: AtomicU64,
}

impl SettableClock {
    pub const fn at(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    pub fn set(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for SettableClock {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settable_clock_reads_what_it_was_last_set_to() {
        let clock = SettableClock::at(40);
        assert_eq!(clock.monotonic_ms(), 40);
        clock.set(75);
        assert_eq!(clock.monotonic_ms(), 75);
    }
}
