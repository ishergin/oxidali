use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[must_use]
pub fn monotonic_ms() -> u32 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let base = BASE.get_or_init(std::time::Instant::now);
    base.elapsed().as_millis() as u32
}

#[derive(Debug)]
pub struct LivenessBeat {
    name: &'static str,
    last_ms: AtomicU32,
    seen: AtomicBool,
    stale_after_ms: u32,
}

impl LivenessBeat {
    #[must_use]
    pub const fn new(name: &'static str, stale_after_ms: u32) -> Self {
        Self {
            name,
            last_ms: AtomicU32::new(0),
            seen: AtomicBool::new(false),
            stale_after_ms,
        }
    }

    pub fn beat(&self, now_ms: u32) {
        self.last_ms.store(now_ms, Ordering::Release);
        self.seen.store(true, Ordering::Release);
    }

    pub fn while_turning<T>(&self, step: impl FnOnce() -> T) -> T {
        self.beat(monotonic_ms());
        let out = step();
        self.beat(monotonic_ms());
        out
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn age_ms(&self, now_ms: u32) -> Option<u32> {
        if !self.seen.load(Ordering::Acquire) {
            return None;
        }
        let delta = now_ms.wrapping_sub(self.last_ms.load(Ordering::Acquire));
        #[allow(clippy::cast_possible_wrap, reason = "the sign IS the question here")]
        Some(if (delta as i32) < 0 { 0 } else { delta })
    }

    #[must_use]
    pub fn is_fresh(&self, now_ms: u32) -> bool {
        match self.age_ms(now_ms) {
            Some(age) => age <= self.stale_after_ms,
            None => false,
        }
    }
}

#[derive(Debug, Default)]
pub struct LivenessWatch {
    beats: Vec<std::sync::Arc<LivenessBeat>>,
}

impl LivenessWatch {
    #[must_use]
    pub fn new() -> Self {
        Self { beats: Vec::new() }
    }

    pub fn register(&mut self, beat: std::sync::Arc<LivenessBeat>) {
        self.beats.push(beat);
    }

    #[must_use]
    pub fn stale(&self, now_ms: u32) -> Option<&'static str> {
        self.beats
            .iter()
            .find(|b| !b.is_fresh(now_ms))
            .map(|b| b.name())
    }

    #[must_use]
    pub fn stale_age(&self, now_ms: u32) -> Option<(&'static str, Option<u32>)> {
        self.beats
            .iter()
            .find(|b| !b.is_fresh(now_ms))
            .map(|b| (b.name(), b.age_ms(now_ms)))
    }

    #[must_use]
    pub fn all_fresh(&self, now_ms: u32) -> bool {
        self.stale(now_ms).is_none()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.beats.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const STALE_AFTER: u32 = 3_000;

    fn watch(beats: &[&Arc<LivenessBeat>]) -> LivenessWatch {
        let mut w = LivenessWatch::new();
        for b in beats {
            w.register(Arc::clone(b));
        }
        w
    }

    #[test]
    fn a_worker_that_never_reported_a_turn_is_not_fresh() {
        let b = LivenessBeat::new("registry", STALE_AFTER);
        assert!(!b.is_fresh(0));
        assert!(!b.is_fresh(10_000));
        assert_eq!(b.age_ms(10_000), None);
    }

    #[test]
    fn a_bounded_step_is_a_turn_on_the_way_in_and_out() {
        let b = LivenessBeat::new("hcl", STALE_AFTER);
        let started = monotonic_ms();
        let waited = b.while_turning(|| {
            while monotonic_ms().wrapping_sub(started) < 3 {}
            monotonic_ms().wrapping_sub(started)
        });
        assert!(waited >= 3);
        assert!(b.age_ms(monotonic_ms()).expect("stamped") < waited);
    }

    #[test]
    fn a_stale_worker_says_whether_it_ever_turned() {
        let never = Arc::new(LivenessBeat::new("hcl", STALE_AFTER));
        let stopped = Arc::new(LivenessBeat::new("rules", STALE_AFTER));
        stopped.beat(1_000);
        assert_eq!(watch(&[&never]).stale_age(2_000), Some(("hcl", None)));
        assert_eq!(watch(&[&stopped]).stale_age(9_000), Some(("rules", Some(8_000))));
        assert_eq!(watch(&[&stopped]).stale_age(3_000), None);
    }

    #[test]
    fn a_turning_worker_stays_fresh_up_to_its_window() {
        let b = LivenessBeat::new("rules", STALE_AFTER);
        b.beat(1_000);
        assert!(b.is_fresh(1_000));
        assert!(b.is_fresh(4_000));
        assert!(!b.is_fresh(4_001));
    }

    #[test]
    fn one_stale_worker_names_itself() {
        let registry = Arc::new(LivenessBeat::new("registry", STALE_AFTER));
        let hcl = Arc::new(LivenessBeat::new("hcl", STALE_AFTER));
        registry.beat(1_000);
        hcl.beat(1_000);
        let w = watch(&[&registry, &hcl]);
        assert!(w.all_fresh(2_000));
        registry.beat(5_000);
        assert_eq!(w.stale(5_000), Some("hcl"));
    }

    #[test]
    fn an_empty_watch_is_vacuously_fresh_and_says_it_is_empty() {
        let w = LivenessWatch::new();
        assert!(w.all_fresh(1_000));
        assert!(w.is_empty());
    }

    #[test]
    fn the_millisecond_wrap_is_not_a_stale_worker() {
        let b = LivenessBeat::new("registry", STALE_AFTER);
        let now = u32::MAX - 500;
        b.beat(now);
        assert!(b.is_fresh(now.wrapping_add(1_000)));
        assert!(!b.is_fresh(now.wrapping_add(3_001)));
    }

    #[test]
    fn a_stamp_from_the_immediate_future_is_a_turn_not_a_wrap() {
        let b = LivenessBeat::new("registry", STALE_AFTER);
        b.beat(10_000);
        for ahead in 1..=50u32 {
            let now = 10_000 - ahead;
            assert_eq!(b.age_ms(now), Some(0), "stamp {ahead} ms ahead of now");
            assert!(b.is_fresh(now), "a worker that just turned is not stale");
        }
        assert_eq!(b.age_ms(12_000), Some(2_000));
        assert!(!b.is_fresh(13_001));
    }

    #[test]
    fn the_shared_clock_is_monotonic_and_starts_near_zero() {
        let a = monotonic_ms();
        let b = monotonic_ms();
        assert!(b >= a);
        assert!(a < 60_000);
    }

    #[test]
    fn a_recovered_worker_is_fresh_again_with_no_reset_call() {
        let b = LivenessBeat::new("hcl", STALE_AFTER);
        b.beat(1_000);
        assert!(!b.is_fresh(9_000));
        b.beat(9_100);
        assert!(b.is_fresh(9_100));
    }
}
