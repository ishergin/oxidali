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
