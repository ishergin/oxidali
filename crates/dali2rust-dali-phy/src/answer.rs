use core::sync::atomic::{AtomicU8, Ordering};

use crate::halfbits::HalfBitBuffer;

const ANSWER_IDLE: u8 = 0;
const ANSWER_STAGED: u8 = 1;

pub struct AtomicAnswerCell {
    state: AtomicU8,
    epoch: AtomicU8,
    data: [AtomicU8; HalfBitBuffer::DATA_LEN],
    len: AtomicU8,
}

impl AtomicAnswerCell {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(ANSWER_IDLE),
            epoch: AtomicU8::new(0),
            data: [const { AtomicU8::new(0) }; HalfBitBuffer::DATA_LEN],
            len: AtomicU8::new(0),
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn stage(&self, epoch: u8, hb: &HalfBitBuffer) -> bool {
        if hb.length == 0 || hb.length > HalfBitBuffer::MAX_DATA_HALF_BITS {
            return false;
        }
        if self.state.load(Ordering::Acquire) != ANSWER_IDLE {
            return false;
        }
        let [c0, c1, c2, c3, c4, c5, c6, c7, c8] = &self.data;
        let [b0, b1, b2, b3, b4, b5, b6, b7, b8] = hb.data;
        c0.store(b0, Ordering::Relaxed);
        c1.store(b1, Ordering::Relaxed);
        c2.store(b2, Ordering::Relaxed);
        c3.store(b3, Ordering::Relaxed);
        c4.store(b4, Ordering::Relaxed);
        c5.store(b5, Ordering::Relaxed);
        c6.store(b6, Ordering::Relaxed);
        c7.store(b7, Ordering::Relaxed);
        c8.store(b8, Ordering::Relaxed);
        self.len.store(hb.length, Ordering::Relaxed);
        self.epoch.store(epoch, Ordering::Relaxed);
        self.state
            .compare_exchange(ANSWER_IDLE, ANSWER_STAGED, Ordering::Release, Ordering::Relaxed)
            .is_ok()
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn staged_epoch(&self) -> Option<u8> {
        if self.state.load(Ordering::Acquire) != ANSWER_STAGED {
            return None;
        }
        Some(self.epoch.load(Ordering::Relaxed))
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn take(&self) -> Option<(u8, HalfBitBuffer)> {
        if self.state.load(Ordering::Acquire) != ANSWER_STAGED {
            return None;
        }
        let len = self.len.load(Ordering::Relaxed);
        let epoch = self.epoch.load(Ordering::Relaxed);
        const _: () = assert!(
            HalfBitBuffer::DATA_LEN == 9,
            "the array pattern below spells out every element"
        );
        let [b0, b1, b2, b3, b4, b5, b6, b7, b8] = &self.data;
        let data = [
            b0.load(Ordering::Relaxed),
            b1.load(Ordering::Relaxed),
            b2.load(Ordering::Relaxed),
            b3.load(Ordering::Relaxed),
            b4.load(Ordering::Relaxed),
            b5.load(Ordering::Relaxed),
            b6.load(Ordering::Relaxed),
            b7.load(Ordering::Relaxed),
            b8.load(Ordering::Relaxed),
        ];
        self.state.store(ANSWER_IDLE, Ordering::Release);
        Some((epoch, HalfBitBuffer { data, length: len }))
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn discard(&self) {
        self.state.store(ANSWER_IDLE, Ordering::Release);
    }
}

impl Default for AtomicAnswerCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(first: u8) -> HalfBitBuffer {
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = first;
        HalfBitBuffer { data, length: 22 }
    }

    #[test]
    fn stage_and_take_roundtrip_carries_the_epoch() {
        let cell = AtomicAnswerCell::new();
        assert!(cell.stage(7, &buf(0xAB)));
        assert_eq!(cell.staged_epoch(), Some(7));
        let (epoch, hb) = cell.take().unwrap();
        assert_eq!(epoch, 7);
        assert_eq!(hb.data[0], 0xAB);
        assert_eq!(hb.length, 22);
        assert!(cell.take().is_none());
        assert_eq!(cell.staged_epoch(), None);
    }

    #[test]
    fn a_second_stage_while_one_is_pending_is_refused() {
        let cell = AtomicAnswerCell::new();
        assert!(cell.stage(1, &buf(0x01)));
        assert!(!cell.stage(2, &buf(0x02)));
        assert_eq!(cell.staged_epoch(), Some(1));
    }

    #[test]
    fn discard_frees_the_slot_for_the_next_stage() {
        let cell = AtomicAnswerCell::new();
        assert!(cell.stage(1, &buf(0x01)));
        cell.discard();
        assert_eq!(cell.staged_epoch(), None);
        assert!(cell.stage(2, &buf(0x02)));
        assert_eq!(cell.staged_epoch(), Some(2));
    }

    #[test]
    fn an_invalid_length_is_refused() {
        let cell = AtomicAnswerCell::new();
        assert!(!cell.stage(1, &HalfBitBuffer { data: [0xFF; 9], length: 0 }));
        assert!(!cell.stage(1, &HalfBitBuffer { data: [0xFF; 9], length: 73 }));
    }
}
