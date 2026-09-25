pub mod bus_unit;
pub mod layout;
pub mod part251;

pub use bus_unit::{
    BusUnitConfiguration, EmergencyGearType, ImplementedParts, BANK0_EXTENDED_LAST_OFFSET,
    BUS_UNIT_CONFIGURATION_OFFSET, IMPLEMENTED_PARTS_OFFSET,
};
pub use layout::{
    bank_field_map, bank_last_offset, chunk_len, field_containing, is_field_boundary, BankField,
    BANK_ACTIVE_ENERGY,
    BANK_APPARENT_ENERGY, BANK_CONTROL_GEAR_DIAGNOSTICS, BANK_LIGHT_SOURCE_DIAGNOSTICS,
    BANK_LOADSIDE_ENERGY, BANK_LOCK_BYTE, BANK_LUMINAIRE_MAINTENANCE, BANK_META_LEN,
    BANK_VERSION_OFFSET, DEVICE_TYPE_DIAGNOSTICS, DEVICE_TYPE_ENERGY,
};
pub use part251::{
    CctReading, LightDistributionType, LuminaireFormat, TextRegion, BANK1_BASE_LAST_OFFSET,
    BANK1_FORMAT_GATE_LAST_OFFSET, BANK_LUMINAIRE_INFO, CONTENT_FORMAT_ID_OFFSET,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueWidth {
    U8,
    U16,
    U24,
    U32,
    U48,
    I8,
    I16,
    I32,
}

impl ValueWidth {
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U24 => 3,
            Self::U32 | Self::I32 => 4,
            Self::U48 => 6,
        }
    }

    // DiiA 252 Table 2
    pub const fn mask(self) -> u64 {
        match self {
            Self::U8 => 0xFF,
            Self::U16 => 0xFFFF,
            Self::U24 => 0x00FF_FFFF,
            Self::U32 => 0xFFFF_FFFF,
            Self::U48 => 0xFFFF_FFFF_FFFF,
            Self::I8 => 0x7F,
            Self::I16 => 0x7FFF,
            Self::I32 => 0x7FFF_FFFF,
        }
    }

    // DiiA 252 Table 3
    pub const fn tmask(self) -> u64 {
        self.mask() - 1
    }

    // DiiA 252 §9.2.8
    pub const fn max_valid(self) -> u64 {
        self.mask() - 2
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32)
    }

    pub const fn to_signed(self, raw: u64) -> i64 {
        let bits = (self.bytes() * 8) as u32;
        let shift = 64 - bits;
        ((raw << shift) as i64) >> shift
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankValue {
    Reading(u64),
    NotImplemented,
    TemporarilyUnavailable,
}

impl BankValue {
    pub const fn value(self) -> Option<u64> {
        match self {
            Self::Reading(v) => Some(v),
            _ => None,
        }
    }

    pub const fn is_temporarily_unavailable(self) -> bool {
        matches!(self, Self::TemporarilyUnavailable)
    }

    pub const fn is_saturated(self, width: ValueWidth) -> bool {
        matches!(self, Self::Reading(v) if v == width.max_valid())
    }
}

pub fn read_value(bytes: &[u8], offset: usize, width: ValueWidth) -> Option<BankValue> {
    let slice = bytes.get(offset..offset.checked_add(width.bytes())?)?;
    let raw = slice
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    Some(classify(raw, width))
}

pub const fn classify(raw: u64, width: ValueWidth) -> BankValue {
    if raw == width.mask() {
        BankValue::NotImplemented
    } else if raw == width.tmask() {
        BankValue::TemporarilyUnavailable
    } else {
        BankValue::Reading(raw)
    }
}

pub fn encode_value(raw: u64, width: ValueWidth, out: &mut [u8]) {
    let n = width.bytes();
    for (i, slot) in out.iter_mut().take(n).enumerate() {
        let shift = 8 * (n - 1 - i);
        *slot = ((raw >> shift) & 0xFF) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_and_tmask_match_the_diia_tables_at_every_width() {
        let rows = [
            (ValueWidth::U8, 0xFFu64, 0xFEu64),
            (ValueWidth::U16, 0xFFFF, 0xFFFE),
            (ValueWidth::U24, 0x00FF_FFFF, 0x00FF_FFFE),
            (ValueWidth::U32, 0xFFFF_FFFF, 0xFFFF_FFFE),
            (ValueWidth::U48, 0xFFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFE),
            (ValueWidth::I8, 0x7F, 0x7E),
            (ValueWidth::I16, 0x7FFF, 0x7FFE),
            (ValueWidth::I32, 0x7FFF_FFFF, 0x7FFF_FFFE),
        ];
        for (width, mask, tmask) in rows {
            assert_eq!(width.mask(), mask, "{width:?} MASK");
            assert_eq!(width.tmask(), tmask, "{width:?} TMASK");
            assert_eq!(width.max_valid(), mask - 2, "{width:?} §9.2.8 ceiling");
        }
    }

    #[test]
    fn a_tmask_energy_counter_is_not_a_number() {
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE];
        assert_eq!(
            read_value(&bytes, 0, ValueWidth::U48),
            Some(BankValue::TemporarilyUnavailable)
        );
        assert_eq!(
            read_value(&bytes, 0, ValueWidth::U48).and_then(BankValue::value),
            None,
            "281 474 976 710 654 Wh is what happens without this"
        );
    }

    #[test]
    fn mask_and_a_real_value_are_told_apart_at_the_same_width() {
        assert_eq!(
            read_value(&[0xFF, 0xFF], 0, ValueWidth::U16),
            Some(BankValue::NotImplemented)
        );
        assert_eq!(
            read_value(&[0xFF, 0x00], 0, ValueWidth::U16),
            Some(BankValue::Reading(0xFF00))
        );
    }

    #[test]
    fn a_short_read_is_neither_sentinel() {
        assert_eq!(read_value(&[0xFF, 0xFF], 0, ValueWidth::U32), None);
        assert_eq!(read_value(&[], 0, ValueWidth::U8), None);
    }

    #[test]
    fn scale_factors_read_as_twos_complement() {
        assert_eq!(ValueWidth::I8.to_signed(0xFD), -3);
        assert_eq!(ValueWidth::I8.to_signed(0x06), 6);
        assert_eq!(ValueWidth::I8.to_signed(0xFA), -6);
        assert_eq!(ValueWidth::I16.to_signed(0xFFFF), -1);
        assert_eq!(ValueWidth::I32.to_signed(0xFFFF_FFFE), -2);
    }

    #[test]
    fn saturation_is_its_own_reading() {
        assert!(BankValue::Reading(0xFD).is_saturated(ValueWidth::U8));
        assert!(!BankValue::Reading(0xFC).is_saturated(ValueWidth::U8));
        assert!(!BankValue::NotImplemented.is_saturated(ValueWidth::U8));
    }

    #[test]
    fn encode_round_trips_through_classify() {
        let mut buf = [0u8; 6];
        encode_value(0x0000_1234_5678, ValueWidth::U48, &mut buf);
        assert_eq!(buf, [0x00, 0x00, 0x12, 0x34, 0x56, 0x78]);
        assert_eq!(
            read_value(&buf, 0, ValueWidth::U48),
            Some(BankValue::Reading(0x0000_1234_5678))
        );
    }
}
