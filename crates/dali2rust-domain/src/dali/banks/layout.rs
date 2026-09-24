use super::ValueWidth;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankField {
    pub offset: u8,
    pub width: ValueWidth,
}

const fn f(offset: u8, width: ValueWidth) -> BankField {
    BankField { offset, width }
}

pub const BANK_META_LEN: u16 = 3;
pub const BANK_LOCK_BYTE: u8 = 0x02;
pub const BANK_VERSION_OFFSET: u8 = 0x03;

use ValueWidth::{I8, U16, U24, U32, U48, U8};

const META: [BankField; 3] = [f(0x00, U8), f(0x01, U8), f(0x02, U8)];

// DiiA 252 §9.2.9, §9.2.10, §9.2.11
pub const BANK_ENERGY: [BankField; 8] = [
    META[0],
    META[1],
    META[2],
    f(0x03, U8),
    f(0x04, I8),
    f(0x05, U48),
    f(0x0B, I8),
    f(0x0C, U32),
];

// DiiA 253 §9.2.16
pub const BANK_GEAR_DIAGNOSTICS: [BankField; 21] = [
    META[0],
    META[1],
    META[2],
    f(0x03, U8),
    f(0x04, U32),
    f(0x08, U24),
    f(0x0B, U16),
    f(0x0D, U8),
    f(0x0E, U8),
    f(0x0F, U8),
    f(0x10, U8),
    f(0x11, U8),
    f(0x12, U8),
    f(0x13, U8),
    f(0x14, U8),
    f(0x15, U8),
    f(0x16, U8),
    f(0x17, U8),
    f(0x18, U8),
    f(0x19, U8),
    f(0x1A, U8),
];

pub const BANK_GEAR_DIAGNOSTICS_TAIL: [BankField; 2] = [
    f(0x1B, U8),
    f(0x1C, U8),
];

// DiiA 253 §9.2.17
pub const BANK_SOURCE_DIAGNOSTICS: [BankField; 20] = [
    META[0],
    META[1],
    META[2],
    f(0x03, U8),
    f(0x04, U24),
    f(0x07, U24),
    f(0x0A, U32),
    f(0x0E, U32),
    f(0x12, U16),
    f(0x14, U16),
    f(0x16, U8),
    f(0x17, U8),
    f(0x18, U8),
    f(0x19, U8),
    f(0x1A, U8),
    f(0x1B, U8),
    f(0x1C, U8),
    f(0x1D, U8),
    f(0x1E, U8),
    f(0x1F, U8),
];

pub const BANK_SOURCE_DIAGNOSTICS_TAIL: [BankField; 1] = [f(0x20, U8)];

// DiiA 253 §9.2.18
pub const BANK_LUMINAIRE: [BankField; 7] = [
    META[0],
    META[1],
    META[2],
    f(0x03, U8),
    f(0x04, U8),
    f(0x05, U8),
    f(0x06, U16),
];

pub const DEVICE_TYPE_ENERGY: u8 = 51;
pub const DEVICE_TYPE_DIAGNOSTICS: u8 = 52;

pub const BANK_ACTIVE_ENERGY: u8 = 202;
pub const BANK_APPARENT_ENERGY: u8 = 203;
pub const BANK_LOADSIDE_ENERGY: u8 = 204;
pub const BANK_CONTROL_GEAR_DIAGNOSTICS: u8 = 205;
pub const BANK_LIGHT_SOURCE_DIAGNOSTICS: u8 = 206;
pub const BANK_LUMINAIRE_MAINTENANCE: u8 = 207;

pub const fn bank_last_offset(bank: u8) -> Option<u8> {
    match bank {
        BANK_ACTIVE_ENERGY | BANK_APPARENT_ENERGY | BANK_LOADSIDE_ENERGY => Some(0x0F),
        BANK_CONTROL_GEAR_DIAGNOSTICS => Some(0x1C),
        BANK_LIGHT_SOURCE_DIAGNOSTICS => Some(0x20),
        BANK_LUMINAIRE_MAINTENANCE => Some(0x07),
        _ => None,
    }
}

const GEAR_DIAGNOSTICS_ALL: [BankField; 23] = concat_gear();
const SOURCE_DIAGNOSTICS_ALL: [BankField; 21] = concat_source();

const fn concat_gear() -> [BankField; 23] {
    let mut out = [f(0, U8); 23];
    let mut i = 0;
    while i < BANK_GEAR_DIAGNOSTICS.len() {
        out[i] = BANK_GEAR_DIAGNOSTICS[i];
        i += 1;
    }
    let mut j = 0;
    while j < BANK_GEAR_DIAGNOSTICS_TAIL.len() {
        out[i + j] = BANK_GEAR_DIAGNOSTICS_TAIL[j];
        j += 1;
    }
    out
}

const fn concat_source() -> [BankField; 21] {
    let mut out = [f(0, U8); 21];
    let mut i = 0;
    while i < BANK_SOURCE_DIAGNOSTICS.len() {
        out[i] = BANK_SOURCE_DIAGNOSTICS[i];
        i += 1;
    }
    let mut j = 0;
    while j < BANK_SOURCE_DIAGNOSTICS_TAIL.len() {
        out[i + j] = BANK_SOURCE_DIAGNOSTICS_TAIL[j];
        j += 1;
    }
    out
}

pub fn bank_field_map(bank: u8) -> &'static [BankField] {
    match bank {
        super::part251::BANK_LUMINAIRE_INFO => &super::part251::BANK_LUMINAIRE_INFO_MAP,
        BANK_ACTIVE_ENERGY | BANK_APPARENT_ENERGY | BANK_LOADSIDE_ENERGY => &BANK_ENERGY,
        BANK_CONTROL_GEAR_DIAGNOSTICS => &GEAR_DIAGNOSTICS_ALL,
        BANK_LIGHT_SOURCE_DIAGNOSTICS => &SOURCE_DIAGNOSTICS_ALL,
        BANK_LUMINAIRE_MAINTENANCE => &BANK_LUMINAIRE,
        _ => &[],
    }
}

pub fn field_containing(bank: u8, offset: u16) -> Option<BankField> {
    bank_field_map(bank).iter().copied().find(|field| {
        let start = u16::from(field.offset);
        offset >= start && offset < start + field.width.bytes() as u16
    })
}

pub fn is_field_boundary(bank: u8, offset: u16) -> bool {
    field_containing(bank, offset).is_none_or(|field| u16::from(field.offset) == offset)
}

// DiiA 252 §9.2.2
pub fn chunk_len(bank: u8, offset: u16, remaining: u16, budget: u16) -> u16 {
    let map = bank_field_map(bank);
    if map.is_empty() {
        return budget.min(remaining);
    }
    let mut taken: u16 = 0;
    for field in map {
        let start = u16::from(field.offset);
        if start < offset {
            continue;
        }
        if start != offset + taken {
            break;
        }
        let width = field.width.bytes() as u16;
        if taken + width > remaining {
            break;
        }
        if taken > 0 && taken + width > budget {
            break;
        }
        taken += width;
        if taken >= budget {
            break;
        }
    }
    if taken == 0 {
        budget.min(remaining)
    } else {
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapped_bank_is_contiguous_and_ends_where_it_declares() {
        for bank in [202u8, 203, 204, 205, 206, 207] {
            let map = bank_field_map(bank);
            assert!(!map.is_empty(), "bank {bank} has a map");
            let mut next = 0u16;
            for field in map {
                assert_eq!(
                    u16::from(field.offset),
                    next,
                    "bank {bank} has a hole or an overlap at {:#04x}",
                    field.offset
                );
                next += field.width.bytes() as u16;
            }
            assert_eq!(
                next - 1,
                u16::from(bank_last_offset(bank).expect("declared")),
                "bank {bank} map must cover exactly up to its last addressable location"
            );
        }
    }

    #[test]
    fn chunks_never_split_a_latched_value() {
        assert_eq!(chunk_len(202, 0x03, 13, 5), 2, "0x03 + 0x04, then stop");
        assert_eq!(
            chunk_len(202, 0x05, 11, 5),
            6,
            "the energy value is wider than the budget and goes alone"
        );
        assert_eq!(chunk_len(202, 0x0B, 5, 5), 5, "scale byte + the u32 power");
    }

    #[test]
    fn a_chunk_starting_inside_a_field_falls_back_and_would_split_the_latch() {
        assert!(!is_field_boundary(202, 0x06));
        let field = field_containing(202, 0x06).expect("inside the energy value");
        assert_eq!(field.offset, 0x05, "the value this chunk would enter halfway");

        assert_eq!(
            chunk_len(202, 0x06, 10, 5),
            5,
            "the flat budget, not a field-aligned length"
        );

        let taken = chunk_len(202, 0x07, 10, 5);
        assert_eq!(taken, 5);
        let end_of_field = u16::from(field.offset) + field.width.bytes() as u16;
        assert!(
            0x07 + taken > end_of_field,
            "a chunk from 0x07 must reach past the value's end at {end_of_field:#04x}"
        );
    }

    #[test]
    fn a_field_boundary_is_a_field_start_or_an_unmapped_offset() {
        assert!(is_field_boundary(202, 0x05), "the energy value starts here");
        assert!(is_field_boundary(202, 0x0B), "so does the power scale factor");
        assert!(!is_field_boundary(202, 0x08), "mid-energy");
        assert!(is_field_boundary(0, 0x07), "bank 0 has no map at all");
        assert!(
            is_field_boundary(1, 0x40),
            "bank 1's map ends at 0x24; the string regions are unmapped"
        );
    }

    #[test]
    fn an_unmapped_bank_keeps_the_flat_budget() {
        assert_eq!(chunk_len(0, 3, 24, 5), 5);
        assert_eq!(chunk_len(1, 3, 2, 5), 2, "clamped by what is left");
    }

    #[test]
    fn a_reread_restarts_the_field_the_offset_landed_in() {
        assert_eq!(
            field_containing(202, 0x08),
            Some(BankField { offset: 0x05, width: ValueWidth::U48 })
        );
        assert_eq!(
            field_containing(206, 0x0C),
            Some(BankField { offset: 0x0A, width: ValueWidth::U32 }),
            "LightSourceOnTimeResettable spans 0x0A..0x0D"
        );
        assert_eq!(field_containing(0, 0x05), None, "bank 0 is ROM, no latch");
    }

    #[test]
    fn bank_206_short_circuit_precedes_open_circuit_by_address() {
        let map = bank_field_map(206);
        let offsets: Vec<u8> = map.iter().map(|f| f.offset).collect();
        let short = offsets.iter().position(|o| *o == 0x18).expect("0x18");
        let open = offsets.iter().position(|o| *o == 0x1A).expect("0x1A");
        assert!(short < open);
    }
}
