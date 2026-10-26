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
use serde::ser::SerializeMap;

#[derive(Clone, Debug, Serialize)]
pub struct ObservedValueDto<V> {
    pub last_read_ms: Option<u64>,
    pub last_write_confirmed_ms: Option<u64>,
    pub source: &'static str,
    pub value: V,
}

impl<V> ObservedValueDto<V> {
    fn map<U>(self, f: impl FnOnce(V) -> U) -> ObservedValueDto<U> {
        ObservedValueDto {
            last_read_ms: self.last_read_ms,
            last_write_confirmed_ms: self.last_write_confirmed_ms,
            source: self.source,
            value: f(self.value),
        }
    }
}

fn observed_opt<T>(value: &Option<ObservedValue<T>>) -> Option<ObservedValueDto<&T>> {
    value.as_ref().map(|v| ObservedValueDto {
        last_read_ms: v.last_read_ms,
        last_write_confirmed_ms: v.last_write_confirmed_ms,
        source: match v.source {
            AttributeSource::Readback => "readback",
            AttributeSource::WriteConfirmed => "write_confirmed",
        },
        value: &v.value,
    })
}

macro_rules! attribute_section_dto {
    ($dto:ident from $view:ty { $($field:ident: $ty:ty),+ $(,)? }) => {
        #[derive(Clone, Debug, Serialize)]
        pub struct $dto<'a> {
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<ObservedValueDto<&'a $ty>>,
            )+
        }

        impl<'a> $dto<'a> {
            fn from_view(v: &'a $view) -> Option<Self> {
                if !Self::is_present(v) {
                    return None;
                }
                Some(Self { $($field: observed_opt(&v.$field),)+ })
            }

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
        if !Self::is_present(v) {
            return None;
        }
        Some(Self {
            configuration: observed_opt(&v.configuration)
                .map(|o| o.map(|raw| BusUnitConfigurationDto::from_raw(*raw))),
            implemented_parts: observed_opt(&v.implemented_parts)
                .map(|o| o.map(|raw| ImplementedPartsDto::from_raw(*raw))),
        })
    }

    fn is_present(v: &MemoryBusUnitAttributesView) -> bool {
        v.configuration.is_some() || v.implemented_parts.is_some()
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
        Self::is_present(v).then_some(Self(v))
    }

    fn is_present(v: &MemoryEnergyAttributesView) -> bool {
        EnergyBankDto::is_present(&v.active)
            || EnergyBankDto::is_present(&v.apparent)
            || EnergyBankDto::is_present(&v.loadside)
    }
}

impl Serialize for MemoryEnergyAttributesDto<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (key, view) in [
            ("active", &self.0.active),
            ("apparent", &self.0.apparent),
            ("loadside", &self.0.loadside),
        ] {
            serialize_present_entry(&mut map, key, || EnergyBankDto::from_view(view))?;
        }
        map.end()
    }
}

attribute_section_dto!(ConditionDto from ConditionView {
    counter: BankReading,
    flag: BankReading,
});

#[derive(Clone, Debug, Serialize)]
pub struct GearDiagnosticsDto<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_version: Option<ObservedValueDto<&'a u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operating_time_s: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_current_percent: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_power_limitation: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_failure: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overvoltage: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_factor_centi: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_frequency_hz: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_voltage_decivolt: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_offset60: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_derating: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_shutdown: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undervoltage: Option<ConditionDto<'a>>,
}

impl<'a> GearDiagnosticsDto<'a> {
    fn from_view(v: &'a GearDiagnosticsView) -> Option<Self> {
        if !Self::is_present(v) {
            return None;
        }
        Some(Self {
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
        })
    }

    fn is_present(v: &GearDiagnosticsView) -> bool {
        v.bank_version.is_some()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceDiagnosticsDto<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_version: Option<ObservedValueDto<&'a u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_milliamp: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_time_resettable_s: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_time_s: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_circuit: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_failure: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_circuit: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_counter_resettable: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_offset60: Option<ObservedValueDto<&'a BankReading>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_derating: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_shutdown: Option<ConditionDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_decivolt: Option<ObservedValueDto<&'a BankReading>>,
}

impl<'a> SourceDiagnosticsDto<'a> {
    fn from_view(v: &'a SourceDiagnosticsView) -> Option<Self> {
        if !Self::is_present(v) {
            return None;
        }
        Some(Self {
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
        })
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
        Self::is_present(v).then_some(Self(v))
    }

    fn is_present(v: &MemoryDiagnosticsAttributesView) -> bool {
        GearDiagnosticsDto::is_present(&v.control_gear)
            || SourceDiagnosticsDto::is_present(&v.light_source)
            || LuminaireMaintenanceDto::is_present(&v.luminaire)
    }
}

impl Serialize for MemoryDiagnosticsAttributesDto<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let v = self.0;
        serialize_present_entry(&mut map, "control_gear", || {
            GearDiagnosticsDto::from_view(&v.control_gear)
        })?;
        serialize_present_entry(&mut map, "light_source", || {
            SourceDiagnosticsDto::from_view(&v.light_source)
        })?;
        serialize_present_entry(&mut map, "luminaire", || {
            LuminaireMaintenanceDto::from_view(&v.luminaire)
        })?;
        map.end()
    }
}

const SCENE_LEX_ORDER: [usize; 16] = [0, 1, 10, 11, 12, 13, 14, 15, 2, 3, 4, 5, 6, 7, 8, 9];
const SCENE_KEYS: [&str; 16] = [
    "scene_0", "scene_1", "scene_2", "scene_3", "scene_4", "scene_5", "scene_6", "scene_7",
    "scene_8", "scene_9", "scene_10", "scene_11", "scene_12", "scene_13", "scene_14", "scene_15",
];

pub struct ScenesAttributesDto<'a>(&'a ScenesAttributesView);

impl<'a> ScenesAttributesDto<'a> {
    fn from_view(v: &'a ScenesAttributesView) -> Option<Self> {
        Self::is_present(v).then_some(Self(v))
    }

    fn is_present(v: &ScenesAttributesView) -> bool {
        v.levels.iter().any(Option::is_some)
    }
}

impl Serialize for ScenesAttributesDto<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for idx in SCENE_LEX_ORDER {
            if let Some(slot) = observed_opt(&self.0.levels[idx]) {
                map.serialize_entry(SCENE_KEYS[idx], &slot)?;
            }
        }
        map.end()
    }
}

pub struct AttributeSectionDto<'a>(&'a AttributeSectionView);

impl<'a> AttributeSectionDto<'a> {
    pub fn from_view(view: &'a AttributeSectionView) -> Option<Self> {
        let present = match view {
            AttributeSectionView::Common102(v) => Common102AttributesDto::is_present(v),
            AttributeSectionView::Dt6Led(v) => Dt6LedAttributesDto::is_present(v),
            AttributeSectionView::Dt8Color(v) => Dt8ColorAttributesDto::is_present(v),
            AttributeSectionView::Extended(v) => ExtendedAttributesDto::is_present(v),
            AttributeSectionView::Groups(v) => GroupsAttributesDto::is_present(v),
            AttributeSectionView::MemoryDiagnostics(v) => MemoryDiagnosticsAttributesDto::is_present(v),
            AttributeSectionView::MemoryEnergy(v) => MemoryEnergyAttributesDto::is_present(v),
            AttributeSectionView::MemoryBusUnit(v) => MemoryBusUnitAttributesDto::is_present(v),
            AttributeSectionView::MemoryIdentity(v) => MemoryIdentityAttributesDto::is_present(v),
            AttributeSectionView::MemoryLuminaire(v) => MemoryLuminaireAttributesDto::is_present(v),
            AttributeSectionView::MemoryProfile(v) => MemoryProfileAttributesDto::is_present(v),
            AttributeSectionView::Scenes(v) => ScenesAttributesDto::is_present(v),
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
                serialize_section(s, || Common102AttributesDto::from_view(v))
            }
            AttributeSectionView::Dt6Led(v) => {
                serialize_section(s, || Dt6LedAttributesDto::from_view(v))
            }
            AttributeSectionView::Dt8Color(v) => {
                serialize_section(s, || Dt8ColorAttributesDto::from_view(v))
            }
            AttributeSectionView::Extended(v) => {
                serialize_section(s, || ExtendedAttributesDto::from_view(v))
            }
            AttributeSectionView::Groups(v) => {
                serialize_section(s, || GroupsAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryDiagnostics(v) => {
                serialize_section(s, || MemoryDiagnosticsAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryEnergy(v) => {
                serialize_section(s, || MemoryEnergyAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryBusUnit(v) => {
                serialize_section(s, || MemoryBusUnitAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryIdentity(v) => {
                serialize_section(s, || MemoryIdentityAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryLuminaire(v) => {
                serialize_section(s, || MemoryLuminaireAttributesDto::from_view(v))
            }
            AttributeSectionView::MemoryProfile(v) => {
                serialize_section(s, || MemoryProfileAttributesDto::from_view(v))
            }
            AttributeSectionView::Scenes(v) => {
                serialize_section(s, || ScenesAttributesDto::from_view(v))
            }
        }
    }
}

const SECTION_PRESENT: &str = "AttributeSectionDto only exists for a non-empty section";

#[inline(never)]
fn serialize_section<S: serde::Serializer, D: Serialize>(
    s: S,
    build: impl FnOnce() -> Option<D>,
) -> Result<S::Ok, S::Error> {
    build().as_ref().expect(SECTION_PRESENT).serialize(s)
}

#[inline(never)]
fn serialize_present_entry<M: SerializeMap, D: Serialize>(
    map: &mut M,
    key: &'static str,
    build: impl FnOnce() -> Option<D>,
) -> Result<(), M::Error> {
    match &build() {
        Some(dto) => map.serialize_entry(key, dto),
        None => Ok(()),
    }
}

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
