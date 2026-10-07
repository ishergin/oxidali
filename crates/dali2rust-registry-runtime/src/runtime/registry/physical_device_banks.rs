use dali2rust_platform::slice_store::SliceKey;

const SHORTS_PER_BANK_MASK: u64 = (1 << SliceKey::DEVICES_PER_BANK) - 1;

pub(crate) fn bank_bit(short_address: u8) -> u16 {
    1u16.checked_shl(u32::from(short_address / SliceKey::DEVICES_PER_BANK))
        .unwrap_or(0)
}

pub(crate) fn short_bit(short_address: u8) -> u64 {
    1u64.checked_shl(u32::from(short_address)).unwrap_or(0)
}

pub(crate) fn banks_of(shorts: u64) -> u16 {
    (0..SliceKey::PHYSICAL_DEVICE_BANKS)
        .filter(|bank| {
            let first = u32::from(*bank) * u32::from(SliceKey::DEVICES_PER_BANK);
            shorts.checked_shr(first).unwrap_or(0) & SHORTS_PER_BANK_MASK != 0
        })
        .fold(0, |banks, bank| banks | (1 << bank))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotOutcome {
    Loaded,
    Missing,
    Rejected,
    Unread,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BankOutcomes {
    loaded: u16,
    missing: u16,
    rejected: u16,
    unread: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BankDecision {
    pub(crate) rewrite: u16,
    pub(crate) settled: u16,
    pub(crate) waiting: u16,
}

impl BankOutcomes {
    pub(crate) fn note(&mut self, bank: u8, outcome: SlotOutcome) {
        let bit = 1u16 << bank;
        match outcome {
            SlotOutcome::Loaded => self.loaded |= bit,
            SlotOutcome::Missing => self.missing |= bit,
            SlotOutcome::Rejected => self.rejected |= bit,
            SlotOutcome::Unread => self.unread |= bit,
        }
    }

    pub(crate) fn open(self) -> u16 {
        self.missing | self.rejected
    }

    pub(crate) fn decide(self, old_slot: Option<SlotOutcome>) -> BankDecision {
        let open = self.open();
        let (rewrite, waiting) = match old_slot {
            None => (0, 0),
            Some(SlotOutcome::Loaded | SlotOutcome::Rejected) => (open, 0),
            Some(SlotOutcome::Missing) => (self.rejected, 0),
            Some(SlotOutcome::Unread) => (0, open),
        };
        BankDecision {
            rewrite,
            settled: self.loaded | (open & !waiting),
            waiting: waiting | self.unread,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{banks_of, short_bit, BankDecision, BankOutcomes, SlotOutcome};

    const LOADED: u8 = 0;
    const MISSING: u8 = 1;
    const REJECTED: u8 = 2;
    const UNREAD: u8 = 3;

    fn one_of_each() -> BankOutcomes {
        let mut banks = BankOutcomes::default();
        banks.note(LOADED, SlotOutcome::Loaded);
        banks.note(MISSING, SlotOutcome::Missing);
        banks.note(REJECTED, SlotOutcome::Rejected);
        banks.note(UNREAD, SlotOutcome::Unread);
        banks
    }

    fn decision(rewrite: &[u8], settled: &[u8], waiting: &[u8]) -> BankDecision {
        let mask = |banks: &[u8]| banks.iter().fold(0u16, |mask, bank| mask | (1 << bank));
        BankDecision { rewrite: mask(rewrite), settled: mask(settled), waiting: mask(waiting) }
    }

    #[test]
    fn the_old_slot_decides_which_banks_are_rewritten_settled_or_kept_waiting() {
        let banks = one_of_each();
        let all_open = [MISSING, REJECTED];
        let cases = [
            (Some(SlotOutcome::Loaded), decision(&all_open, &[LOADED, MISSING, REJECTED], &[UNREAD])),
            (Some(SlotOutcome::Rejected), decision(&all_open, &[LOADED, MISSING, REJECTED], &[UNREAD])),
            (Some(SlotOutcome::Missing), decision(&[REJECTED], &[LOADED, MISSING, REJECTED], &[UNREAD])),
            (Some(SlotOutcome::Unread), decision(&[], &[LOADED], &[MISSING, REJECTED, UNREAD])),
        ];
        for (old_slot, expected) in cases {
            assert_eq!(banks.decide(old_slot), expected, "old slot {old_slot:?}");
        }
    }

    #[test]
    fn a_bank_is_held_when_any_of_its_four_shorts_is() {
        assert_eq!(banks_of(short_bit(0) | short_bit(7) | short_bit(63)), 0b1000_0000_0000_0011);
        assert_eq!(banks_of(0), 0);
        assert_eq!(short_bit(64), 0);
    }
}
