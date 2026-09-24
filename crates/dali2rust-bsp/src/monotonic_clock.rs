use std::sync::OnceLock;
use std::time::Instant;

fn anchor() -> &'static Instant {
    static ANCHOR: OnceLock<Instant> = OnceLock::new();
    ANCHOR.get_or_init(Instant::now)
}

#[inline]
pub fn monotonic_millis() -> u64 {
    anchor().elapsed().as_millis() as u64
}

#[inline]
pub fn observation_stamp_ms() -> u32 {
    monotonic_millis() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_does_not_go_backwards() {
        let first = monotonic_millis();
        let second = monotonic_millis();
        assert!(second >= first);
    }

    #[test]
    fn the_wire_stamp_is_the_low_half_of_the_same_scale() {
        assert_eq!(u64::from(observation_stamp_ms()), monotonic_millis() & 0xFFFF_FFFF);
    }
}
