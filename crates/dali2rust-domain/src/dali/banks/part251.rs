use super::{BankField, ValueWidth};

pub const BANK_LUMINAIRE_INFO: u8 = 1;

pub const CONTENT_FORMAT_ID_OFFSET: u8 = 0x11;
pub const BANK1_BASE_LAST_OFFSET: u8 = 0x10;
pub const BANK1_FORMAT_GATE_LAST_OFFSET: u8 = 0x12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LuminaireFormat {
    V3,
    V4,
    V5,
}

impl LuminaireFormat {
    pub const fn from_content_format_id(id: u16) -> Option<Self> {
        match id {
            3 => Some(Self::V3),
            4 => Some(Self::V4),
            5 => Some(Self::V5),
            _ => None,
        }
    }

    pub const fn content_format_id(self) -> u16 {
        match self {
            Self::V3 => 3,
            Self::V4 => 4,
            Self::V5 => 5,
        }
    }

    pub const fn min_last_offset(self) -> u8 {
        match self {
            Self::V3 => 0x77,
            Self::V4 => 0x8F,
            Self::V5 => 0xC9,
        }
    }
}

pub const YEAR_OFFSET: u8 = 0x13;
pub const WEEK_OFFSET: u8 = 0x14;
pub const NOMINAL_INPUT_POWER_OFFSET: u8 = 0x15;
pub const POWER_AT_MINIMUM_OFFSET: u8 = 0x17;
pub const NOMINAL_MIN_AC_VOLTAGE_OFFSET: u8 = 0x19;
pub const NOMINAL_MAX_AC_VOLTAGE_OFFSET: u8 = 0x1B;
pub const NOMINAL_LIGHT_OUTPUT_OFFSET: u8 = 0x1D;
pub const CRI_OFFSET: u8 = 0x20;
pub const CCT_OFFSET: u8 = 0x21;
pub const LIGHT_DISTRIBUTION_TYPE_OFFSET: u8 = 0x23;

use ValueWidth::{U16, U24, U8};

const fn f(offset: u8, width: ValueWidth) -> BankField {
    BankField { offset, width }
}

pub const BANK_LUMINAIRE_INFO_MAP: [BankField; 28] = [
    f(0x00, U8),
    f(0x01, U8),
    f(0x02, U8),
    f(0x03, U8),
    f(0x04, U8),
    f(0x05, U8),
    f(0x06, U8),
    f(0x07, U8),
    f(0x08, U8),
    f(0x09, U8),
    f(0x0A, U8),
    f(0x0B, U8),
    f(0x0C, U8),
    f(0x0D, U8),
    f(0x0E, U8),
    f(0x0F, U8),
    f(0x10, U8),
    f(CONTENT_FORMAT_ID_OFFSET, U16),
    f(YEAR_OFFSET, U8),
    f(WEEK_OFFSET, U8),
    f(NOMINAL_INPUT_POWER_OFFSET, U16),
    f(POWER_AT_MINIMUM_OFFSET, U16),
    f(NOMINAL_MIN_AC_VOLTAGE_OFFSET, U16),
    f(NOMINAL_MAX_AC_VOLTAGE_OFFSET, U16),
    f(NOMINAL_LIGHT_OUTPUT_OFFSET, U24),
    f(CRI_OFFSET, U8),
    f(CCT_OFFSET, U16),
    f(LIGHT_DISTRIBUTION_TYPE_OFFSET, U8),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRegion {
    pub offset: u8,
    pub len: u8,
}

const fn t(offset: u8, len: u8) -> TextRegion {
    TextRegion { offset, len }
}

pub const LUMINAIRE_COLOUR: TextRegion = t(0x24, 24);
pub const LUMINAIRE_IDENTIFICATION: TextRegion = t(0x3C, 60);
pub const LIGHT_DISTRIBUTION: TextRegion = t(0x78, 24);
pub const OEM_NAME: TextRegion = t(0x90, 32);
pub const CUSTOMER_STOCKING_NUMBER: TextRegion = t(0xB0, 16);
pub const FREE_USE: TextRegion = t(0xC2, 8);
pub const LAMP_CURRENT_OFFSET: u8 = 0xC0;

impl LuminaireFormat {
    pub const fn has_light_distribution(self) -> bool {
        !matches!(self, Self::V3)
    }

    pub const fn has_extended_block(self) -> bool {
        matches!(self, Self::V5)
    }
}

pub fn text_bytes(bank: &[u8], region: TextRegion) -> Option<&[u8]> {
    let start = region.offset as usize;
    let end = start.checked_add(region.len as usize)?;
    let raw = bank.get(start..end)?;
    let cut = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    Some(&raw[..cut])
}

pub const fn year(raw: u8) -> Option<u8> {
    if raw <= 99 {
        Some(raw)
    } else {
        None
    }
}

pub const fn week(raw: u8) -> Option<u8> {
    if raw >= 1 && raw <= 53 {
        Some(raw)
    } else {
        None
    }
}

pub const fn power_w(raw: u16) -> Option<u16> {
    if raw == u16::MAX {
        None
    } else {
        Some(raw)
    }
}

pub const fn mains_voltage_v(raw: u16) -> Option<u16> {
    if raw >= 90 && raw <= 480 {
        Some(raw)
    } else {
        None
    }
}

pub const fn light_output_lm(raw: u32) -> Option<u32> {
    const MASK: u32 = 0x00FF_FFFF;
    if raw == MASK {
        None
    } else {
        Some(raw)
    }
}

pub const fn cri(raw: u8) -> Option<u8> {
    if raw <= 100 {
        Some(raw)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CctReading {
    Kelvin(u16),
    Part209Implemented,
    Unknown,
}

pub const fn cct(raw: u16) -> CctReading {
    if raw <= 17_000 {
        CctReading::Kelvin(raw)
    } else if raw == u16::MAX - 1 {
        CctReading::Part209Implemented
    } else {
        CctReading::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightDistributionType {
    NotSpecified,
    Type(u8),
    EmergencyLuminaire,
    Other,
    Reserved(u8),
    Unknown,
}

pub const fn light_distribution_type(raw: u8, format: LuminaireFormat) -> LightDistributionType {
    match raw {
        0 => LightDistributionType::NotSpecified,
        1..=5 => LightDistributionType::Type(raw),
        0xFF => LightDistributionType::Unknown,
        _ => light_distribution_type_extended(raw, format),
    }
}

const fn light_distribution_type_extended(
    raw: u8,
    format: LuminaireFormat,
) -> LightDistributionType {
    if matches!(format, LuminaireFormat::V3) {
        return LightDistributionType::Reserved(raw);
    }
    match raw {
        6 => LightDistributionType::Type(6),
        253 => LightDistributionType::EmergencyLuminaire,
        254 => LightDistributionType::Other,
        _ => LightDistributionType::Reserved(raw),
    }
}

pub const fn lamp_current_ma(raw: u16) -> Option<u16> {
    if raw == u16::MAX {
        None
    } else {
        Some(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_three_content_format_ids_open_the_layout() {
        assert_eq!(LuminaireFormat::from_content_format_id(3), Some(LuminaireFormat::V3));
        assert_eq!(LuminaireFormat::from_content_format_id(4), Some(LuminaireFormat::V4));
        assert_eq!(LuminaireFormat::from_content_format_id(5), Some(LuminaireFormat::V5));
        for id in [0u16, 1, 2, 6, 0x0300, 0xFFFF] {
            assert_eq!(LuminaireFormat::from_content_format_id(id), None, "id {id}");
        }
    }

    #[test]
    fn mask_minus_one_is_a_value_here_not_temporarily_unavailable() {
        assert_eq!(power_w(0xFFFE), Some(65_534), "nominal input power");
        assert_eq!(power_w(0xFFFF), None, "MASK alone is unknown");
        assert_eq!(light_output_lm(0x00FF_FFFE), Some(16_777_214));
        assert_eq!(light_output_lm(0x00FF_FFFF), None);
        assert_eq!(cct(0xFFFE), CctReading::Part209Implemented);
        assert_eq!(cct(0xFFFF), CctReading::Unknown);
    }

    #[test]
    fn cct_splits_at_seventeen_thousand() {
        assert_eq!(cct(0), CctReading::Kelvin(0));
        assert_eq!(cct(2_700), CctReading::Kelvin(2_700));
        assert_eq!(cct(17_000), CctReading::Kelvin(17_000));
        assert_eq!(cct(17_001), CctReading::Unknown);
        assert_eq!(cct(0xFFFD), CctReading::Unknown);
    }

    #[test]
    fn year_week_cri_and_voltage_follow_their_own_ranges() {
        assert_eq!(year(0), Some(0));
        assert_eq!(year(99), Some(99));
        assert_eq!(year(100), None);
        assert_eq!(week(0), None, "0 is explicitly unknown, not week zero");
        assert_eq!(week(1), Some(1));
        assert_eq!(week(53), Some(53));
        assert_eq!(week(54), None);
        assert_eq!(cri(100), Some(100));
        assert_eq!(cri(101), None);
        assert_eq!(mains_voltage_v(89), None);
        assert_eq!(mains_voltage_v(90), Some(90));
        assert_eq!(mains_voltage_v(230), Some(230));
        assert_eq!(mains_voltage_v(480), Some(480));
        assert_eq!(mains_voltage_v(481), None);
    }

    #[test]
    fn light_distribution_type_is_classified_per_format() {
        use LightDistributionType as L;
        assert_eq!(light_distribution_type(0, LuminaireFormat::V3), L::NotSpecified);
        assert_eq!(light_distribution_type(3, LuminaireFormat::V3), L::Type(3));
        assert_eq!(light_distribution_type(6, LuminaireFormat::V3), L::Reserved(6));
        assert_eq!(light_distribution_type(6, LuminaireFormat::V4), L::Type(6));
        assert_eq!(
            light_distribution_type(253, LuminaireFormat::V5),
            L::EmergencyLuminaire
        );
        assert_eq!(light_distribution_type(253, LuminaireFormat::V3), L::Reserved(253));
        assert_eq!(light_distribution_type(254, LuminaireFormat::V4), L::Other);
        assert_eq!(light_distribution_type(0xFF, LuminaireFormat::V3), L::Unknown);
        assert_eq!(light_distribution_type(7, LuminaireFormat::V5), L::Reserved(7));
    }

    #[test]
    fn a_text_region_is_cut_at_the_first_nul() {
        let mut bank = [0u8; 0x40];
        bank[0x24..0x24 + 5].copy_from_slice(b"warm\0");
        assert_eq!(text_bytes(&bank, LUMINAIRE_COLOUR), Some(&b"warm"[..]));
    }

    #[test]
    fn an_unread_region_and_an_empty_one_are_different() {
        let short = [0u8; 0x30];
        assert_eq!(
            text_bytes(&short, LUMINAIRE_IDENTIFICATION),
            None,
            "0x3C+60 is past this image"
        );
        let full = [0u8; 0x78];
        assert_eq!(
            text_bytes(&full, LUMINAIRE_IDENTIFICATION),
            Some(&b""[..]),
            "all-0x00 is the factory default, and it was read"
        );
    }

    #[test]
    fn text_regions_return_raw_bytes_not_a_decoded_string() {
        let mut bank = [0u8; 0x40];
        bank[0x24] = 0xC3;
        bank[0x25] = 0xA9;
        bank[0x26] = 0x00;
        assert_eq!(text_bytes(&bank, LUMINAIRE_COLOUR), Some(&[0xC3u8, 0xA9][..]));
    }

    #[test]
    fn the_map_is_contiguous_and_covers_the_numeric_block() {
        let mut next = 0u16;
        for field in BANK_LUMINAIRE_INFO_MAP {
            assert_eq!(u16::from(field.offset), next, "gap before {:#04x}", field.offset);
            next += field.width.bytes() as u16;
        }
        assert_eq!(next, 0x24, "map ends where the string regions begin");
    }

    #[test]
    fn format_supersets_are_ordered() {
        assert!(!LuminaireFormat::V3.has_light_distribution());
        assert!(LuminaireFormat::V4.has_light_distribution());
        assert!(LuminaireFormat::V5.has_light_distribution());
        assert!(!LuminaireFormat::V4.has_extended_block());
        assert!(LuminaireFormat::V5.has_extended_block());
        assert_eq!(LuminaireFormat::V3.min_last_offset(), 0x77);
        assert_eq!(LuminaireFormat::V4.min_last_offset(), 0x8F);
        assert_eq!(LuminaireFormat::V5.min_last_offset(), 0xC9);
    }
}
