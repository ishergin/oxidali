use core::sync::atomic::{AtomicU32, Ordering};

pub const ARBITRATION_REFLEX_SLOTS: usize = 4;

type PackedSlot = u32;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArbitrationReflexCounters {
    pub answered: u32,
    pub suppressed_lease: u32,
    pub cell_busy: u32,
    pub aborted: u32,
    pub window_closed: u32,
    pub out_of_window: u32,
}

#[derive(Debug)]
pub struct ArbitrationReflex {
    slots: [AtomicU32; ARBITRATION_REFLEX_SLOTS],
    lease_until_ms: AtomicU32,
    answered: AtomicU32,
    suppressed_lease: AtomicU32,
    cell_busy: AtomicU32,
    aborted: AtomicU32,
    window_closed: AtomicU32,
    out_of_window: AtomicU32,
}

impl Default for ArbitrationReflex {
    fn default() -> Self {
        Self::new()
    }
}

impl ArbitrationReflex {
    pub const fn new() -> Self {
        #[allow(clippy::declare_interior_mutable_const, reason = "array initialiser; each element is copied into its own slot")]
        const EMPTY: AtomicU32 = AtomicU32::new(0);
        Self {
            slots: [EMPTY; ARBITRATION_REFLEX_SLOTS],
            lease_until_ms: AtomicU32::new(0),
            answered: AtomicU32::new(0),
            suppressed_lease: AtomicU32::new(0),
            cell_busy: AtomicU32::new(0),
            aborted: AtomicU32::new(0),
            window_closed: AtomicU32::new(0),
            out_of_window: AtomicU32::new(0),
        }
    }

    const fn pack(frame: [u8; 3], answer: u8) -> PackedSlot {
        (frame[0] as u32) << 24 | (frame[1] as u32) << 16 | (frame[2] as u32) << 8 | answer as u32
    }

    pub fn set_slot(&self, index: usize, frame: [u8; 3], answer: u8) -> bool {
        let packed = Self::pack(frame, answer);
        if packed == 0 {
            return false;
        }
        match self.slots.get(index) {
            Some(slot) => {
                slot.store(packed, Ordering::Release);
                true
            }
            None => false,
        }
    }

    pub fn clear_slot(&self, index: usize) {
        if let Some(slot) = self.slots.get(index) {
            slot.store(0, Ordering::Release);
        }
    }

    pub fn clear_all(&self) {
        for slot in &self.slots {
            slot.store(0, Ordering::Release);
        }
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

    pub fn answer_for(&self, frame: [u8; 3], now_ms: u32) -> Option<u8> {
        let wanted = Self::pack(frame, 0) & 0xFFFF_FF00;
        for slot in &self.slots {
            let packed = slot.load(Ordering::Acquire);
            if packed == 0 || packed & 0xFFFF_FF00 != wanted {
                continue;
            }
            if !self.lease_alive(now_ms) {
                self.suppressed_lease.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            return Some((packed & 0xFF) as u8);
        }
        None
    }

    pub fn note_cell_busy(&self) {
        self.cell_busy.fetch_add(1, Ordering::Relaxed);
    }

    pub fn mirror_transmit(&self, answered: u32, aborted: u32, window_closed: u32, late: u32) {
        self.answered.store(answered, Ordering::Relaxed);
        self.aborted.store(aborted, Ordering::Relaxed);
        self.window_closed.store(window_closed, Ordering::Relaxed);
        self.out_of_window.store(late, Ordering::Relaxed);
    }

    pub fn counters(&self) -> ArbitrationReflexCounters {
        ArbitrationReflexCounters {
            answered: self.answered.load(Ordering::Relaxed),
            suppressed_lease: self.suppressed_lease.load(Ordering::Relaxed),
            cell_busy: self.cell_busy.load(Ordering::Relaxed),
            aborted: self.aborted.load(Ordering::Relaxed),
            window_closed: self.window_closed.load(Ordering::Relaxed),
            out_of_window: self.out_of_window.load(Ordering::Relaxed),
        }
    }

    pub fn is_armed(&self, now_ms: u32) -> bool {
        if !self.lease_alive(now_ms) {
            return false;
        }
        self.slots
            .iter()
            .any(|slot| slot.load(Ordering::Acquire) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY_BROADCAST: [u8; 3] = [0xFF, 0xFE, 0x3D];
    const QUERY_ADDRESSED: [u8; 3] = [0x07, 0xFE, 0x3D];
    const YES: u8 = 0xFF;

    fn armed() -> ArbitrationReflex {
        let r = ArbitrationReflex::new();
        assert!(r.set_slot(0, QUERY_BROADCAST, YES));
        r.extend_lease(1_000, 3_000);
        r
    }

    #[test]
    fn a_published_frame_is_answered_while_the_lease_is_alive() {
        let r = armed();
        assert_eq!(r.answer_for(QUERY_BROADCAST, 1_000), Some(YES));
        assert_eq!(r.answer_for(QUERY_BROADCAST, 3_999), Some(YES));
    }

    #[test]
    fn an_expired_lease_answers_nothing_and_says_so() {
        let r = armed();
        assert_eq!(r.answer_for(QUERY_BROADCAST, 4_000), None);
        assert_eq!(r.counters().suppressed_lease, 1);
    }

    #[test]
    fn a_supervisor_that_stops_calling_is_all_it_takes() {
        let r = armed();
        assert!(r.lease_alive(3_500));
        assert!(!r.lease_alive(4_001));
        assert!(!r.is_armed(4_001));
    }

    #[test]
    fn extending_never_shortens_a_live_lease() {
        let r = armed();
        r.extend_lease(1_000, 500);
        assert!(r.lease_alive(3_999));
    }

    #[test]
    fn revoking_is_immediate() {
        let r = armed();
        r.revoke_lease();
        assert_eq!(r.answer_for(QUERY_BROADCAST, 1_000), None);
    }

    #[test]
    fn an_unpublished_frame_matches_nothing_and_is_not_counted() {
        let r = armed();
        assert_eq!(r.answer_for(QUERY_ADDRESSED, 1_000), None);
        assert_eq!(r.counters().suppressed_lease, 0);
    }

    #[test]
    fn slots_are_independent_and_clearable() {
        let r = armed();
        assert!(r.set_slot(1, QUERY_ADDRESSED, YES));
        assert_eq!(r.answer_for(QUERY_ADDRESSED, 1_000), Some(YES));
        r.clear_slot(1);
        assert_eq!(r.answer_for(QUERY_ADDRESSED, 1_000), None);
        assert_eq!(r.answer_for(QUERY_BROADCAST, 1_000), Some(YES));
    }

    #[test]
    fn an_out_of_range_slot_is_refused_rather_than_wrapped() {
        let r = ArbitrationReflex::new();
        assert!(!r.set_slot(ARBITRATION_REFLEX_SLOTS, QUERY_BROADCAST, YES));
    }

    #[test]
    fn the_empty_encoding_is_refused_instead_of_being_documented() {
        let r = ArbitrationReflex::new();
        assert!(!r.set_slot(0, [0, 0, 0], 0));
    }

    #[test]
    fn the_millisecond_wrap_is_a_non_event() {
        let r = ArbitrationReflex::new();
        assert!(r.set_slot(0, QUERY_BROADCAST, YES));
        let now = u32::MAX - 1_000;
        r.extend_lease(now, 3_000);
        assert!(r.lease_alive(now));
        assert!(r.lease_alive(now.wrapping_add(2_999)));
        assert!(!r.lease_alive(now.wrapping_add(3_001)));
    }

    #[test]
    fn nothing_is_armed_before_a_slot_is_published() {
        let r = ArbitrationReflex::new();
        r.extend_lease(1_000, 3_000);
        assert!(!r.is_armed(1_000));
    }

    #[test]
    fn mirror_transmit_republishes_the_interrupts_answering_counts() {
        let r = ArbitrationReflex::new();
        r.mirror_transmit(7, 2, 1, 3);
        let c = r.counters();
        assert_eq!((c.answered, c.aborted, c.window_closed, c.out_of_window), (7, 2, 1, 3));
        r.note_cell_busy();
        assert_eq!(r.counters().cell_busy, 1);
        r.mirror_transmit(9, 2, 1, 3);
        assert_eq!(r.counters().answered, 9);
    }
}
