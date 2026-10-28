pub const NO_SHORT_ADDRESS: u8 = 0xFF;

pub const DEVICE_QUERY_FIRST_OPCODE: u8 = 0x30;
pub const DEVICE_QUERY_ENTRIES: usize = 32;
pub const COMPARE_ENTRY: usize = DEVICE_QUERY_ENTRIES;
pub const VERIFY_SHORT_ADDRESS_ENTRY: usize = DEVICE_QUERY_ENTRIES + 1;
pub const QUERY_SHORT_ADDRESS_ENTRY: usize = DEVICE_QUERY_ENTRIES + 2;
pub const ANSWER_ENTRIES: usize = DEVICE_QUERY_ENTRIES + 3;

pub const ENTRY_ANSWER_MASK: u32 = 0xFF;
pub const ENTRY_PRESENT: u32 = 1 << 8;
pub const ENTRY_LAPSED_SHIFT: u32 = 16;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AnswerEntry(pub u32);

impl AnswerEntry {
    pub const SILENT: Self = Self(0);

    #[must_use]
    pub const fn always(answer: u8) -> Self {
        Self::by_lease(Some(answer), Some(answer))
    }

    #[must_use]
    pub const fn by_lease(alive: Option<u8>, lapsed: Option<u8>) -> Self {
        Self(Self::half(alive) | Self::half(lapsed) << ENTRY_LAPSED_SHIFT)
    }

    const fn half(answer: Option<u8>) -> u32 {
        match answer {
            Some(byte) => ENTRY_PRESENT | byte as u32,
            None => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnswerTableImage {
    pub short_address: u8,
    pub groups: u32,
    pub entries: [AnswerEntry; ANSWER_ENTRIES],
}

impl AnswerTableImage {
    pub const SILENT: Self = Self {
        short_address: NO_SHORT_ADDRESS,
        groups: 0,
        entries: [AnswerEntry::SILENT; ANSWER_ENTRIES],
    };

    pub fn set_device_query(&mut self, opcode: u8, entry: AnswerEntry) -> bool {
        let Some(index) = opcode.checked_sub(DEVICE_QUERY_FIRST_OPCODE) else {
            return false;
        };
        match self.entries.get_mut(usize::from(index)) {
            Some(slot) if usize::from(index) < DEVICE_QUERY_ENTRIES => {
                *slot = entry;
                true
            }
            _ => false,
        }
    }
}

impl Default for AnswerTableImage {
    fn default() -> Self {
        Self::SILENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_keeps_its_two_answers_apart() {
        let entry = AnswerEntry::by_lease(Some(0x12), None);
        assert_eq!(entry.0 & ENTRY_ANSWER_MASK, 0x12);
        assert_ne!(entry.0 & ENTRY_PRESENT, 0);
        assert_eq!(entry.0 >> ENTRY_LAPSED_SHIFT, 0);
        assert_eq!(AnswerEntry::always(0).0 >> ENTRY_LAPSED_SHIFT, ENTRY_PRESENT);
    }

    #[test]
    fn only_device_query_opcodes_have_a_device_query_entry() {
        let mut image = AnswerTableImage::SILENT;
        assert!(image.set_device_query(0x30, AnswerEntry::always(1)));
        assert!(image.set_device_query(0x4F, AnswerEntry::always(2)));
        assert!(!image.set_device_query(0x2F, AnswerEntry::always(3)));
        assert!(!image.set_device_query(0x50, AnswerEntry::always(4)));
        assert_eq!(image.entries[COMPARE_ENTRY], AnswerEntry::SILENT);
    }
}
