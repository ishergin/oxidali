use dali2rust_contracts::msg::LuminaireNumbers as N;
use dali2rust_domain::dali::banks::luminaire_write::{
    number_patch, planned_writes, text_patch, FieldKind, FieldWrite, LUMINAIRE_FIELDS,
};
use dali2rust_domain::dali::banks::{LuminaireFormat, CONTENT_FORMAT_ID_OFFSET};
use dali2rust_domain::registry::LuminaireValue;

use super::{parse_bank1_luminaire, ParsedLuminaire};

const V5: LuminaireFormat = LuminaireFormat::V5;

const SAMPLES: [u64; 16] = [
    0, 1, 3, 5, 7, 24, 53, 90, 100, 230, 480, 2_700, 6_500, 17_000, 0xFFFE, 0x00FF_FFFE,
];

const SHORT_TEXT: &str = "warm white";

const PRINTABLE_60: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456 ";

type NumberLeaf = fn(&ParsedLuminaire) -> Option<LuminaireValue>;
type TextLeaf = fn(&ParsedLuminaire) -> Option<String>;

const NUMBER_LEAVES: [(&str, NumberLeaf); 11] = [
    ("year", |p| p.year),
    ("week", |p| p.week),
    ("nominal_input_power_w", |p| p.nominal_input_power_w),
    ("power_at_minimum_w", |p| p.power_at_minimum_w),
    ("nominal_min_ac_voltage_v", |p| p.nominal_min_ac_voltage_v),
    ("nominal_max_ac_voltage_v", |p| p.nominal_max_ac_voltage_v),
    ("nominal_light_output_lm", |p| p.nominal_light_output_lm),
    ("cri", |p| p.cri),
    ("cct_kelvin", |p| p.cct_kelvin),
    ("light_distribution_type", |p| p.light_distribution_type),
    ("lamp_current_ma", |p| p.lamp_current_ma),
];

const TEXT_LEAVES: [(&str, TextLeaf); 6] = [
    ("luminaire_colour", |p| p.luminaire_colour.as_ref().map(|t| t.as_str().to_owned())),
    ("luminaire_identification", |p| p.luminaire_identification.as_ref().map(|t| t.as_str().to_owned())),
    ("light_distribution", |p| p.light_distribution.as_ref().map(|t| t.as_str().to_owned())),
    ("oem_name", |p| p.oem_name.as_ref().map(|t| t.as_str().to_owned())),
    ("customer_stocking_number", |p| p.customer_stocking_number.as_ref().map(|t| t.as_str().to_owned())),
    ("free_use", |p| p.free_use.as_ref().map(|t| t.as_str().to_owned())),
];

fn leaf<T: Copy>(table: &[(&str, T)], name: &str) -> T {
    table.iter().find(|(leaf, _)| *leaf == name).map(|(_, read)| *read).expect(name)
}

fn whole_bank_declaring(format: LuminaireFormat) -> Vec<u8> {
    let mut bank = vec![0u8; usize::from(V5.min_last_offset()) + 1];
    bank[0] = V5.min_last_offset();
    let at = usize::from(CONTENT_FORMAT_ID_OFFSET);
    bank[at..at + 2].copy_from_slice(&format.content_format_id().to_be_bytes());
    bank
}

fn bank_after(writes: &[FieldWrite]) -> ParsedLuminaire {
    let mut bank = whole_bank_declaring(V5);
    for write in writes {
        bank[usize::from(write.field.offset)..write.end()].copy_from_slice(&write.bytes);
    }
    parse_bank1_luminaire(&bank, 0)
}

// DiiA 251 §9.2.5
#[test]
fn the_parser_fills_exactly_the_fields_the_write_table_puts_in_each_format() {
    for format in [LuminaireFormat::V3, LuminaireFormat::V4, V5] {
        let parsed = parse_bank1_luminaire(&whole_bank_declaring(format), 0);
        for field in &LUMINAIRE_FIELDS {
            let filled = match field.kind {
                FieldKind::LayoutId => continue,
                FieldKind::Number { .. } => leaf(&NUMBER_LEAVES, field.name)(&parsed).is_some(),
                FieldKind::Text(_) => leaf(&TEXT_LEAVES, field.name)(&parsed).is_some(),
            };
            let id = format.content_format_id();
            assert_eq!(filled, field.in_format(format), "{} under content format {id}", field.name);
        }
    }
}

// DiiA 251 §9.2.3
#[test]
fn every_number_the_patch_accepts_reads_back_as_the_same_value() {
    for field in &LUMINAIRE_FIELDS {
        let FieldKind::Number { patch, .. } = field.kind else { continue };
        let read = leaf(&NUMBER_LEAVES, field.name);
        let accepted: Vec<u32> =
            SAMPLES.iter().filter(|v| field.accepts_number(**v, V5)).map(|v| *v as u32).collect();
        assert!(accepted.len() > 1, "{} is tried with too few values", field.name);
        for value in accepted {
            let shown = read(&bank_after(&planned_writes(&number_patch(&[(patch, value)]), None)));
            assert_eq!(shown.and_then(|leaf| leaf.value), Some(value), "{} = {value}", field.name);
        }
    }
}

// DiiA 251 §9.2.5
#[test]
fn every_text_the_patch_accepts_reads_back_as_written() {
    for field in &LUMINAIRE_FIELDS {
        let FieldKind::Text(_) = field.kind else { continue };
        let read = leaf(&TEXT_LEAVES, field.name);
        let len = usize::from(field.len);
        let short = &SHORT_TEXT[..len.min(SHORT_TEXT.len())];
        for text in ["x", short, &PRINTABLE_60[..len]] {
            assert!(field.accepts_text(text), "{} refuses {text:?}", field.name);
            let patch = text_patch(field, text).expect("a text field");
            let shown = read(&bank_after(&planned_writes(&N::default(), Some(&patch))));
            assert_eq!(shown.as_deref(), Some(text), "{}", field.name);
        }
    }
}
