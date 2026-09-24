use dali2rust_platform::clock::UnixTimeMs;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StdUnixTimeMs;

impl UnixTimeMs for StdUnixTimeMs {
    fn unix_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[inline]
pub fn unix_wall_clock_millis() -> u64 {
    StdUnixTimeMs.unix_millis()
}
