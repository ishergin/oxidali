use dali2rust_contracts::msg::{FixedText, FixedText32, FixedText64};
use dali2rust_domain::dali::banks::{
    part251, read_value, BankValue, ImplementedParts, LuminaireFormat, ValueWidth,
    BUS_UNIT_CONFIGURATION_OFFSET, CONTENT_FORMAT_ID_OFFSET, IMPLEMENTED_PARTS_BASE_OFFSET,
    IMPLEMENTED_PARTS_EXTENSION_OFFSET,
};
use dali2rust_domain::registry::{BankReading, ImplementedPartsView, LuminaireValue};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedMemoryIdentity {
    pub last_memory_bank: Option<u8>,
    pub gtin: Option<u64>,
    pub firmware_version_major: Option<u8>,
    pub firmware_version_minor: Option<u8>,
    pub identification_number: Option<u64>,
    pub hardware_version_major: Option<u8>,
    pub hardware_version_minor: Option<u8>,
    pub dali_101_version: Option<u8>,
    pub dali_102_version: Option<u8>,
    pub dali_103_version: Option<u8>,
    pub logical_control_device_units: Option<u8>,
    pub logical_control_gear_units: Option<u8>,
    pub logical_control_gear_index: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedMemoryProfile {
    pub bank1_lock_byte: Option<u8>,
    pub oem_gtin: Option<u64>,
    pub oem_identification_number: Option<u64>,
}

fn parse_be_u64(bytes: &[u8], start_offset: u16, start: usize, len: usize) -> Option<u64> {
    let rel = start.checked_sub(usize::from(start_offset))?;
    let slice = bytes.get(rel..rel + len)?;
    if slice.iter().all(|byte| *byte == 0xFF) {
        return None;
    }
    Some(
        slice
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)),
    )
}

fn byte(bytes: &[u8], start_offset: u16, index: usize) -> Option<u8> {
    bytes.get(index.checked_sub(usize::from(start_offset))?).copied()
}

fn parse_reading(
    bytes: &[u8],
    start_offset: u16,
    offset: u8,
    width: ValueWidth,
) -> Option<BankReading> {
    let rel = usize::from(offset).checked_sub(usize::from(start_offset))?;
    let raw = read_value(bytes, rel, width)?;
    Some(BankReading {
        value: raw.value(),
        not_implemented: matches!(raw, BankValue::NotImplemented),
        temporarily_unavailable: raw.is_temporarily_unavailable(),
        tmask_since_ms: None,
        saturated: raw.is_saturated(width),
    })
}

fn parse_scale(bytes: &[u8], start_offset: u16, offset: u8) -> Option<i8> {
    let rel = usize::from(offset).checked_sub(usize::from(start_offset))?;
    let raw = read_value(bytes, rel, ValueWidth::I8)?.value()?;
    let signed = ValueWidth::I8.to_signed(raw);
    (-6..=6).contains(&signed).then_some(signed as i8)
}

pub(crate) fn parse_bank0_identity(bytes: &[u8], start_offset: u16) -> ParsedMemoryIdentity {
    ParsedMemoryIdentity {
        last_memory_bank: byte(bytes, start_offset, 2),
        gtin: parse_be_u64(bytes, start_offset, 3, 6),
        firmware_version_major: byte(bytes, start_offset, 9),
        firmware_version_minor: byte(bytes, start_offset, 10),
        identification_number: parse_be_u64(bytes, start_offset, 11, 8),
        hardware_version_major: byte(bytes, start_offset, 19),
        hardware_version_minor: byte(bytes, start_offset, 20),
        dali_101_version: byte(bytes, start_offset, 21),
        dali_102_version: byte(bytes, start_offset, 22),
        dali_103_version: byte(bytes, start_offset, 23),
        logical_control_device_units: byte(bytes, start_offset, 24),
        logical_control_gear_units: byte(bytes, start_offset, 25),
        logical_control_gear_index: byte(bytes, start_offset, 26),
    }
}

pub(crate) fn parse_bank1_profile(bytes: &[u8], start_offset: u16) -> ParsedMemoryProfile {
    ParsedMemoryProfile {
        bank1_lock_byte: byte(bytes, start_offset, 2),
        oem_gtin: parse_be_u64(bytes, start_offset, 3, 6),
        oem_identification_number: parse_be_u64(bytes, start_offset, 9, 8),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedBusUnit {
    pub configuration: Option<u8>,
    pub implemented_parts: Option<ImplementedPartsView>,
}

pub(crate) fn parse_bank0_bus_unit(bytes: &[u8], start_offset: u16) -> ParsedBusUnit {
    let base = byte(bytes, start_offset, usize::from(IMPLEMENTED_PARTS_BASE_OFFSET));
    let extension = byte(bytes, start_offset, usize::from(IMPLEMENTED_PARTS_EXTENSION_OFFSET));
    let implemented_parts = base.map(|base| match extension {
        Some(extension) => ImplementedParts::from_both(base, extension),
        None => ImplementedParts::from_base(base),
    });
    ParsedBusUnit {
        configuration: byte(bytes, start_offset, usize::from(BUS_UNIT_CONFIGURATION_OFFSET)),
        implemented_parts: implemented_parts.map(|parts| ImplementedPartsView {
            raw: parts.raw(),
            bytes: parts.byte_count(),
        }),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedLuminaire {
    pub content_format_id: Option<u16>,
    pub year: Option<LuminaireValue>,
    pub week: Option<LuminaireValue>,
    pub nominal_input_power_w: Option<LuminaireValue>,
    pub power_at_minimum_w: Option<LuminaireValue>,
    pub nominal_min_ac_voltage_v: Option<LuminaireValue>,
    pub nominal_max_ac_voltage_v: Option<LuminaireValue>,
    pub nominal_light_output_lm: Option<LuminaireValue>,
    pub cri: Option<LuminaireValue>,
    pub cct_kelvin: Option<LuminaireValue>,
    pub light_distribution_type: Option<LuminaireValue>,
    pub luminaire_colour: Option<FixedText32>,
    pub luminaire_identification: Option<FixedText64>,
    pub light_distribution: Option<FixedText32>,
    pub oem_name: Option<FixedText32>,
    pub customer_stocking_number: Option<FixedText32>,
    pub lamp_current_ma: Option<LuminaireValue>,
    pub free_use: Option<FixedText32>,
}

fn luminaire_value(raw: u32, value: Option<u32>) -> LuminaireValue {
    LuminaireValue {
        raw,
        value,
        part209_implemented: false,
    }
}

fn u8_field(
    bytes: &[u8],
    start_offset: u16,
    offset: u8,
    classify: impl Fn(u8) -> Option<u8>,
) -> Option<LuminaireValue> {
    let raw = byte(bytes, start_offset, usize::from(offset))?;
    Some(luminaire_value(u32::from(raw), classify(raw).map(u32::from)))
}

fn u16_field(
    bytes: &[u8],
    start_offset: u16,
    offset: u8,
    classify: impl Fn(u16) -> Option<u16>,
) -> Option<LuminaireValue> {
    let rel = usize::from(offset).checked_sub(usize::from(start_offset))?;
    let slice = bytes.get(rel..rel + 2)?;
    let raw = (u16::from(slice[0]) << 8) | u16::from(slice[1]);
    Some(luminaire_value(u32::from(raw), classify(raw).map(u32::from)))
}

pub(crate) fn parse_bank1_luminaire(bytes: &[u8], start_offset: u16) -> ParsedLuminaire {
    let Some(id) = u16_field(bytes, start_offset, CONTENT_FORMAT_ID_OFFSET, Some) else {
        return ParsedLuminaire::default();
    };
    let raw_id = id.raw as u16;
    let Some(format) = LuminaireFormat::from_content_format_id(raw_id) else {
        return ParsedLuminaire {
            content_format_id: Some(raw_id),
            ..Default::default()
        };
    };
    let mut parsed = parse_luminaire_numbers(bytes, start_offset, format);
    parsed.content_format_id = Some(raw_id);
    fill_luminaire_text(&mut parsed, bytes, start_offset, format);
    parsed
}

fn parse_luminaire_electrical(
    bytes: &[u8],
    start_offset: u16,
) -> [Option<LuminaireValue>; 4] {
    [
        u16_field(bytes, start_offset, part251::NOMINAL_INPUT_POWER_OFFSET, part251::power_w),
        u16_field(bytes, start_offset, part251::POWER_AT_MINIMUM_OFFSET, part251::power_w),
        u16_field(
            bytes,
            start_offset,
            part251::NOMINAL_MIN_AC_VOLTAGE_OFFSET,
            part251::mains_voltage_v,
        ),
        u16_field(
            bytes,
            start_offset,
            part251::NOMINAL_MAX_AC_VOLTAGE_OFFSET,
            part251::mains_voltage_v,
        ),
    ]
}

fn parse_light_distribution_type(
    bytes: &[u8],
    start_offset: u16,
    format: LuminaireFormat,
) -> Option<LuminaireValue> {
    u8_field(
        bytes,
        start_offset,
        part251::LIGHT_DISTRIBUTION_TYPE_OFFSET,
        move |raw| match part251::light_distribution_type(raw, format) {
            part251::LightDistributionType::Unknown
            | part251::LightDistributionType::Reserved(_) => None,
            _ => Some(raw),
        },
    )
}

fn parse_luminaire_numbers(
    bytes: &[u8],
    start_offset: u16,
    format: LuminaireFormat,
) -> ParsedLuminaire {
    let [power, power_min, v_min, v_max] = parse_luminaire_electrical(bytes, start_offset);
    ParsedLuminaire {
        year: u8_field(bytes, start_offset, part251::YEAR_OFFSET, part251::year),
        week: u8_field(bytes, start_offset, part251::WEEK_OFFSET, part251::week),
        nominal_input_power_w: power,
        power_at_minimum_w: power_min,
        nominal_min_ac_voltage_v: v_min,
        nominal_max_ac_voltage_v: v_max,
        nominal_light_output_lm: parse_light_output(bytes, start_offset),
        cri: u8_field(bytes, start_offset, part251::CRI_OFFSET, part251::cri),
        cct_kelvin: parse_cct(bytes, start_offset),
        light_distribution_type: parse_light_distribution_type(bytes, start_offset, format),
        lamp_current_ma: format.has_extended_block().then(|| {
            u16_field(
                bytes,
                start_offset,
                part251::LAMP_CURRENT_OFFSET,
                part251::lamp_current_ma,
            )
        })
        .flatten(),
        ..Default::default()
    }
}

fn parse_light_output(bytes: &[u8], start_offset: u16) -> Option<LuminaireValue> {
    let rel = usize::from(part251::NOMINAL_LIGHT_OUTPUT_OFFSET)
        .checked_sub(usize::from(start_offset))?;
    let slice = bytes.get(rel..rel + 3)?;
    let raw = (u32::from(slice[0]) << 16) | (u32::from(slice[1]) << 8) | u32::from(slice[2]);
    Some(luminaire_value(raw, part251::light_output_lm(raw)))
}

fn parse_cct(bytes: &[u8], start_offset: u16) -> Option<LuminaireValue> {
    let rel = usize::from(part251::CCT_OFFSET).checked_sub(usize::from(start_offset))?;
    let slice = bytes.get(rel..rel + 2)?;
    let raw = (u16::from(slice[0]) << 8) | u16::from(slice[1]);
    Some(match part251::cct(raw) {
        part251::CctReading::Kelvin(k) => luminaire_value(u32::from(raw), Some(u32::from(k))),
        part251::CctReading::Part209Implemented => LuminaireValue {
            raw: u32::from(raw),
            value: None,
            part209_implemented: true,
        },
        part251::CctReading::Unknown => luminaire_value(u32::from(raw), None),
    })
}

fn fill_luminaire_text(
    parsed: &mut ParsedLuminaire,
    bytes: &[u8],
    start_offset: u16,
    format: LuminaireFormat,
) {
    parsed.luminaire_colour = text32(bytes, start_offset, part251::LUMINAIRE_COLOUR);
    parsed.luminaire_identification =
        text_region(bytes, start_offset, part251::LUMINAIRE_IDENTIFICATION);
    if format.has_light_distribution() {
        parsed.light_distribution = text32(bytes, start_offset, part251::LIGHT_DISTRIBUTION);
    }
    if format.has_extended_block() {
        parsed.oem_name = text32(bytes, start_offset, part251::OEM_NAME);
        parsed.customer_stocking_number =
            text32(bytes, start_offset, part251::CUSTOMER_STOCKING_NUMBER);
        parsed.free_use = text32(bytes, start_offset, part251::FREE_USE);
    }
}

fn text32(bytes: &[u8], start_offset: u16, region: part251::TextRegion) -> Option<FixedText32> {
    text_region(bytes, start_offset, region)
}

fn text_region<const N: usize>(
    bytes: &[u8],
    start_offset: u16,
    region: part251::TextRegion,
) -> Option<FixedText<N>> {
    let rel = usize::from(region.offset).checked_sub(usize::from(start_offset))?;
    let raw = bytes.get(rel..rel + usize::from(region.len))?;
    let cut = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    Some(escape_into_bounded(&raw[..cut]))
}

fn escape_into_bounded<const N: usize>(raw: &[u8]) -> FixedText<N> {
    let mut out = FixedText::new();
    let mut rest = raw;
    while !rest.is_empty() {
        match core::str::from_utf8(rest) {
            Ok(text) => {
                push_chars(&mut out, text);
                return out;
            }
            Err(err) => {
                let (good, bad) = rest.split_at(err.valid_up_to());
                if let Ok(text) = core::str::from_utf8(good) {
                    push_chars(&mut out, text);
                }
                let skip = err.error_len().unwrap_or(bad.len()).max(1);
                for byte in bad.iter().take(skip) {
                    let mut buf = [0u8; 4];
                    if out.push_str(format_hex_escape(*byte, &mut buf)).is_err() {
                        return out;
                    }
                }
                rest = &bad[skip.min(bad.len())..];
            }
        }
    }
    out
}

fn push_chars<const N: usize>(out: &mut FixedText<N>, text: &str) {
    for ch in text.chars() {
        if out.push(ch).is_err() {
            return;
        }
    }
}

fn format_hex_escape(byte: u8, buf: &mut [u8; 4]) -> &str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    buf[0] = b'\\';
    buf[1] = b'x';
    buf[2] = HEX[usize::from(byte >> 4)];
    buf[3] = HEX[usize::from(byte & 0x0F)];
    core::str::from_utf8(buf).unwrap_or("")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedEnergyBank {
    pub bank_version: Option<u8>,
    pub energy_scale: Option<i8>,
    pub energy: Option<BankReading>,
    pub power_scale: Option<i8>,
    pub power: Option<BankReading>,
}

pub(crate) fn parse_energy_bank(bytes: &[u8], start_offset: u16) -> ParsedEnergyBank {
    ParsedEnergyBank {
        bank_version: byte(bytes, start_offset, 0x03),
        energy_scale: parse_scale(bytes, start_offset, 0x04),
        energy: parse_reading(bytes, start_offset, 0x05, ValueWidth::U48),
        power_scale: parse_scale(bytes, start_offset, 0x0B),
        power: parse_reading(bytes, start_offset, 0x0C, ValueWidth::U32),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedCondition {
    pub flag: Option<BankReading>,
    pub counter: Option<BankReading>,
}

fn parse_condition(bytes: &[u8], start_offset: u16, flag_offset: u8) -> ParsedCondition {
    ParsedCondition {
        flag: parse_reading(bytes, start_offset, flag_offset, ValueWidth::U8),
        counter: parse_reading(bytes, start_offset, flag_offset + 1, ValueWidth::U8),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedGearDiagnostics {
    pub bank_version: Option<u8>,
    pub operating_time_s: Option<BankReading>,
    pub start_counter: Option<BankReading>,
    pub supply_voltage_decivolt: Option<BankReading>,
    pub supply_frequency_hz: Option<BankReading>,
    pub power_factor_centi: Option<BankReading>,
    pub overall_failure: ParsedCondition,
    pub undervoltage: ParsedCondition,
    pub overvoltage: ParsedCondition,
    pub output_power_limitation: ParsedCondition,
    pub thermal_derating: ParsedCondition,
    pub thermal_shutdown: ParsedCondition,
    pub temperature_offset60: Option<BankReading>,
    pub output_current_percent: Option<BankReading>,
}

pub(crate) fn parse_gear_diagnostics(bytes: &[u8], start_offset: u16) -> ParsedGearDiagnostics {
    ParsedGearDiagnostics {
        bank_version: byte(bytes, start_offset, 0x03),
        operating_time_s: parse_reading(bytes, start_offset, 0x04, ValueWidth::U32),
        start_counter: parse_reading(bytes, start_offset, 0x08, ValueWidth::U24),
        supply_voltage_decivolt: parse_reading(bytes, start_offset, 0x0B, ValueWidth::U16),
        supply_frequency_hz: parse_reading(bytes, start_offset, 0x0D, ValueWidth::U8),
        power_factor_centi: parse_reading(bytes, start_offset, 0x0E, ValueWidth::U8),
        overall_failure: parse_condition(bytes, start_offset, 0x0F),
        undervoltage: parse_condition(bytes, start_offset, 0x11),
        overvoltage: parse_condition(bytes, start_offset, 0x13),
        output_power_limitation: parse_condition(bytes, start_offset, 0x15),
        thermal_derating: parse_condition(bytes, start_offset, 0x17),
        thermal_shutdown: parse_condition(bytes, start_offset, 0x19),
        temperature_offset60: parse_reading(bytes, start_offset, 0x1B, ValueWidth::U8),
        output_current_percent: parse_reading(bytes, start_offset, 0x1C, ValueWidth::U8),
    }
}

// DiiA 253 §9.2.17
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedSourceDiagnostics {
    pub bank_version: Option<u8>,
    pub start_counter_resettable: Option<BankReading>,
    pub start_counter: Option<BankReading>,
    pub on_time_resettable_s: Option<BankReading>,
    pub on_time_s: Option<BankReading>,
    pub voltage_decivolt: Option<BankReading>,
    pub current_milliamp: Option<BankReading>,
    pub overall_failure: ParsedCondition,
    pub short_circuit: ParsedCondition,
    pub open_circuit: ParsedCondition,
    pub thermal_derating: ParsedCondition,
    pub thermal_shutdown: ParsedCondition,
    pub temperature_offset60: Option<BankReading>,
}

pub(crate) fn parse_source_diagnostics(bytes: &[u8], start_offset: u16) -> ParsedSourceDiagnostics {
    ParsedSourceDiagnostics {
        bank_version: byte(bytes, start_offset, 0x03),
        start_counter_resettable: parse_reading(bytes, start_offset, 0x04, ValueWidth::U24),
        start_counter: parse_reading(bytes, start_offset, 0x07, ValueWidth::U24),
        on_time_resettable_s: parse_reading(bytes, start_offset, 0x0A, ValueWidth::U32),
        on_time_s: parse_reading(bytes, start_offset, 0x0E, ValueWidth::U32),
        voltage_decivolt: parse_reading(bytes, start_offset, 0x12, ValueWidth::U16),
        current_milliamp: parse_reading(bytes, start_offset, 0x14, ValueWidth::U16),
        overall_failure: parse_condition(bytes, start_offset, 0x16),
        short_circuit: parse_condition(bytes, start_offset, 0x18),
        open_circuit: parse_condition(bytes, start_offset, 0x1A),
        thermal_derating: parse_condition(bytes, start_offset, 0x1C),
        thermal_shutdown: parse_condition(bytes, start_offset, 0x1E),
        temperature_offset60: parse_reading(bytes, start_offset, 0x20, ValueWidth::U8),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParsedLuminaireMaintenance {
    pub bank_version: Option<u8>,
    pub rated_life_kilohours: Option<BankReading>,
    pub reference_temperature_offset60: Option<BankReading>,
    pub rated_starts_hundreds: Option<BankReading>,
}

pub(crate) fn parse_luminaire_maintenance(bytes: &[u8], start_offset: u16) -> ParsedLuminaireMaintenance {
    ParsedLuminaireMaintenance {
        bank_version: byte(bytes, start_offset, 0x03),
        rated_life_kilohours: parse_reading(bytes, start_offset, 0x04, ValueWidth::U8),
        reference_temperature_offset60: parse_reading(bytes, start_offset, 0x05, ValueWidth::U8),
        rated_starts_hundreds: parse_reading(bytes, start_offset, 0x06, ValueWidth::U16),
    }
}

#[cfg(test)]
mod tests {
    use super::{escape_into_bounded, parse_bank0_identity, parse_bank1_profile};
    use dali2rust_contracts::msg::FixedText;

    #[test]
    fn only_the_invalid_bytes_of_a_text_region_are_escaped() {
        let out: FixedText<32> = escape_into_bounded(b"warm\xFFwhite");
        assert_eq!(out.as_str(), "warm\\xFFwhite");
    }

    #[test]
    fn valid_utf8_passes_through_untouched() {
        let out: FixedText<32> = escape_into_bounded("wärm wîte".as_bytes());
        assert_eq!(out.as_str(), "wärm wîte");
    }

    #[test]
    fn a_truncated_utf8_sequence_terminates() {
        let out: FixedText<32> = escape_into_bounded(&[b'a', 0xE2, 0x82]);
        assert_eq!(out.as_str(), "a\\xE2\\x82");
    }

    #[test]
    fn an_overflowing_escape_truncates_rather_than_panicking() {
        let out: FixedText<8> = escape_into_bounded(&[0xFF; 10]);
        assert!(out.len() <= 8);
    }

    #[test]
    fn parse_bank0_identity_matches_golden_addr0_bytes() {
        let bank0 = [
            0x1C, 0x00, 0x01, 0x00, 0x9D, 0xAD, 0xA2, 0x1B, 0x43, 0x01, 0x00, 0x26, 0x01, 0x06,
            0xFA, 0x9E, 0x78, 0x7D, 0x9B, 0x01, 0x00, 0x08, 0x08, 0xFF, 0x00, 0x01, 0x00,
        ];
        let parsed = parse_bank0_identity(&bank0, 0);
        assert_eq!(parsed.last_memory_bank, Some(0x01));
        assert_eq!(parsed.gtin, Some(0x009D_ADA2_1B43));
        assert_eq!(parsed.firmware_version_major, Some(0x01));
        assert_eq!(parsed.firmware_version_minor, Some(0x00));
        assert_eq!(parsed.identification_number, Some(0x2601_06FA_9E78_7D9B));
        assert_eq!(parsed.hardware_version_major, Some(0x01));
        assert_eq!(parsed.hardware_version_minor, Some(0x00));
        assert_eq!(parsed.dali_101_version, Some(0x08));
        assert_eq!(parsed.dali_102_version, Some(0x08));
        assert_eq!(parsed.dali_103_version, Some(0xFF));
        assert_eq!(parsed.logical_control_device_units, Some(0x00));
        assert_eq!(parsed.logical_control_gear_units, Some(0x01));
        assert_eq!(parsed.logical_control_gear_index, Some(0x00));
    }

    #[test]
    fn parse_bank1_profile_matches_golden_addr1_bytes() {
        let bank1 = [
            0x10, 0x00, 0x06, 0x58, 0x23, 0x32, 0xA8, 0xFC, 0xFF, 0xF9, 0x00, 0x00, 0x00, 0xE6,
            0xFF, 0x9B,
        ];
        let parsed = parse_bank1_profile(&bank1, 0);
        assert_eq!(parsed.bank1_lock_byte, Some(0x06));
        assert_eq!(parsed.oem_gtin, Some(0x5823_32A8_FCFF));
        assert_eq!(parsed.oem_identification_number, None);
    }

    #[test]
    fn parse_bank1_profile_keeps_oem_identity_when_full_range_present() {
        let bank1 = [
            0x10, 0x00, 0x06, 0x58, 0x23, 0x32, 0xA8, 0xFC, 0xFF, 0xF9, 0x00, 0x00, 0x00, 0xE6,
            0xFF, 0x9B, 0x01,
        ];
        let parsed = parse_bank1_profile(&bank1, 0);
        assert_eq!(
            parsed.oem_identification_number,
            Some(0xF900_0000_E6FF_9B01)
        );
    }
}
