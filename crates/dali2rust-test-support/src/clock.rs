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
