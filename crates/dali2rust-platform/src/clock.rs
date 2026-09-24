pub trait UnixTimeMs: Send + Sync {
    fn unix_millis(&self) -> u64;
}

pub trait Clock: Send + Sync {
    fn monotonic_ms(&self) -> u64;

    fn monotonic_duration(&self) -> core::time::Duration {
        core::time::Duration::from_millis(self.monotonic_ms())
    }
}
