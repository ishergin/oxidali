use core::sync::atomic::{AtomicU32, Ordering};

use crate::bus_unit::{BusUnitSnapshot, Busy, Latest};
use crate::liveness::CLOCK_READ_SKEW_MS;

pub const LEASE_TTL_MS: u32 = 3_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArbitrationAnswerCounters {
    pub answered: u32,
    pub suppressed_lease: u32,
    pub cell_busy: u32,
    pub aborted: u32,
    pub window_closed: u32,
    pub out_of_window: u32,
}

#[derive(Debug)]
pub struct AnswerInputs {
    bus_unit: Latest<BusUnitSnapshot>,
    lease_until_ms: AtomicU32,
    answered: AtomicU32,
    suppressed_lease: AtomicU32,
    cell_busy: AtomicU32,
    aborted: AtomicU32,
    window_closed: AtomicU32,
    out_of_window: AtomicU32,
}

impl Default for AnswerInputs {
    fn default() -> Self {
        Self::new()
    }
}

impl AnswerInputs {
    pub const fn new() -> Self {
        Self {
            bus_unit: Latest::new(),
            lease_until_ms: AtomicU32::new(0),
            answered: AtomicU32::new(0),
            suppressed_lease: AtomicU32::new(0),
            cell_busy: AtomicU32::new(0),
            aborted: AtomicU32::new(0),
            window_closed: AtomicU32::new(0),
            out_of_window: AtomicU32::new(0),
        }
    }

    pub fn publish_bus_unit(&self, snapshot: BusUnitSnapshot) {
        self.bus_unit.publish(snapshot);
    }

    pub fn bus_unit(&self) -> Option<BusUnitSnapshot> {
        self.bus_unit.latest()
    }

    pub fn try_bus_unit(&self) -> Result<Option<BusUnitSnapshot>, Busy> {
        self.bus_unit.try_latest()
    }

    pub fn extend_lease(&self, now_ms: u32, ttl_ms: u32) {
        let ttl_ms = ttl_ms.min(LEASE_TTL_MS);
        if ttl_ms > self.lease_remaining_ms(now_ms) {
            self.lease_until_ms
                .store(now_ms.wrapping_add(ttl_ms), Ordering::Release);
        }
    }

    pub fn revoke_lease(&self, now_ms: u32) {
        self.lease_until_ms.store(now_ms, Ordering::Release);
    }

    pub fn lease_alive(&self, now_ms: u32) -> bool {
        self.lease_remaining_ms(now_ms) > 0
    }

    pub fn lease_remaining_ms(&self, now_ms: u32) -> u32 {
        let remaining = self.lease_until_ms.load(Ordering::Acquire).wrapping_sub(now_ms);
        if remaining <= LEASE_TTL_MS {
            remaining
        } else if remaining <= LEASE_TTL_MS + CLOCK_READ_SKEW_MS {
            LEASE_TTL_MS
        } else {
            0
        }
    }

    pub fn is_answering(&self, now_ms: u32) -> bool {
        self.lease_alive(now_ms) && self.bus_unit().is_some_and(|unit| unit.nvm.application_active)
    }

    pub fn mirror_answers(&self, counts: ArbitrationAnswerCounters) {
        self.answered.store(counts.answered, Ordering::Relaxed);
        self.suppressed_lease.store(counts.suppressed_lease, Ordering::Relaxed);
        self.cell_busy.store(counts.cell_busy, Ordering::Relaxed);
        self.aborted.store(counts.aborted, Ordering::Relaxed);
        self.window_closed.store(counts.window_closed, Ordering::Relaxed);
        self.out_of_window.store(counts.out_of_window, Ordering::Relaxed);
    }

    pub fn counters(&self) -> ArbitrationAnswerCounters {
        ArbitrationAnswerCounters {
            answered: self.answered.load(Ordering::Relaxed),
            suppressed_lease: self.suppressed_lease.load(Ordering::Relaxed),
            cell_busy: self.cell_busy.load(Ordering::Relaxed),
            aborted: self.aborted.load(Ordering::Relaxed),
            window_closed: self.window_closed.load(Ordering::Relaxed),
            out_of_window: self.out_of_window.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE_AT_7: BusUnitSnapshot = BusUnitSnapshot {
        nvm: crate::bus_unit::BusUnitNvm {
            short_address: Some(7),
            random_address: 0x00FF_FFFF,
            device_groups: 0,
            operating_mode: 0,
            power_cycle_notification: false,
            application_active: true,
        },
        revision: 1,
        acknowledged_wire_sequence: 0,
    };

    fn leased() -> AnswerInputs {
        let inputs = AnswerInputs::new();
        inputs.extend_lease(1_000, 3_000);
        inputs
    }

    #[test]
    fn nothing_is_published_before_the_registry_speaks() {
        assert_eq!(AnswerInputs::new().bus_unit(), None);
        assert!(!leased().is_answering(1_000), "no unit, no answer");
    }

    #[test]
    fn a_supervisor_that_stops_calling_is_all_it_takes() {
        let inputs = leased();
        inputs.publish_bus_unit(ACTIVE_AT_7);
        assert!(inputs.is_answering(3_500));
        assert!(!inputs.lease_alive(4_001));
        assert!(!inputs.is_answering(4_001));
    }

    #[test]
    fn extending_never_shortens_a_live_lease() {
        let inputs = leased();
        inputs.extend_lease(1_000, 500);
        assert!(inputs.lease_alive(3_999));
    }

    #[test]
    fn revoking_is_immediate() {
        let inputs = leased();
        inputs.revoke_lease(1_000);
        assert!(!inputs.lease_alive(1_000));
    }

    #[test]
    fn a_lease_revoked_late_in_the_clock_stays_revoked_and_extends_again() {
        for revoked_at in [(1u32 << 31) + 10, u32::MAX - 1_000] {
            let inputs = AnswerInputs::new();
            inputs.revoke_lease(revoked_at);
            let later = revoked_at.wrapping_add(500);
            assert!(!inputs.lease_alive(revoked_at), "revoked at {revoked_at}");
            assert_eq!(inputs.lease_remaining_ms(later), 0, "revoked at {revoked_at}");
            inputs.extend_lease(later, LEASE_TTL_MS);
            assert!(inputs.lease_alive(later));
            assert!(!inputs.lease_alive(later.wrapping_add(LEASE_TTL_MS)));
        }
    }

    #[test]
    fn a_lease_left_alone_past_half_the_clock_stays_lapsed() {
        let inputs = leased();
        let long_after = 4_000u32.wrapping_add(1 << 31).wrapping_add(5);
        assert!(!inputs.lease_alive(long_after));
        assert_eq!(inputs.lease_remaining_ms(long_after), 0);
        inputs.extend_lease(long_after, LEASE_TTL_MS);
        assert!(inputs.lease_alive(long_after));
    }

    #[test]
    fn a_lease_extended_on_a_clock_a_little_ahead_reads_whole() {
        let inputs = AnswerInputs::new();
        inputs.extend_lease(10_001, LEASE_TTL_MS);
        assert_eq!(inputs.lease_remaining_ms(10_000), LEASE_TTL_MS);
        let too_far_ahead = AnswerInputs::new();
        too_far_ahead.extend_lease(10_000 + CLOCK_READ_SKEW_MS + 1, LEASE_TTL_MS);
        assert_eq!(too_far_ahead.lease_remaining_ms(10_000), 0);
    }

    #[test]
    fn a_lease_asked_for_beyond_the_ttl_is_held_to_it() {
        let inputs = AnswerInputs::new();
        inputs.extend_lease(1_000, 10 * LEASE_TTL_MS);
        assert_eq!(inputs.lease_remaining_ms(1_000), LEASE_TTL_MS);
        assert!(inputs.lease_alive(1_000 + LEASE_TTL_MS - 1));
        assert!(!inputs.lease_alive(1_000 + LEASE_TTL_MS));
    }

    #[test]
    fn a_passive_unit_is_not_answering_under_a_live_lease() {
        let inputs = leased();
        let mut passive = ACTIVE_AT_7;
        passive.nvm.application_active = false;
        inputs.publish_bus_unit(passive);
        assert!(!inputs.is_answering(1_000));
    }

    #[test]
    fn the_millisecond_wrap_is_a_non_event() {
        let inputs = AnswerInputs::new();
        let now = u32::MAX - 1_000;
        inputs.extend_lease(now, 3_000);
        assert!(inputs.lease_alive(now));
        assert!(inputs.lease_alive(now.wrapping_add(2_999)));
        assert!(!inputs.lease_alive(now.wrapping_add(3_001)));
    }

    #[test]
    fn the_interrupt_s_answer_counts_are_mirrored_whole() {
        let inputs = AnswerInputs::new();
        let counts = ArbitrationAnswerCounters {
            answered: 7,
            suppressed_lease: 4,
            cell_busy: 1,
            aborted: 2,
            window_closed: 1,
            out_of_window: 3,
        };
        inputs.mirror_answers(counts);
        assert_eq!(inputs.counters(), counts);
    }
}
