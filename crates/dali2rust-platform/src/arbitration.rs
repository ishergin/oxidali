use core::sync::atomic::{AtomicU32, Ordering};

const UNIT_PUBLISHED: u32 = 1 << 31;
const UNIT_ACTIVE: u32 = 1 << 8;
const UNIT_ADDRESS_MASK: u32 = 0xFF;
const UNIT_NO_ADDRESS: u32 = 0xFF;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArbitrationAnswerCounters {
    pub answered: u32,
    pub suppressed_lease: u32,
    pub cell_busy: u32,
    pub aborted: u32,
    pub window_closed: u32,
    pub out_of_window: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitInputs {
    pub application_active: bool,
    pub device_short_address: Option<u8>,
}

#[derive(Debug)]
pub struct AnswerInputs {
    unit: AtomicU32,
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
            unit: AtomicU32::new(0),
            lease_until_ms: AtomicU32::new(0),
            answered: AtomicU32::new(0),
            suppressed_lease: AtomicU32::new(0),
            cell_busy: AtomicU32::new(0),
            aborted: AtomicU32::new(0),
            window_closed: AtomicU32::new(0),
            out_of_window: AtomicU32::new(0),
        }
    }

    pub fn publish_unit(&self, inputs: UnitInputs) {
        let address = inputs
            .device_short_address
            .map_or(UNIT_NO_ADDRESS, u32::from);
        let active = if inputs.application_active { UNIT_ACTIVE } else { 0 };
        self.unit
            .store(UNIT_PUBLISHED | active | (address & UNIT_ADDRESS_MASK), Ordering::Release);
    }

    pub fn unit(&self) -> Option<UnitInputs> {
        let packed = self.unit.load(Ordering::Acquire);
        if packed & UNIT_PUBLISHED == 0 {
            return None;
        }
        let address = packed & UNIT_ADDRESS_MASK;
        Some(UnitInputs {
            application_active: packed & UNIT_ACTIVE != 0,
            device_short_address: (address != UNIT_NO_ADDRESS).then_some(address as u8),
        })
    }

    pub fn extend_lease(&self, now_ms: u32, ttl_ms: u32) {
        let until = now_ms.wrapping_add(ttl_ms);
        let current = self.lease_until_ms.load(Ordering::Acquire);
        if Self::is_after(until, current) {
            self.lease_until_ms.store(until, Ordering::Release);
        }
    }

    pub fn revoke_lease(&self) {
        self.lease_until_ms.store(0, Ordering::Release);
    }

    pub fn lease_alive(&self, now_ms: u32) -> bool {
        Self::is_after(self.lease_until_ms.load(Ordering::Acquire), now_ms)
    }

    pub fn lease_remaining_ms(&self, now_ms: u32) -> u32 {
        let until = self.lease_until_ms.load(Ordering::Acquire);
        if Self::is_after(until, now_ms) {
            until.wrapping_sub(now_ms)
        } else {
            0
        }
    }

    fn is_after(a: u32, b: u32) -> bool {
        (a.wrapping_sub(b) as i32) > 0
    }

    pub fn is_answering(&self, now_ms: u32) -> bool {
        self.lease_alive(now_ms) && self.unit().is_some_and(|unit| unit.application_active)
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

    const ACTIVE_AT_7: UnitInputs = UnitInputs {
        application_active: true,
        device_short_address: Some(7),
    };

    fn leased() -> AnswerInputs {
        let inputs = AnswerInputs::new();
        inputs.extend_lease(1_000, 3_000);
        inputs
    }

    #[test]
    fn nothing_is_published_before_the_supervisor_speaks() {
        assert_eq!(AnswerInputs::new().unit(), None);
    }

    #[test]
    fn the_unit_reads_back_as_published() {
        let inputs = AnswerInputs::new();
        inputs.publish_unit(ACTIVE_AT_7);
        assert_eq!(inputs.unit(), Some(ACTIVE_AT_7));
        let passive = UnitInputs {
            application_active: false,
            device_short_address: None,
        };
        inputs.publish_unit(passive);
        assert_eq!(inputs.unit(), Some(passive));
    }

    #[test]
    fn short_address_zero_survives_the_packing() {
        let inputs = AnswerInputs::new();
        let at_zero = UnitInputs {
            application_active: true,
            device_short_address: Some(0),
        };
        inputs.publish_unit(at_zero);
        assert_eq!(inputs.unit(), Some(at_zero));
    }

    #[test]
    fn a_supervisor_that_stops_calling_is_all_it_takes() {
        let inputs = leased();
        inputs.publish_unit(ACTIVE_AT_7);
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
        inputs.revoke_lease();
        assert!(!inputs.lease_alive(1_000));
    }

    #[test]
    fn a_passive_unit_is_not_answering_under_a_live_lease() {
        let inputs = leased();
        inputs.publish_unit(UnitInputs {
            application_active: false,
            device_short_address: Some(7),
        });
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
