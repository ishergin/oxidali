use core::sync::atomic::{fence, AtomicU32, AtomicU8, Ordering};

use dali2rust_platform::answer_table::{
    AnswerTableImage, ANSWER_ENTRIES, COMPARE_ENTRY, DEVICE_QUERY_ENTRIES,
    DEVICE_QUERY_FIRST_OPCODE, ENTRY_ANSWER_MASK, ENTRY_LAPSED_SHIFT, ENTRY_PRESENT,
    NO_SHORT_ADDRESS, QUERY_SHORT_ADDRESS_ENTRY, VERIFY_SHORT_ADDRESS_ENTRY,
};

const COMMAND_BIT: u8 = 0x01;
const BROADCAST: u8 = 0xFF;
const BROADCAST_UNADDRESSED: u8 = 0xFD;
const SPECIAL_COMMAND: u8 = 0xC1;
const INSTANCE_DEVICE: u8 = 0xFE;
const SHORT_FORM_MASK: u8 = 0x80;
const GROUP_FORM_MASK: u8 = 0xC0;
const GROUP_FORM: u8 = 0x80;
const SHORT_ADDRESS_BITS: u8 = 0x3F;
const GROUP_BITS: u8 = 0x1F;
const SPECIAL_COMPARE: u8 = 0x03;
const SPECIAL_VERIFY_SHORT_ADDRESS: u8 = 0x09;
const SPECIAL_QUERY_SHORT_ADDRESS: u8 = 0x0A;
const NEVER_PUBLISHED: u32 = 0;

// IEC 62386-103 §7.2
#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub const fn answer_entry_index(frame: [u8; 3], short_address: u8, groups: u32) -> Option<usize> {
    let [address, instance, opcode] = frame;
    if address == SPECIAL_COMMAND {
        return match instance {
            SPECIAL_COMPARE => Some(COMPARE_ENTRY),
            SPECIAL_VERIFY_SHORT_ADDRESS => Some(VERIFY_SHORT_ADDRESS_ENTRY),
            SPECIAL_QUERY_SHORT_ADDRESS => Some(QUERY_SHORT_ADDRESS_ENTRY),
            _ => None,
        };
    }
    if address & COMMAND_BIT == 0 || instance != INSTANCE_DEVICE {
        return None;
    }
    if !addresses_the_device(address, short_address, groups) {
        return None;
    }
    let index = opcode.wrapping_sub(DEVICE_QUERY_FIRST_OPCODE);
    if opcode < DEVICE_QUERY_FIRST_OPCODE || index as usize >= DEVICE_QUERY_ENTRIES {
        return None;
    }
    Some(index as usize)
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
const fn addresses_the_device(address: u8, short_address: u8, groups: u32) -> bool {
    match address {
        BROADCAST => true,
        BROADCAST_UNADDRESSED => short_address == NO_SHORT_ADDRESS,
        _ if address & SHORT_FORM_MASK == 0 => (address >> 1) & SHORT_ADDRESS_BITS == short_address,
        _ if address & GROUP_FORM_MASK == GROUP_FORM => {
            match groups.checked_shr(((address >> 1) & GROUP_BITS) as u32) {
                Some(bits) => bits & 1 != 0,
                None => false,
            }
        }
        _ => false,
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub const fn entry_answer(
    index: usize,
    word: u32,
    frame: [u8; 3],
    short_address: u8,
    lease_alive: bool,
) -> Option<u8> {
    let [_, _, data] = frame;
    if index == VERIFY_SHORT_ADDRESS_ENTRY && data != short_address {
        return None;
    }
    let half = if lease_alive { word } else { word >> ENTRY_LAPSED_SHIFT };
    if half & ENTRY_PRESENT == 0 {
        return None;
    }
    Some((half & ENTRY_ANSWER_MASK) as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLookup {
    NotPublished,
    NotAddressed,
    Behind,
    Torn,
    Silent,
    LeaseLapsed,
    Answer(u8),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TableRefusalCounts {
    pub lease_lapsed: u32,
    pub behind: u32,
    pub torn: u32,
}

struct TableRefusals {
    lease_lapsed: AtomicU32,
    behind: AtomicU32,
    torn: AtomicU32,
}

impl TableRefusals {
    const fn new() -> Self {
        Self {
            lease_lapsed: AtomicU32::new(0),
            behind: AtomicU32::new(0),
            torn: AtomicU32::new(0),
        }
    }
}

#[derive(Clone, Copy)]
struct LiveRead {
    torn: bool,
    accounts_for: u8,
    short_address: u8,
    index: Option<usize>,
    word: u32,
}

struct TableCopy {
    generation: AtomicU32,
    accounts_for: AtomicU8,
    short_address: AtomicU8,
    groups: AtomicU32,
    entries: [AtomicU32; ANSWER_ENTRIES],
}

impl TableCopy {
    const fn new() -> Self {
        Self {
            generation: AtomicU32::new(NEVER_PUBLISHED),
            accounts_for: AtomicU8::new(0),
            short_address: AtomicU8::new(NO_SHORT_ADDRESS),
            groups: AtomicU32::new(0),
            entries: [const { AtomicU32::new(0) }; ANSWER_ENTRIES],
        }
    }

    fn write(&self, image: &AnswerTableImage, accounts_for: u8) {
        let generation = self.generation.load(Ordering::Relaxed);
        self.generation.store(generation.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.accounts_for.store(accounts_for, Ordering::Relaxed);
        self.short_address.store(image.short_address, Ordering::Relaxed);
        self.groups.store(image.groups, Ordering::Relaxed);
        for (slot, entry) in self.entries.iter().zip(image.entries.iter()) {
            slot.store(entry.0, Ordering::Relaxed);
        }
        self.generation.store(generation.wrapping_add(2), Ordering::Release);
    }
}

pub struct AnswerTableCell {
    live: AtomicU8,
    copies: [TableCopy; 2],
    lease_until_tick: AtomicU32,
    refusals: TableRefusals,
}

impl AnswerTableCell {
    pub const fn new() -> Self {
        Self {
            live: AtomicU8::new(0),
            copies: [TableCopy::new(), TableCopy::new()],
            lease_until_tick: AtomicU32::new(0),
            refusals: TableRefusals::new(),
        }
    }

    pub fn publish(&self, image: &AnswerTableImage, accounts_for: u8) {
        let next = (self.live.load(Ordering::Relaxed) ^ 1) & 1;
        if let Some(copy) = self.copies.get(usize::from(next)) {
            copy.write(image, accounts_for);
            self.live.store(next, Ordering::Release);
        }
    }

    pub fn set_lease_until_tick(&self, tick: u32) {
        self.lease_until_tick.store(tick, Ordering::Release);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn lease_alive(&self, now_tick: u32) -> bool {
        (self.lease_until_tick.load(Ordering::Acquire).wrapping_sub(now_tick) as i32) > 0
    }

    pub fn lookup(&self, frame: [u8; 3], previous_capture: u8, now_tick: u32) -> TableLookup {
        self.decide(frame, previous_capture, now_tick, &TableRefusals::new())
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn answer(&self, frame: [u8; 3], previous_capture: u8, now_tick: u32) -> Option<u8> {
        match self.decide(frame, previous_capture, now_tick, &self.refusals) {
            TableLookup::Answer(answer) => Some(answer),
            _ => None,
        }
    }

    pub fn refusal_counts(&self) -> TableRefusalCounts {
        TableRefusalCounts {
            lease_lapsed: self.refusals.lease_lapsed.load(Ordering::Relaxed),
            behind: self.refusals.behind.load(Ordering::Relaxed),
            torn: self.refusals.torn.load(Ordering::Relaxed),
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn decide(
        &self,
        frame: [u8; 3],
        previous_capture: u8,
        now_tick: u32,
        refusals: &TableRefusals,
    ) -> TableLookup {
        let Some(read) = self.read_live(frame) else {
            return TableLookup::NotPublished;
        };
        if read.torn {
            refusals.torn.fetch_add(1, Ordering::Relaxed);
            return TableLookup::Torn;
        }
        let Some(index) = read.index else {
            return TableLookup::NotAddressed;
        };
        if read.accounts_for != previous_capture {
            refusals.behind.fetch_add(1, Ordering::Relaxed);
            return TableLookup::Behind;
        }
        let lease_alive = self.lease_alive(now_tick);
        let (word, short_address) = (read.word, read.short_address);
        match entry_answer(index, word, frame, short_address, lease_alive) {
            Some(answer) => TableLookup::Answer(answer),
            None if !lease_alive && entry_answer(index, word, frame, short_address, true).is_some() => {
                refusals.lease_lapsed.fetch_add(1, Ordering::Relaxed);
                TableLookup::LeaseLapsed
            }
            None => TableLookup::Silent,
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn read_live(&self, frame: [u8; 3]) -> Option<LiveRead> {
        let copy = self.copies.get(usize::from(self.live.load(Ordering::Acquire) & 1))?;
        let before = copy.generation.load(Ordering::Acquire);
        if before == NEVER_PUBLISHED {
            return None;
        }
        let accounts_for = copy.accounts_for.load(Ordering::Relaxed);
        let short_address = copy.short_address.load(Ordering::Relaxed);
        let groups = copy.groups.load(Ordering::Relaxed);
        let index = answer_entry_index(frame, short_address, groups);
        let word = match index {
            Some(index) => match copy.entries.get(index) {
                Some(entry) => entry.load(Ordering::Relaxed),
                None => 0,
            },
            None => 0,
        };
        fence(Ordering::Acquire);
        let torn = before & 1 != 0 || copy.generation.load(Ordering::Relaxed) != before;
        Some(LiveRead { torn, accounts_for, short_address, index, word })
    }
}

impl Default for AnswerTableCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_platform::answer_table::AnswerEntry;

    const QACE: u8 = 0x3D;
    const NOW: u32 = 1_000;

    fn image(short_address: u8, groups: u32) -> AnswerTableImage {
        let mut image = AnswerTableImage::SILENT;
        image.short_address = short_address;
        image.groups = groups;
        assert!(image.set_device_query(QACE, AnswerEntry::by_lease(Some(0xFF), None)));
        image
    }

    fn published(image: &AnswerTableImage, accounts_for: u8) -> AnswerTableCell {
        let cell = AnswerTableCell::new();
        cell.publish(image, accounts_for);
        cell.set_lease_until_tick(NOW + 100);
        cell
    }

    #[test]
    fn the_device_answers_its_short_address_its_groups_and_broadcast() {
        let cell = published(&image(7, 1 << 3), 4);
        for address in [0x0F, 0x87, 0xFF] {
            assert_eq!(cell.lookup([address, 0xFE, QACE], 4, NOW), TableLookup::Answer(0xFF));
        }
        for address in [0x11, 0x85, 0xFD, 0xC3] {
            assert_eq!(cell.lookup([address, 0xFE, QACE], 4, NOW), TableLookup::NotAddressed);
        }
    }

    #[test]
    fn the_unaddressed_broadcast_reaches_only_a_device_without_a_short_address() {
        let cell = published(&image(NO_SHORT_ADDRESS, 0), 0);
        assert_eq!(cell.lookup([0xFD, 0xFE, QACE], 0, NOW), TableLookup::Answer(0xFF));
        assert_eq!(cell.lookup([0x7F, 0xFE, QACE], 0, NOW), TableLookup::NotAddressed);
    }

    #[test]
    fn short_address_zero_is_an_address() {
        let cell = published(&image(0, 0), 0);
        assert_eq!(cell.lookup([0x01, 0xFE, QACE], 0, NOW), TableLookup::Answer(0xFF));
        assert_eq!(cell.lookup([0xFD, 0xFE, QACE], 0, NOW), TableLookup::NotAddressed);
    }

    #[test]
    fn an_event_frame_or_an_instance_command_is_not_a_device_query() {
        let cell = published(&image(7, 0), 0);
        assert_eq!(cell.lookup([0x0E, 0xFE, QACE], 0, NOW), TableLookup::NotAddressed);
        assert_eq!(cell.lookup([0x0F, 0x00, QACE], 0, NOW), TableLookup::NotAddressed);
        assert_eq!(cell.lookup([0x0F, 0xFE, 0x2F], 0, NOW), TableLookup::NotAddressed);
        assert_eq!(cell.lookup([0x0F, 0xFE, 0x50], 0, NOW), TableLookup::NotAddressed);
    }

    #[test]
    fn a_table_behind_the_previous_capture_answers_nothing() {
        let cell = published(&image(7, 0), 4);
        assert_eq!(cell.lookup([0xFF, 0xFE, QACE], 5, NOW), TableLookup::Behind);
        assert_eq!(cell.lookup([0x11, 0xFE, QACE], 5, NOW), TableLookup::NotAddressed);
    }

    #[test]
    fn nothing_answers_before_the_first_table() {
        let cell = AnswerTableCell::new();
        cell.set_lease_until_tick(NOW + 100);
        assert_eq!(cell.lookup([0xFF, 0xFE, QACE], 0, NOW), TableLookup::NotPublished);
    }

    #[test]
    fn a_lapsed_lease_selects_the_lapsed_answer() {
        let cell = published(&image(7, 0), 0);
        assert_eq!(cell.lookup([0xFF, 0xFE, QACE], 0, NOW + 100), TableLookup::LeaseLapsed);
        assert_eq!(cell.lookup([0xFF, 0xFE, QACE], 0, NOW + 99), TableLookup::Answer(0xFF));
        assert!(!AnswerTableCell::new().lease_alive(NOW));
    }

    #[test]
    fn the_lease_survives_the_tick_counter_wrapping() {
        let cell = AnswerTableCell::new();
        cell.set_lease_until_tick(5);
        assert!(cell.lease_alive(u32::MAX - 5));
        assert!(!cell.lease_alive(6));
    }

    #[test]
    fn verify_short_address_answers_only_its_own_address() {
        let mut image = image(9, 0);
        image.entries[VERIFY_SHORT_ADDRESS_ENTRY] = AnswerEntry::always(0xFF);
        let cell = published(&image, 0);
        assert_eq!(cell.lookup([0xC1, 0x09, 9], 0, NOW), TableLookup::Answer(0xFF));
        assert_eq!(cell.lookup([0xC1, 0x09, 8], 0, NOW), TableLookup::Silent);
        assert_eq!(cell.lookup([0xC1, 0x0B, 9], 0, NOW), TableLookup::NotAddressed);
    }

    #[test]
    fn a_republished_table_replaces_the_one_before() {
        let cell = published(&image(7, 0), 0);
        cell.publish(&image(8, 0), 1);
        assert_eq!(cell.lookup([0x0F, 0xFE, QACE], 1, NOW), TableLookup::NotAddressed);
        assert_eq!(cell.lookup([0x11, 0xFE, QACE], 1, NOW), TableLookup::Answer(0xFF));
        cell.publish(&image(7, 0), 2);
        assert_eq!(cell.lookup([0x0F, 0xFE, QACE], 2, NOW), TableLookup::Answer(0xFF));
    }

    #[test]
    fn a_copy_caught_mid_write_reads_as_torn() {
        let cell = published(&image(7, 0), 0);
        let live = usize::from(cell.live.load(Ordering::Relaxed));
        cell.copies[live].generation.fetch_add(1, Ordering::Relaxed);
        assert_eq!(cell.lookup([0xFF, 0xFE, QACE], 0, NOW), TableLookup::Torn);
    }

    #[test]
    fn the_interrupt_s_answer_counts_each_refusal_and_a_task_lookup_counts_none() {
        let cell = published(&image(7, 0), 0);
        let qace = [0xFF, 0xFE, QACE];
        assert_eq!(cell.answer(qace, 0, NOW), Some(0xFF));
        assert_eq!(cell.lookup(qace, 1, NOW), TableLookup::Behind);
        assert_eq!(cell.answer(qace, 1, NOW), None);
        cell.set_lease_until_tick(NOW);
        assert_eq!(cell.lookup(qace, 0, NOW), TableLookup::LeaseLapsed);
        assert_eq!(cell.answer(qace, 0, NOW), None);
        let live = usize::from(cell.live.load(Ordering::Relaxed));
        cell.copies[live].generation.fetch_add(1, Ordering::Relaxed);
        assert_eq!(cell.lookup(qace, 0, NOW), TableLookup::Torn);
        assert_eq!(cell.answer(qace, 0, NOW), None);
        let once_each = TableRefusalCounts { lease_lapsed: 1, behind: 1, torn: 1 };
        assert_eq!(cell.refusal_counts(), once_each);
    }
}
