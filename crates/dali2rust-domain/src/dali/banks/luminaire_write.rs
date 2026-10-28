use dali2rust_contracts::msg::{
    LuminaireNumbers as N, LuminaireText, LuminaireTextField as T, LuminaireTextWrite,
};

use super::part251::{self, CctReading, LightDistributionType, LuminaireFormat, TextRegion};
use super::BANK1_FORMAT_GATE_LAST_OFFSET;
use NumberRule as R;

// IEC 62386-102 §9.10.2, Table 8
pub const BANK_UNLOCKED: u8 = 0x55;

pub const BANK1_HEADER_LEN: u16 = BANK1_FORMAT_GATE_LAST_OFFSET as u16 + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberRule {
    Year,
    Week,
    Power,
    MainsVoltage,
    LightOutput,
    Cri,
    Cct,
    LightDistributionType,
    LampCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    LayoutId,
    Number { patch: u16, rule: NumberRule },
    Text(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuminaireField {
    pub name: &'static str,
    pub offset: u8,
    pub len: u8,
    pub since: LuminaireFormat,
    pub kind: FieldKind,
}

const fn number(name: &'static str, offset: u8, len: u8, patch: u16, rule: NumberRule) -> LuminaireField {
    LuminaireField { name, offset, len, since: LuminaireFormat::V3, kind: FieldKind::Number { patch, rule } }
}

const fn text(name: &'static str, region: TextRegion, since: LuminaireFormat, field: T) -> LuminaireField {
    LuminaireField { name, offset: region.offset, len: region.len, since, kind: FieldKind::Text(field) }
}

// DiiA 251 §9.2.5; IEC 62386-251 Table 1
pub const LUMINAIRE_FIELDS: [LuminaireField; 18] = [
    LuminaireField {
        name: "content_format_id",
        offset: part251::CONTENT_FORMAT_ID_OFFSET,
        len: 2,
        since: LuminaireFormat::V3,
        kind: FieldKind::LayoutId,
    },
    number("year", part251::YEAR_OFFSET, 1, N::PATCH_YEAR, R::Year),
    number("week", part251::WEEK_OFFSET, 1, N::PATCH_WEEK, R::Week),
    number("nominal_input_power_w", part251::NOMINAL_INPUT_POWER_OFFSET, 2, N::PATCH_NOMINAL_INPUT_POWER_W, R::Power),
    number("power_at_minimum_w", part251::POWER_AT_MINIMUM_OFFSET, 2, N::PATCH_POWER_AT_MINIMUM_W, R::Power),
    number("nominal_min_ac_voltage_v", part251::NOMINAL_MIN_AC_VOLTAGE_OFFSET, 2, N::PATCH_NOMINAL_MIN_AC_VOLTAGE_V, R::MainsVoltage),
    number("nominal_max_ac_voltage_v", part251::NOMINAL_MAX_AC_VOLTAGE_OFFSET, 2, N::PATCH_NOMINAL_MAX_AC_VOLTAGE_V, R::MainsVoltage),
    number("nominal_light_output_lm", part251::NOMINAL_LIGHT_OUTPUT_OFFSET, 3, N::PATCH_NOMINAL_LIGHT_OUTPUT_LM, R::LightOutput),
    number("cri", part251::CRI_OFFSET, 1, N::PATCH_CRI, R::Cri),
    number("cct_kelvin", part251::CCT_OFFSET, 2, N::PATCH_CCT_KELVIN, R::Cct),
    number("light_distribution_type", part251::LIGHT_DISTRIBUTION_TYPE_OFFSET, 1, N::PATCH_LIGHT_DISTRIBUTION_TYPE, R::LightDistributionType),
    text("luminaire_colour", part251::LUMINAIRE_COLOUR, LuminaireFormat::V3, T::LuminaireColour),
    text("luminaire_identification", part251::LUMINAIRE_IDENTIFICATION, LuminaireFormat::V3, T::LuminaireIdentification),
    text("light_distribution", part251::LIGHT_DISTRIBUTION, LuminaireFormat::V4, T::LightDistribution),
    text("oem_name", part251::OEM_NAME, LuminaireFormat::V5, T::OemName),
    text("customer_stocking_number", part251::CUSTOMER_STOCKING_NUMBER, LuminaireFormat::V5, T::CustomerStockingNumber),
    LuminaireField {
        since: LuminaireFormat::V5,
        ..number("lamp_current_ma", part251::LAMP_CURRENT_OFFSET, 2, N::PATCH_LAMP_CURRENT_MA, R::LampCurrent)
    },
    text("free_use", part251::FREE_USE, LuminaireFormat::V5, T::FreeUse),
];

impl LuminaireField {
    pub fn named(name: &str) -> Option<&'static Self> {
        LUMINAIRE_FIELDS.iter().find(|field| field.name == name)
    }

    pub fn of_text(which: T) -> Option<&'static Self> {
        LUMINAIRE_FIELDS.iter().find(|field| field.kind == FieldKind::Text(which))
    }

    pub const fn last_offset(&self) -> u8 {
        self.offset + (self.len - 1)
    }

    pub fn accepts_number(&self, value: u64, format: LuminaireFormat) -> bool {
        let FieldKind::Number { rule, .. } = self.kind else {
            return false;
        };
        let fits = value < 1u64 << (8 * u32::from(self.len));
        fits && rule.shows_back(value as u32, format)
    }

    // DiiA 251 §9.2.5 footnote d
    pub fn accepts_text(&self, text: &str) -> bool {
        let printable = |byte: &u8| (b' '..=b'~').contains(byte);
        matches!(self.kind, FieldKind::Text(_))
            && text.len() <= usize::from(self.len)
            && text.bytes().all(|byte| printable(&byte))
    }
}

impl NumberRule {
    const fn shows_back(self, raw: u32, format: LuminaireFormat) -> bool {
        match self {
            Self::Year => part251::year(raw as u8).is_some(),
            Self::Week => part251::week(raw as u8).is_some(),
            Self::Power => part251::power_w(raw as u16).is_some(),
            Self::MainsVoltage => part251::mains_voltage_v(raw as u16).is_some(),
            Self::LightOutput => part251::light_output_lm(raw).is_some(),
            Self::Cri => part251::cri(raw as u8).is_some(),
            Self::Cct => matches!(part251::cct(raw as u16), CctReading::Kelvin(_)),
            Self::LightDistributionType => !matches!(
                part251::light_distribution_type(raw as u8, format),
                LightDistributionType::Unknown | LightDistributionType::Reserved(_)
            ),
            Self::LampCurrent => part251::lamp_current_ma(raw as u16).is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bank1Layout {
    pub implemented: Option<bool>,
    pub content_format_id: Option<u16>,
    pub last_offset: Option<u8>,
}

impl Bank1Layout {
    // IEC 62386-102 §9.10.4
    pub fn from_header(header: &[u8]) -> Self {
        let format = usize::from(part251::CONTENT_FORMAT_ID_OFFSET);
        Self {
            implemented: Some(!header.is_empty()),
            content_format_id: header
                .get(format..format + 2)
                .map(|id| u16::from_be_bytes([id[0], id[1]])),
            last_offset: header.first().copied(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bank1WriteRefusal {
    FieldReadOnly,
    NotImplemented,
    NotPart251,
    FieldNotInFormat,
    TooShort,
}

impl Bank1WriteRefusal {
    pub const fn cause(self) -> &'static str {
        match self {
            Self::FieldReadOnly => "bank1_field_read_only",
            Self::NotImplemented => "bank1_not_implemented",
            Self::NotPart251 => "bank1_not_part251",
            Self::FieldNotInFormat => "bank1_field_not_in_format",
            Self::TooShort => "bank1_too_short",
        }
    }
}

pub fn check_writable(
    layout: &Bank1Layout,
    field: &LuminaireField,
) -> Result<LuminaireFormat, Bank1WriteRefusal> {
    if field.kind == FieldKind::LayoutId {
        return Err(Bank1WriteRefusal::FieldReadOnly);
    }
    if layout.implemented == Some(false) {
        return Err(Bank1WriteRefusal::NotImplemented);
    }
    let format = layout
        .content_format_id
        .and_then(LuminaireFormat::from_content_format_id)
        .ok_or(Bank1WriteRefusal::NotPart251)?;
    if format < field.since {
        return Err(Bank1WriteRefusal::FieldNotInFormat);
    }
    let last = layout.last_offset.ok_or(Bank1WriteRefusal::NotPart251)?;
    if last < field.last_offset() {
        return Err(Bank1WriteRefusal::TooShort);
    }
    Ok(format)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldWrite {
    pub field: &'static LuminaireField,
    pub bytes: Vec<u8>,
}

impl FieldWrite {
    pub fn end(&self) -> usize {
        usize::from(self.field.offset) + self.bytes.len()
    }

    pub fn held_by(&self, bank: &[u8]) -> bool {
        bank.get(usize::from(self.field.offset)..self.end()) == Some(self.bytes.as_slice())
    }
}

fn number_values(numbers: &N) -> [(u16, u32); 11] {
    [
        (N::PATCH_YEAR, u32::from(numbers.year)),
        (N::PATCH_WEEK, u32::from(numbers.week)),
        (N::PATCH_NOMINAL_INPUT_POWER_W, u32::from(numbers.nominal_input_power_w)),
        (N::PATCH_POWER_AT_MINIMUM_W, u32::from(numbers.power_at_minimum_w)),
        (N::PATCH_NOMINAL_MIN_AC_VOLTAGE_V, u32::from(numbers.nominal_min_ac_voltage_v)),
        (N::PATCH_NOMINAL_MAX_AC_VOLTAGE_V, u32::from(numbers.nominal_max_ac_voltage_v)),
        (N::PATCH_NOMINAL_LIGHT_OUTPUT_LM, numbers.nominal_light_output_lm),
        (N::PATCH_CRI, u32::from(numbers.cri)),
        (N::PATCH_CCT_KELVIN, u32::from(numbers.cct_kelvin)),
        (N::PATCH_LIGHT_DISTRIBUTION_TYPE, u32::from(numbers.light_distribution_type)),
        (N::PATCH_LAMP_CURRENT_MA, u32::from(numbers.lamp_current_ma)),
    ]
}

pub fn number_patch(values: &[(u16, u32)]) -> N {
    let value = |patch: u16| {
        values.iter().find(|(bit, _)| *bit == patch).map_or(0, |(_, value)| *value)
    };
    N {
        patch_mask: values.iter().fold(0, |mask, (bit, _)| mask | bit),
        year: value(N::PATCH_YEAR) as u8,
        week: value(N::PATCH_WEEK) as u8,
        nominal_input_power_w: value(N::PATCH_NOMINAL_INPUT_POWER_W) as u16,
        power_at_minimum_w: value(N::PATCH_POWER_AT_MINIMUM_W) as u16,
        nominal_min_ac_voltage_v: value(N::PATCH_NOMINAL_MIN_AC_VOLTAGE_V) as u16,
        nominal_max_ac_voltage_v: value(N::PATCH_NOMINAL_MAX_AC_VOLTAGE_V) as u16,
        nominal_light_output_lm: value(N::PATCH_NOMINAL_LIGHT_OUTPUT_LM),
        cri: value(N::PATCH_CRI) as u8,
        cct_kelvin: value(N::PATCH_CCT_KELVIN) as u16,
        light_distribution_type: value(N::PATCH_LIGHT_DISTRIBUTION_TYPE) as u8,
        lamp_current_ma: value(N::PATCH_LAMP_CURRENT_MA) as u16,
    }
}

pub fn text_patch(field: &LuminaireField, text: &str) -> Option<LuminaireTextWrite> {
    let FieldKind::Text(which) = field.kind else {
        return None;
    };
    let mut stored = LuminaireText::new();
    stored.push_str(text).ok()?;
    Some(LuminaireTextWrite { field: which, text: stored })
}

fn number_write(field: &'static LuminaireField, numbers: &N) -> Option<FieldWrite> {
    let FieldKind::Number { patch, .. } = field.kind else {
        return None;
    };
    if numbers.patch_mask & patch == 0 {
        return None;
    }
    let (_, value) = number_values(numbers).into_iter().find(|(bit, _)| *bit == patch)?;
    let wide = value.to_be_bytes();
    Some(FieldWrite { field, bytes: wide[wide.len() - usize::from(field.len)..].to_vec() })
}

// DiiA 251 §9.2.5 footnote d
fn text_write(write: &LuminaireTextWrite) -> Option<FieldWrite> {
    let field = LuminaireField::of_text(write.field)?;
    let mut bytes = write.text.as_bytes().to_vec();
    if bytes.len() < usize::from(field.len) {
        bytes.push(0);
    }
    bytes.truncate(usize::from(field.len));
    Some(FieldWrite { field, bytes })
}

pub fn planned_writes(numbers: &N, text: Option<&LuminaireTextWrite>) -> Vec<FieldWrite> {
    let mut writes: Vec<FieldWrite> = LUMINAIRE_FIELDS
        .iter()
        .filter_map(|field| number_write(field, numbers))
        .collect();
    writes.extend(text.and_then(text_write));
    writes.sort_by_key(|write| write.field.offset);
    writes
}

#[cfg(test)]
mod tests;
