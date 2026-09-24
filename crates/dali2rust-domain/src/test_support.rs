use core::sync::atomic::{AtomicU64, Ordering};

use dali2rust_platform::clock::Clock;

pub(crate) struct FakeClock {
    now: AtomicU64,
}

impl FakeClock {
    pub(crate) fn new(start_ms: u64) -> Self {
        Self {
            now: AtomicU64::new(start_ms),
        }
    }

    pub(crate) fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn monotonic_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}
