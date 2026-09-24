// DiiA (SW)098bp §7.2.7
pub const BUS_UNIT_CONFIGURATION_OFFSET: u8 = 0x1B;
pub const IMPLEMENTED_PARTS_BASE_OFFSET: u8 = 0x1C;
pub const IMPLEMENTED_PARTS_EXTENSION_OFFSET: u8 = 0x1D;

pub const BANK0_EXTENDED_LAST_OFFSET: u8 = IMPLEMENTED_PARTS_EXTENSION_OFFSET;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusUnitConfiguration {
    Defined(u8),
    EmergencyType(EmergencyGearType),
    Reserved(u8),
    ManufacturerSpecific(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyGearType {
    A,
    B,
    C,
    D,
}

impl EmergencyGearType {
    pub const fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::D => 'D',
        }
    }
}

impl BusUnitConfiguration {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Defined(_) => "unnamed row",
            Self::EmergencyType(_) => "Part 202 emergency control gear",
            Self::Reserved(_) => "reserved",
            Self::ManufacturerSpecific(_) => "manufacturer-specific",
        }
    }

    pub const fn emergency_letter(self) -> Option<char> {
        match self {
            Self::EmergencyType(t) => Some(t.letter()),
            _ => None,
        }
    }
}

impl BusUnitConfiguration {
    pub const fn from_byte(raw: u8) -> Self {
        match raw {
            0..=8 => Self::Defined(raw),
            9 => Self::EmergencyType(EmergencyGearType::A),
            10 => Self::EmergencyType(EmergencyGearType::B),
            11 => Self::EmergencyType(EmergencyGearType::C),
            12 => Self::EmergencyType(EmergencyGearType::D),
            13..=191 => Self::Reserved(raw),
            _ => Self::ManufacturerSpecific(raw),
        }
    }

    pub const fn raw(self) -> u8 {
        match self {
            Self::Defined(v) | Self::Reserved(v) | Self::ManufacturerSpecific(v) => v,
            Self::EmergencyType(t) => match t {
                EmergencyGearType::A => 9,
                EmergencyGearType::B => 10,
                EmergencyGearType::C => 11,
                EmergencyGearType::D => 12,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImplementedParts {
    raw: u16,
    bytes: u8,
}

impl ImplementedParts {
    pub const fn from_base(base: u8) -> Self {
        Self {
            raw: base as u16,
            bytes: 1,
        }
    }

    pub const fn from_both(base: u8, extension: u8) -> Self {
        Self {
            raw: ((extension as u16) << 8) | base as u16,
            bytes: 2,
        }
    }

    pub const fn raw(self) -> u16 {
        self.raw
    }

    pub const fn byte_count(self) -> u8 {
        self.bytes
    }

    pub const fn known_bits(self) -> u8 {
        self.bytes * 8
    }

    pub const fn implements_part(self, part: u16) -> Option<bool> {
        if part < 151 {
            return None;
        }
        let bit = part - 151;
        if bit >= self.known_bits() as u16 {
            return None;
        }
        Some(self.raw & (1u16 << bit) != 0)
    }

    pub fn parts(self) -> impl Iterator<Item = u16> {
        let raw = self.raw;
        let bits = self.known_bits() as u16;
        (0..bits).filter_map(move |bit| (raw & (1u16 << bit) != 0).then_some(151 + bit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_5_rows_that_are_legible_are_the_only_ones_named() {
        assert_eq!(
            BusUnitConfiguration::from_byte(9),
            BusUnitConfiguration::EmergencyType(EmergencyGearType::A)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(12),
            BusUnitConfiguration::EmergencyType(EmergencyGearType::D)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(13),
            BusUnitConfiguration::Reserved(13)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(191),
            BusUnitConfiguration::Reserved(191)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(192),
            BusUnitConfiguration::ManufacturerSpecific(192)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(254),
            BusUnitConfiguration::ManufacturerSpecific(254)
        );
        assert_eq!(
            BusUnitConfiguration::from_byte(255),
            BusUnitConfiguration::ManufacturerSpecific(255)
        );
    }

    #[test]
    fn rows_zero_to_eight_are_reported_but_not_named() {
        for raw in 0u8..=8 {
            assert_eq!(
                BusUnitConfiguration::from_byte(raw),
                BusUnitConfiguration::Defined(raw)
            );
        }
    }

    #[test]
    fn every_byte_is_labelled_and_only_the_emergency_rows_have_a_letter() {
        for raw in 0u8..=255 {
            let class = BusUnitConfiguration::from_byte(raw);
            assert!(!class.label().is_empty(), "byte {raw} has no label");
            let expected_letter = match raw {
                9 => Some('A'),
                10 => Some('B'),
                11 => Some('C'),
                12 => Some('D'),
                _ => None,
            };
            assert_eq!(class.emergency_letter(), expected_letter, "byte {raw}");
        }
        assert_eq!(BusUnitConfiguration::from_byte(0).label(), "unnamed row");
        assert_eq!(BusUnitConfiguration::from_byte(9).label(), "Part 202 emergency control gear");
        assert_eq!(BusUnitConfiguration::from_byte(13).label(), "reserved");
        assert_eq!(BusUnitConfiguration::from_byte(192).label(), "manufacturer-specific");
    }

    #[test]
    fn every_classification_keeps_the_raw_byte() {
        for raw in 0u8..=255 {
            assert_eq!(BusUnitConfiguration::from_byte(raw).raw(), raw);
        }
    }

    #[test]
    fn a_one_byte_mask_leaves_the_high_parts_unknown() {
        let parts = ImplementedParts::from_base(0b0000_0101);
        assert_eq!(parts.byte_count(), 1);
        assert_eq!(parts.implements_part(151), Some(true));
        assert_eq!(parts.implements_part(152), Some(false));
        assert_eq!(parts.implements_part(153), Some(true));
        assert_eq!(parts.implements_part(158), Some(false));
        assert_eq!(parts.implements_part(159), None, "bit 8 was never read");
        assert_eq!(parts.parts().collect::<Vec<_>>(), vec![151, 153]);
    }

    #[test]
    fn a_two_byte_mask_extends_the_base_rather_than_displacing_it() {
        let parts = ImplementedParts::from_both(0x02, 0x01);
        assert_eq!(parts.raw(), 0x0102, "0x1C is the low byte of the word");
        assert_eq!(parts.known_bits(), 16);
        assert_eq!(parts.implements_part(152), Some(true), "bit 1 of 0x1C");
        assert_eq!(parts.implements_part(159), Some(true), "bit 0 of 0x1D");
        assert_eq!(parts.parts().collect::<Vec<_>>(), vec![152, 159]);
    }

    #[test]
    fn a_bit_of_a_location_names_the_same_part_however_many_bytes_answered() {
        for bit in 0..8u16 {
            let base = 1u8 << bit;
            let alone = ImplementedParts::from_base(base);
            let extended = ImplementedParts::from_both(base, 0x00);
            let part = 151 + bit;
            assert_eq!(
                alone.implements_part(part),
                Some(true),
                "0x1C bit {bit} must be Part {part} when it is the only byte"
            );
            assert_eq!(
                extended.implements_part(part),
                Some(true),
                "0x1C bit {bit} must still be Part {part} when 0x1D also answered"
            );
            assert_eq!(alone.parts().collect::<Vec<_>>(), vec![part]);
            assert_eq!(extended.parts().collect::<Vec<_>>(), vec![part]);
        }
    }

    #[test]
    fn the_extension_byte_carries_parts_159_to_166() {
        for bit in 0..8u16 {
            let parts = ImplementedParts::from_both(0x00, 1u8 << bit);
            let part = 159 + bit;
            assert_eq!(parts.implements_part(part), Some(true));
            assert_eq!(parts.parts().collect::<Vec<_>>(), vec![part]);
        }
        assert_eq!(ImplementedParts::from_both(0xFF, 0xFF).implements_part(167), None);
    }

    #[test]
    fn part_numbers_below_151_are_not_in_this_mask() {
        let parts = ImplementedParts::from_base(0xFF);
        assert_eq!(parts.implements_part(150), None);
        assert_eq!(parts.implements_part(102), None);
    }
}
