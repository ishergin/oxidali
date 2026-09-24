use dali2rust_platform::clock::Clock;

#[derive(Debug, Clone)]
pub struct StdClock {
    start: std::time::Instant,
}

impl StdClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Default for StdClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for StdClock {
    fn monotonic_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}
