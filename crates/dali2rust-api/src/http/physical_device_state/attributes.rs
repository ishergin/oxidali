use dali2rust_contracts::msg::{FixedText32, FixedText64};
use dali2rust_domain::dali::banks::{BusUnitConfiguration, ImplementedParts};
use dali2rust_domain::registry::{
    AttributeSectionView, AttributeSource, BankReading, Common102AttributesView, ConditionView,
    Dt6LedAttributesView, Dt8ColorAttributesView, EnergyBankView, ExtendedAttributesView,
    GearDiagnosticsView, GroupsAttributesView, LuminaireMaintenanceView,
    LuminaireValue, MemoryBusUnitAttributesView, MemoryDiagnosticsAttributesView,
    MemoryEnergyAttributesView, MemoryIdentityAttributesView, MemoryLuminaireAttributesView,
    MemoryProfileAttributesView, ObservedValue, ScenesAttributesView, SourceDiagnosticsView,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ObservedValueDto<T> {
    pub last_read_ms: Option<u64>,
    pub last_write_confirmed_ms: Option<u64>,
    pub source: &'static str,
    pub value: T,
}

impl<T> ObservedValueDto<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> ObservedValueDto<U> {
        ObservedValueDto {
            last_read_ms: self.last_read_ms,
            last_write_confirmed_ms: self.last_write_confirmed_ms,
            source: self.source,
            value: f(self.value),
        }
    }
}

fn observed_opt<T: Clone>(value: &Option<ObservedValue<T>>) -> Option<ObservedValueDto<T>> {
    value.as_ref().map(|v| ObservedValueDto {
        last_read_ms: v.last_read_ms,
        last_write_confirmed_ms: v.last_write_confirmed_ms,
        source: match v.source {
            AttributeSource::Readback => "readback",
            AttributeSource::WriteConfirmed => "write_confirmed",
        },
        value: v.value.clone(),
    })
}

macro_rules! attribute_section_dto {
    ($dto:ident from $view:ty { $($field:ident: $ty:ty),+ $(,)? }) => {
        #[derive(Clone, Debug, Serialize)]
        pub struct $dto {
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<ObservedValueDto<$ty>>,
            )+
        }

        impl $dto {
            fn from_view(v: &$view) -> Option<Self> {
                let dto = Self { $($field: observed_opt(&v.$field),)+ };
                ($(dto.$field.is_some())||+).then_some(dto)
            }

            #[allow(dead_code, reason = "generated for every section; only container-gated sections call it")]
            fn is_present(v: &$view) -> bool {
                $(v.$field.is_some())||+
            }
        }
    };
}

attribute_section_dto!(Common102AttributesDto from Common102AttributesView {
    device_type: u8,
    fade_rate: u8,
    // IEC 62386-102 Table 4
    fade_time_ms: u32,
    light_source_type: u8,
    light_source_types: u32,
    max_level: u8,
    min_level: u8,
    physical_minimum: u8,
    power_on_level: u8,
    system_failure_level: u8,
    version: u8,
});

attribute_section_dto!(Dt6LedAttributesDto from Dt6LedAttributesView {
    current_protector_active: u8,
    current_protector_enabled: u8,
    dimming_curve: u8,
    extended_version_number: u8,
    failure_status: u8,
    fast_fade_time: u8,
    features: u8,
    gear_type: u8,
    load_decrease: u8,
    load_increase: u8,
    min_fast_fade_time: u8,
    open_circuit: u8,
    operating_mode: u8,
    possible_operating_mode: u8,
    reference_measurement_failed: u8,
    reference_running: u8,
    short_circuit: u8,
    thermal_overload: u8,
    thermal_shutdown: u8,
});

attribute_section_dto!(Dt8ColorAttributesDto from Dt8ColorAttributesView {
    color_type: u8,
    color_value_0: u16,
    color_value_1: u16,
    color_value_2: u16,
    gear_features: u8,
    rgbwaf_control: u8,
});

attribute_section_dto!(ExtendedAttributesDto from ExtendedAttributesView {
    fade_time_ms: u16,
    version_number: u8,
});

attribute_section_dto!(GroupsAttributesDto from GroupsAttributesView {
    membership: u16,
});

attribute_section_dto!(MemoryIdentityAttributesDto from MemoryIdentityAttributesView {
    dali_101_version: u8,
    dali_102_version: u8,
    dali_103_version: u8,
    firmware_version_major: u8,
    firmware_version_minor: u8,
    gtin: u64,
    hardware_version_major: u8,
    hardware_version_minor: u8,
    identification_number: u64,
    last_memory_bank: u8,
    logical_control_device_units: u8,
    logical_control_gear_index: u8,
    logical_control_gear_units: u8,
});

attribute_section_dto!(MemoryProfileAttributesDto from MemoryProfileAttributesView {
    bank1_lock_byte: u8,
    oem_gtin: u64,
    oem_identification_number: u64,
});

#[derive(Clone, Debug, Serialize)]
pub struct BusUnitConfigurationDto {
    pub raw: u8,
    pub class: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency_type: Option<char>,
}

impl BusUnitConfigurationDto {
    fn from_raw(raw: u8) -> Self {
        let class = BusUnitConfiguration::from_byte(raw);
        Self {
            raw,
            class: class.label(),
            emergency_type: class.emergency_letter(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ImplementedPartsDto {
    pub raw: u8,
    pub parts: Option<Vec<u16>>,
}

impl ImplementedPartsDto {
    fn from_raw(raw: u8) -> Self {
        let parts = ImplementedParts::from_byte(raw);
        Self {
            raw,
            parts: parts.in_range().then(|| parts.parts().collect()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryBusUnitAttributesDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<ObservedValueDto<BusUnitConfigurationDto>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implemented_parts: Option<ObservedValueDto<ImplementedPartsDto>>,
}

impl MemoryBusUnitAttributesDto {
    fn from_view(v: &MemoryBusUnitAttributesView) -> Option<Self> {
        let dto = Self {
            configuration: observed_opt(&v.configuration)
                .map(|o| o.map(BusUnitConfigurationDto::from_raw)),
            implemented_parts: observed_opt(&v.implemented_parts)
                .map(|o| o.map(ImplementedPartsDto::from_raw)),
        };
        (dto.configuration.is_some() || dto.implemented_parts.is_some()).then_some(dto)
    }
}

attribute_section_dto!(MemoryLuminaireAttributesDto from MemoryLuminaireAttributesView {
    cct_kelvin: LuminaireValue,
    content_format_id: u16,
    cri: LuminaireValue,
    customer_stocking_number: FixedText32,
    free_use: FixedText32,
    lamp_current_ma: LuminaireValue,
    light_distribution: FixedText32,
    light_distribution_type: LuminaireValue,
    luminaire_colour: FixedText32,
    luminaire_identification: FixedText64,
    nominal_input_power_w: LuminaireValue,
    nominal_light_output_lm: LuminaireValue,
    nominal_max_ac_voltage_v: LuminaireValue,
    nominal_min_ac_voltage_v: LuminaireValue,
    oem_name: FixedText32,
    power_at_minimum_w: LuminaireValue,
    week: LuminaireValue,
    year: LuminaireValue,
});

attribute_section_dto!(EnergyBankDto from EnergyBankView {
    bank_version: u8,
    energy: BankReading,
    energy_scale: i8,
    power: BankReading,
    power_scale: i8,
});

pub struct MemoryEnergyAttributesDto<'a>(&'a MemoryEnergyAttributesView);

impl<'a> MemoryEnergyAttributesDto<'a> {
    fn from_view(v: &'a MemoryEnergyAttributesView) -> Option<Self> {
        let any = EnergyBankDto::from_view(&v.active).is_some()
            || EnergyBankDto::from_view(&v.apparent).is_some()
            || EnergyBankDto::from_view(&v.loadside).is_some();
        any.then_some(Self(v))
    }
}

impl Serialize for MemoryEnergyAttributesDto<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        for (key, view) in [
            ("active", &self.0.active),
            ("apparent", &self.0.apparent),
            ("loadside", &self.0.loadside),
        ] {
            if let Some(bank) = EnergyBankDto::from_view(view) {
                map.serialize_entry(key, &bank)?;
            }
        }
        map.end()
    }
}

attribute_section_dto!(ConditionDto from ConditionView {
    counter: BankReading,
    flag: BankReading,
});

#[derive(Clone, Debug, Serialize)]
pub struct GearDiagnosticsDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_version: Option<ObservedValueDto<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operating_time_s: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_current_percent: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_power_limitation: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_failure: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overvoltage: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_factor_centi: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_frequency_hz: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_voltage_decivolt: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_offset60: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_derating: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_shutdown: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undervoltage: Option<ConditionDto>,
}

impl GearDiagnosticsDto {
    fn from_view(v: &GearDiagnosticsView) -> Option<Self> {
        let dto = Self {
            bank_version: observed_opt(&v.bank_version),
            operating_time_s: observed_opt(&v.operating_time_s),
            output_current_percent: observed_opt(&v.output_current_percent),
            output_power_limitation: ConditionDto::from_view(&v.output_power_limitation),
            overall_failure: ConditionDto::from_view(&v.overall_failure),
            overvoltage: ConditionDto::from_view(&v.overvoltage),
            power_factor_centi: observed_opt(&v.power_factor_centi),
            start_counter: observed_opt(&v.start_counter),
            supply_frequency_hz: observed_opt(&v.supply_frequency_hz),
            supply_voltage_decivolt: observed_opt(&v.supply_voltage_decivolt),
            temperature_offset60: observed_opt(&v.temperature_offset60),
            thermal_derating: ConditionDto::from_view(&v.thermal_derating),
            thermal_shutdown: ConditionDto::from_view(&v.thermal_shutdown),
            undervoltage: ConditionDto::from_view(&v.undervoltage),
        };
        dto.bank_version.is_some().then_some(dto)
    }

    fn is_present(v: &GearDiagnosticsView) -> bool {
        v.bank_version.is_some()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceDiagnosticsDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_version: Option<ObservedValueDto<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_milliamp: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_time_resettable_s: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_time_s: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_circuit: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_failure: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_circuit: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter_resettable: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_offset60: Option<ObservedValueDto<BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_derating: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_shutdown: Option<ConditionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_decivolt: Option<ObservedValueDto<BankReading>>,
}

impl SourceDiagnosticsDto {
    fn from_view(v: &SourceDiagnosticsView) -> Option<Self> {
        let dto = Self {
            bank_version: observed_opt(&v.bank_version),
            current_milliamp: observed_opt(&v.current_milliamp),
            on_time_resettable_s: observed_opt(&v.on_time_resettable_s),
            on_time_s: observed_opt(&v.on_time_s),
            open_circuit: ConditionDto::from_view(&v.open_circuit),
            overall_failure: ConditionDto::from_view(&v.overall_failure),
            short_circuit: ConditionDto::from_view(&v.short_circuit),
            start_counter: observed_opt(&v.start_counter),
            start_counter_resettable: observed_opt(&v.start_counter_resettable),
            temperature_offset60: observed_opt(&v.temperature_offset60),
            thermal_derating: ConditionDto::from_view(&v.thermal_derating),
            thermal_shutdown: ConditionDto::from_view(&v.thermal_shutdown),
            voltage_decivolt: observed_opt(&v.voltage_decivolt),
        };
        dto.bank_version.is_some().then_some(dto)
    }

    fn is_present(v: &SourceDiagnosticsView) -> bool {
        v.bank_version.is_some()
    }
}

attribute_section_dto!(LuminaireMaintenanceDto from LuminaireMaintenanceView {
    bank_version: u8,
    rated_life_kilohours: BankReading,
    rated_starts_hundreds: BankReading,
    reference_temperature_offset60: BankReading,
});

pub struct MemoryDiagnosticsAttributesDto<'a>(&'a MemoryDiagnosticsAttributesView);

impl<'a> MemoryDiagnosticsAttributesDto<'a> {
    fn from_view(v: &'a MemoryDiagnosticsAttributesView) -> Option<Self> {
        let any = GearDiagnosticsDto::is_present(&v.control_gear)
            || SourceDiagnosticsDto::is_present(&v.light_source)
            || LuminaireMaintenanceDto::is_present(&v.luminaire);
        any.then_some(Self(v))
    }
}

impl Serialize for MemoryDiagnosticsAttributesDto<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        if let Some(gear) = GearDiagnosticsDto::from_view(&self.0.control_gear) {
            map.serialize_entry("control_gear", &gear)?;
        }
        if let Some(source) = SourceDiagnosticsDto::from_view(&self.0.light_source) {
            map.serialize_entry("light_source", &source)?;
        }
        if let Some(luminaire) = LuminaireMaintenanceDto::from_view(&self.0.luminaire) {
            map.serialize_entry("luminaire", &luminaire)?;
        }
        map.end()
    }
}

const SCENE_LEX_ORDER: [usize; 16] = [0, 1, 10, 11, 12, 13, 14, 15, 2, 3, 4, 5, 6, 7, 8, 9];
const SCENE_KEYS: [&str; 16] = [
    "scene_0", "scene_1", "scene_2", "scene_3", "scene_4", "scene_5", "scene_6", "scene_7",
    "scene_8", "scene_9", "scene_10", "scene_11", "scene_12", "scene_13", "scene_14", "scene_15",
];

#[derive(Clone, Debug)]
pub struct ScenesAttributesDto {
    pub levels: [Option<ObservedValueDto<u8>>; 16],
}

impl ScenesAttributesDto {
    fn from_view(v: &ScenesAttributesView) -> Option<Self> {
        let levels = core::array::from_fn(|idx| observed_opt(&v.levels[idx]));
        levels.iter().any(Option::is_some).then_some(Self { levels })
    }
}

impl Serialize for ScenesAttributesDto {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        for idx in SCENE_LEX_ORDER {
            if let Some(slot) = &self.levels[idx] {
                map.serialize_entry(SCENE_KEYS[idx], slot)?;
            }
        }
        map.end()
    }
}

pub struct AttributeSectionDto<'a>(&'a AttributeSectionView);

impl<'a> AttributeSectionDto<'a> {
    pub fn from_view(view: &'a AttributeSectionView) -> Option<Self> {
        let present = match view {
            AttributeSectionView::Common102(v) => Common102AttributesDto::from_view(v).is_some(),
            AttributeSectionView::Dt6Led(v) => Dt6LedAttributesDto::from_view(v).is_some(),
            AttributeSectionView::Dt8Color(v) => Dt8ColorAttributesDto::from_view(v).is_some(),
            AttributeSectionView::Extended(v) => ExtendedAttributesDto::from_view(v).is_some(),
            AttributeSectionView::Groups(v) => GroupsAttributesDto::from_view(v).is_some(),
            AttributeSectionView::MemoryDiagnostics(v) => {
                MemoryDiagnosticsAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::MemoryEnergy(v) => {
                MemoryEnergyAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::MemoryBusUnit(v) => {
                MemoryBusUnitAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::MemoryIdentity(v) => {
                MemoryIdentityAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::MemoryLuminaire(v) => {
                MemoryLuminaireAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::MemoryProfile(v) => {
                MemoryProfileAttributesDto::from_view(v).is_some()
            }
            AttributeSectionView::Scenes(v) => ScenesAttributesDto::from_view(v).is_some(),
        };
        present.then_some(Self(view))
    }

    pub fn key(&self) -> &'static str {
        self.0.kind().wire_name()
    }
}

impl Serialize for AttributeSectionDto<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            AttributeSectionView::Common102(v) => {
                Common102AttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::Dt6Led(v) => {
                Dt6LedAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::Dt8Color(v) => {
                Dt8ColorAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::Extended(v) => {
                ExtendedAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::Groups(v) => {
                GroupsAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryDiagnostics(v) => {
                MemoryDiagnosticsAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryEnergy(v) => {
                MemoryEnergyAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryBusUnit(v) => {
                MemoryBusUnitAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryIdentity(v) => {
                MemoryIdentityAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryLuminaire(v) => {
                MemoryLuminaireAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::MemoryProfile(v) => {
                MemoryProfileAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
            AttributeSectionView::Scenes(v) => {
                ScenesAttributesDto::from_view(v).expect(SECTION_PRESENT).serialize(s)
            }
        }
    }
}

const SECTION_PRESENT: &str = "AttributeSectionDto only exists for a non-empty section";

pub fn write_attribute_section(
    w: &mut dyn std::io::Write,
    view: &AttributeSectionView,
    first: &mut bool,
) -> std::io::Result<()> {
    let Some(dto) = AttributeSectionDto::from_view(view) else {
        return Ok(());
    };
    if !*first {
        w.write_all(b",")?;
    }
    *first = false;
    write!(w, "\"{}\":", dto.key())?;
    serde_json::to_writer(&mut *w, &dto).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::registry::{
        BankReading, GearDiagnosticsView, LuminaireMaintenanceView, ObservedValue,
        SourceDiagnosticsView,
    };

    fn read<T>(value: T) -> Option<ObservedValue<T>> {
        Some(ObservedValue {
            value,
            last_read_ms: Some(1),
            last_write_confirmed_ms: None,
            source: AttributeSource::Readback,
        })
    }

    #[test]
    fn the_cheap_section_predicate_agrees_with_building_the_dto() {
        let mut gear = GearDiagnosticsView::default();
        let mut source = SourceDiagnosticsView::default();
        let mut luminaire = LuminaireMaintenanceView::default();

        for (label, got, want) in [
            (
                "empty control gear",
                GearDiagnosticsDto::is_present(&gear),
                GearDiagnosticsDto::from_view(&gear).is_some(),
            ),
            (
                "empty light source",
                SourceDiagnosticsDto::is_present(&source),
                SourceDiagnosticsDto::from_view(&source).is_some(),
            ),
            (
                "empty luminaire",
                LuminaireMaintenanceDto::is_present(&luminaire),
                LuminaireMaintenanceDto::from_view(&luminaire).is_some(),
            ),
        ] {
            assert_eq!(got, want, "{label}");
            assert!(!got, "{label}: nothing was read, so nothing is present");
        }

        gear.bank_version = read(1u8);
        source.bank_version = read(1u8);
        assert!(GearDiagnosticsDto::is_present(&gear));
        assert_eq!(
            GearDiagnosticsDto::is_present(&gear),
            GearDiagnosticsDto::from_view(&gear).is_some(),
            "control gear with only a bank version"
        );
        assert_eq!(
            SourceDiagnosticsDto::is_present(&source),
            SourceDiagnosticsDto::from_view(&source).is_some(),
            "light source with only a bank version"
        );

        luminaire.rated_life_kilohours = read(BankReading {
            value: Some(50),
            ..Default::default()
        });
        assert!(LuminaireMaintenanceDto::is_present(&luminaire));
        assert_eq!(
            LuminaireMaintenanceDto::is_present(&luminaire),
            LuminaireMaintenanceDto::from_view(&luminaire).is_some(),
            "luminaire with one non-first field read"
        );
    }
}
