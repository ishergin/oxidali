// DiiA (SW)098bp §7.2.7
pub const BUS_UNIT_CONFIGURATION_OFFSET: u8 = 0x1B;
pub const IMPLEMENTED_PARTS_OFFSET: u8 = 0x1C;

pub const BANK0_EXTENDED_LAST_OFFSET: u8 = IMPLEMENTED_PARTS_OFFSET;

const FIRST_IMPLEMENTED_PART: u16 = 150;
const IMPLEMENTED_PART_BITS: u8 = 5;
const IMPLEMENTED_PARTS_DEFINED_MASK: u8 = (1 << IMPLEMENTED_PART_BITS) - 1;

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

// DiiA 098bp Table 5
const fn defined_row_label(row: u8) -> &'static str {
    match row {
        0 => "207 LED, 1 logical unit",
        1 => "207 LED, 2 logical units",
        2 => "207 LED, 3 logical units",
        3 => "207 LED, 4 logical units",
        4 => "209 tunable white, 1 logical unit",
        5 => "209 tunable white, 2 logical units",
        6 => "209 RGBWAF",
        7 => "209 tunable white, RGBWAF and xy",
        _ => "207 LED and 209 tunable white, one logical unit each",
    }
}

impl BusUnitConfiguration {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Defined(row) => defined_row_label(row),
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
pub struct ImplementedParts(u8);

impl ImplementedParts {
    pub const fn from_byte(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub const fn in_range(self) -> bool {
        self.0 & !IMPLEMENTED_PARTS_DEFINED_MASK == 0
    }

    pub fn parts(self) -> impl Iterator<Item = u16> {
        let raw = if self.in_range() { self.0 } else { 0 };
        (0..IMPLEMENTED_PART_BITS)
            .filter(move |bit| raw & (1 << bit) != 0)
            .map(|bit| FIRST_IMPLEMENTED_PART + u16::from(bit))
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
    fn rows_zero_to_eight_carry_their_table_5_names() {
        let names: Vec<&str> = (0u8..=8)
            .map(|raw| {
                assert_eq!(BusUnitConfiguration::from_byte(raw), BusUnitConfiguration::Defined(raw));
                BusUnitConfiguration::from_byte(raw).label()
            })
            .collect();
        assert_eq!(names[0], "207 LED, 1 logical unit");
        assert_eq!(names[3], "207 LED, 4 logical units");
        assert_eq!(names[5], "209 tunable white, 2 logical units");
        assert_eq!(names[6], "209 RGBWAF");
        assert_eq!(names[8], "207 LED and 209 tunable white, one logical unit each");
        let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(distinct.len(), names.len(), "every Table 5 row has its own name");
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
        assert_eq!(BusUnitConfiguration::from_byte(0).label(), "207 LED, 1 logical unit");
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
    fn bit_x_of_the_byte_is_part_15x() {
        for bit in 0..IMPLEMENTED_PART_BITS {
            let parts = ImplementedParts::from_byte(1 << bit);
            assert!(parts.in_range());
            assert_eq!(parts.parts().collect::<Vec<_>>(), vec![150 + u16::from(bit)]);
        }
    }

    #[test]
    fn a_byte_lists_every_part_it_sets() {
        let parts = ImplementedParts::from_byte(0b0000_0101);
        assert_eq!(parts.raw(), 0x05);
        assert_eq!(parts.parts().collect::<Vec<_>>(), vec![150, 152]);
        assert_eq!(
            ImplementedParts::from_byte(0b0001_1111).parts().collect::<Vec<_>>(),
            vec![150, 151, 152, 153, 154]
        );
        assert_eq!(ImplementedParts::from_byte(0).parts().count(), 0);
    }

    #[test]
    fn a_byte_outside_its_range_claims_no_part() {
        for raw in [0x20, 0x21, 0x80, 0xFF] {
            let parts = ImplementedParts::from_byte(raw);
            assert!(!parts.in_range(), "0x{raw:02X} sets a bit Table 4 leaves zero");
            assert_eq!(parts.raw(), raw);
            assert_eq!(parts.parts().count(), 0, "0x{raw:02X}");
        }
    }
}
