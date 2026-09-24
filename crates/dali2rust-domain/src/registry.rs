use core::mem::MaybeUninit;
use std::borrow::Cow;
use core::ptr::addr_of_mut;

use serde::{Deserialize, Serialize};
use dali2rust_contracts::msg::{
    ColorMode, DeviceTypeSet, FixedText32, FixedText64, OperationType,
};
use dali2rust_platform::small_sort::insertion_sort_by;

pub const VIRTUAL_LAMP_COUNT: u8 = 64;

pub const GROUP_COUNT: u8 = 16;

pub const SCENE_COUNT: u8 = 16;

pub const MAX_SHORT_ADDRESSES: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualLampSnapshot {
    pub name: String,
    pub runtime_level: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterSnapshot {
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributeSource {
    #[default]
    Readback,
    WriteConfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValue<T> {
    pub value: T,
    pub source: AttributeSource,
    pub last_read_ms: Option<u64>,
    pub last_write_confirmed_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Common102AttributesView {
    pub version: Option<ObservedValue<u8>>,
    pub device_type: Option<ObservedValue<u8>>,
    pub physical_minimum: Option<ObservedValue<u8>>,
    pub min_level: Option<ObservedValue<u8>>,
    pub max_level: Option<ObservedValue<u8>>,
    pub power_on_level: Option<ObservedValue<u8>>,
    pub system_failure_level: Option<ObservedValue<u8>>,
    pub fade_time_ms: Option<ObservedValue<u32>>,
    pub fade_rate: Option<ObservedValue<u8>>,
    pub light_source_type: Option<ObservedValue<u8>>,
    pub light_source_types: Option<ObservedValue<u32>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupsAttributesView {
    pub membership: Option<ObservedValue<u16>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenesAttributesView {
    pub levels: [Option<ObservedValue<u8>>; 16],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dt6LedAttributesView {
    pub gear_type: Option<ObservedValue<u8>>,
    pub dimming_curve: Option<ObservedValue<u8>>,
    pub possible_operating_mode: Option<ObservedValue<u8>>,
    pub features: Option<ObservedValue<u8>>,
    pub failure_status: Option<ObservedValue<u8>>,
    pub short_circuit: Option<ObservedValue<u8>>,
    pub open_circuit: Option<ObservedValue<u8>>,
    pub load_decrease: Option<ObservedValue<u8>>,
    pub load_increase: Option<ObservedValue<u8>>,
    pub current_protector_active: Option<ObservedValue<u8>>,
    pub thermal_shutdown: Option<ObservedValue<u8>>,
    pub thermal_overload: Option<ObservedValue<u8>>,
    pub reference_running: Option<ObservedValue<u8>>,
    pub reference_measurement_failed: Option<ObservedValue<u8>>,
    pub current_protector_enabled: Option<ObservedValue<u8>>,
    pub operating_mode: Option<ObservedValue<u8>>,
    pub fast_fade_time: Option<ObservedValue<u8>>,
    pub min_fast_fade_time: Option<ObservedValue<u8>>,
    pub extended_version_number: Option<ObservedValue<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dt8ColorAttributesView {
    pub color_type: Option<ObservedValue<u8>>,
    pub color_value_0: Option<ObservedValue<u16>>,
    pub color_value_1: Option<ObservedValue<u16>>,
    pub color_value_2: Option<ObservedValue<u16>>,
    pub gear_features: Option<ObservedValue<u8>>,
    pub rgbwaf_control: Option<ObservedValue<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedAttributesView {
    pub fade_time_ms: Option<ObservedValue<u16>>,
    pub version_number: Option<ObservedValue<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryIdentityAttributesView {
    pub last_memory_bank: Option<ObservedValue<u8>>,
    pub gtin: Option<ObservedValue<u64>>,
    pub firmware_version_major: Option<ObservedValue<u8>>,
    pub firmware_version_minor: Option<ObservedValue<u8>>,
    pub identification_number: Option<ObservedValue<u64>>,
    pub hardware_version_major: Option<ObservedValue<u8>>,
    pub hardware_version_minor: Option<ObservedValue<u8>>,
    pub dali_101_version: Option<ObservedValue<u8>>,
    pub dali_102_version: Option<ObservedValue<u8>>,
    pub dali_103_version: Option<ObservedValue<u8>>,
    pub logical_control_device_units: Option<ObservedValue<u8>>,
    pub logical_control_gear_units: Option<ObservedValue<u8>>,
    pub logical_control_gear_index: Option<ObservedValue<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProfileAttributesView {
    pub bank1_lock_byte: Option<ObservedValue<u8>>,
    pub oem_gtin: Option<ObservedValue<u64>>,
    pub oem_identification_number: Option<ObservedValue<u64>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankReading {
    pub value: Option<u64>,
    pub not_implemented: bool,
    pub temporarily_unavailable: bool,
    pub tmask_since_ms: Option<u64>,
    pub saturated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBusUnitAttributesView {
    pub configuration: Option<ObservedValue<u8>>,
    pub implemented_parts: Option<ObservedValue<ImplementedPartsView>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementedPartsView {
    pub raw: u16,
    pub bytes: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuminaireValue {
    pub raw: u32,
    pub value: Option<u32>,
    pub part209_implemented: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLuminaireAttributesView {
    pub content_format_id: Option<ObservedValue<u16>>,
    pub year: Option<ObservedValue<LuminaireValue>>,
    pub week: Option<ObservedValue<LuminaireValue>>,
    pub nominal_input_power_w: Option<ObservedValue<LuminaireValue>>,
    pub power_at_minimum_w: Option<ObservedValue<LuminaireValue>>,
    pub nominal_min_ac_voltage_v: Option<ObservedValue<LuminaireValue>>,
    pub nominal_max_ac_voltage_v: Option<ObservedValue<LuminaireValue>>,
    pub nominal_light_output_lm: Option<ObservedValue<LuminaireValue>>,
    pub cri: Option<ObservedValue<LuminaireValue>>,
    pub cct_kelvin: Option<ObservedValue<LuminaireValue>>,
    pub light_distribution_type: Option<ObservedValue<LuminaireValue>>,
    pub luminaire_colour: Option<ObservedValue<FixedText32>>,
    pub luminaire_identification: Option<ObservedValue<FixedText64>>,
    pub light_distribution: Option<ObservedValue<FixedText32>>,
    pub oem_name: Option<ObservedValue<FixedText32>>,
    pub customer_stocking_number: Option<ObservedValue<FixedText32>>,
    pub lamp_current_ma: Option<ObservedValue<LuminaireValue>>,
    pub free_use: Option<ObservedValue<FixedText32>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyBankView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub energy_scale: Option<ObservedValue<i8>>,
    pub energy: Option<ObservedValue<BankReading>>,
    pub power_scale: Option<ObservedValue<i8>>,
    pub power: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEnergyAttributesView {
    pub active: EnergyBankView,
    pub apparent: EnergyBankView,
    pub loadside: EnergyBankView,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConditionView {
    pub flag: Option<ObservedValue<BankReading>>,
    pub counter: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GearDiagnosticsView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub operating_time_s: Option<ObservedValue<BankReading>>,
    pub start_counter: Option<ObservedValue<BankReading>>,
    pub supply_voltage_decivolt: Option<ObservedValue<BankReading>>,
    pub supply_frequency_hz: Option<ObservedValue<BankReading>>,
    pub power_factor_centi: Option<ObservedValue<BankReading>>,
    pub overall_failure: ConditionView,
    pub undervoltage: ConditionView,
    pub overvoltage: ConditionView,
    pub output_power_limitation: ConditionView,
    pub thermal_derating: ConditionView,
    pub thermal_shutdown: ConditionView,
    pub temperature_offset60: Option<ObservedValue<BankReading>>,
    pub output_current_percent: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDiagnosticsView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub start_counter_resettable: Option<ObservedValue<BankReading>>,
    pub start_counter: Option<ObservedValue<BankReading>>,
    pub on_time_resettable_s: Option<ObservedValue<BankReading>>,
    pub on_time_s: Option<ObservedValue<BankReading>>,
    pub voltage_decivolt: Option<ObservedValue<BankReading>>,
    pub current_milliamp: Option<ObservedValue<BankReading>>,
    pub overall_failure: ConditionView,
    pub short_circuit: ConditionView,
    pub open_circuit: ConditionView,
    pub thermal_derating: ConditionView,
    pub thermal_shutdown: ConditionView,
    pub temperature_offset60: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuminaireMaintenanceView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub rated_life_kilohours: Option<ObservedValue<BankReading>>,
    pub reference_temperature_offset60: Option<ObservedValue<BankReading>>,
    pub rated_starts_hundreds: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDiagnosticsAttributesView {
    pub control_gear: GearDiagnosticsView,
    pub light_source: SourceDiagnosticsView,
    pub luminaire: LuminaireMaintenanceView,
}

impl MemoryDiagnosticsAttributesView {
    pub fn default_in_place(place: &mut MaybeUninit<Self>) {
        let p = place.as_mut_ptr();
        // SAFETY: `p` is a live, aligned, uninitialised `Self`; every field is written once and none is read.
        unsafe {
            addr_of_mut!((*p).control_gear).write(GearDiagnosticsView::default());
            addr_of_mut!((*p).light_source).write(SourceDiagnosticsView::default());
            addr_of_mut!((*p).luminaire).write(LuminaireMaintenanceView::default());
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalDeviceAttributesView {
    pub common102: Common102AttributesView,
    pub groups: GroupsAttributesView,
    pub scenes: ScenesAttributesView,
    pub dt6_led: Dt6LedAttributesView,
    pub dt8_color: Dt8ColorAttributesView,
    pub extended: ExtendedAttributesView,
    pub memory_identity: MemoryIdentityAttributesView,
    pub memory_profile: MemoryProfileAttributesView,
}

impl PhysicalDeviceAttributesView {
    pub fn default_in_place(place: &mut MaybeUninit<Self>) {
        let p = place.as_mut_ptr();
        // SAFETY: `p` is a live, aligned, uninitialised `Self`; every field is written once and none is read.
        unsafe {
            addr_of_mut!((*p).common102).write(Common102AttributesView::default());
            addr_of_mut!((*p).groups).write(GroupsAttributesView::default());
            addr_of_mut!((*p).scenes).write(ScenesAttributesView::default());
            addr_of_mut!((*p).dt6_led).write(Dt6LedAttributesView::default());
            addr_of_mut!((*p).dt8_color).write(Dt8ColorAttributesView::default());
            addr_of_mut!((*p).extended).write(ExtendedAttributesView::default());
            addr_of_mut!((*p).memory_identity).write(MemoryIdentityAttributesView::default());
            addr_of_mut!((*p).memory_profile).write(MemoryProfileAttributesView::default());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationSource {
    #[default]
    Api,
    Sniffer,
    Poller,
    Mqtt,
    Hcl,
    Cluster,
    AdapterProxy,
    Rules,
}

impl ObservationSource {
    pub const fn rest_name(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Sniffer => "sniffer",
            Self::Poller => "poller",
            Self::Mqtt => "mqtt",
            Self::Hcl => "hcl",
            Self::Cluster => "cluster",
            Self::AdapterProxy => "adapter_proxy",
            Self::Rules => "rules",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastDapcSourceView {
    #[default]
    Unknown,
    Sniffer,
    Scene,
    Group,
}

impl LastDapcSourceView {
    pub const fn rest_name(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Sniffer => Some("sniffer"),
            Self::Scene => Some("scene"),
            Self::Group => Some("group"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusFlagsView {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub lamp_on: bool,
    pub limit_error: bool,
    pub fade_running: bool,
    pub reset_state: bool,
    pub missing_short_address: bool,
    pub power_cycle_seen: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureStatusView {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub communication_failure: bool,
    pub source: ObservationSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeErrorView {
    pub code: dali2rust_contracts::msg::ErrorCode,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalDeviceStateView {
    pub power: String,
    pub level: Option<u8>,
    pub color_mode: String,
    pub color_temperature_kelvin: Option<u16>,
    pub xy: Option<(f64, f64)>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
    pub status: Option<StatusFlagsView>,
    pub failure_status: Option<FailureStatusView>,
    pub value_source: Option<ObservationSource>,
    pub last_seen_ms: Option<u64>,
    pub last_dapc_source: LastDapcSourceView,
    pub error: Option<RuntimeErrorView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFlagsView {
    pub brightness: bool,
    pub cct: bool,
    pub xy: bool,
    pub rgb: bool,
    pub rgbwaf: bool,
    pub scenes: bool,
    pub groups: bool,
}

pub fn capability_supports_color_mode(caps: CapabilityFlagsView, mode: ColorMode) -> bool {
    match mode {
        ColorMode::Brightness => caps.brightness,
        ColorMode::Cct => caps.cct,
        ColorMode::Xy => caps.xy,
        ColorMode::Rgb => caps.rgb,
        ColorMode::Rgbwaf => caps.rgbwaf,
        _ => false,
    }
}

pub fn seed_capability_from_color_mode(caps: &mut CapabilityFlagsView, mode: ColorMode) {
    caps.cct |= mode == ColorMode::Cct;
    caps.xy |= mode == ColorMode::Xy;
    caps.rgb |= mode == ColorMode::Rgb;
    caps.rgb |= mode == ColorMode::Rgbwaf;
    caps.rgbwaf |= mode == ColorMode::Rgbwaf;
}

pub fn capability_accepts_color_mode(caps: CapabilityFlagsView, mode: ColorMode) -> bool {
    let known = caps.cct || caps.xy || caps.rgb || caps.rgbwaf;
    !known || capability_supports_color_mode(caps, mode)
}

#[cfg(test)]
mod capability_color_mode_tests;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBankRangeView {
    pub start: u16,
    pub length: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBankSummaryView {
    pub bank: u8,
    pub total_bytes_read: u16,
    pub last_read_ms: u64,
    pub ranges: Vec<MemoryBankRangeView>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AdapterView {
    pub adapter_id: u8,
    pub name: String,
    pub enabled: bool,
    pub commands: u64,
    pub timeouts: u64,
    pub errors: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalDeviceView {
    pub adapter_id: u8,
    pub short_address: u8,
    pub random_address: Option<u32>,
    pub name: String,
    pub notes: Option<String>,
    pub device_type_discovered: String,
    pub device_type_override: Option<String>,
    pub device_type_effective: String,
    pub device_type_source: String,
    pub supported_device_types: Option<Vec<u8>>,
    pub color_mode_discovered: String,
    pub color_mode_override: Option<String>,
    pub color_mode_effective: String,
    pub color_mode_source: String,
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub attributes: PhysicalDeviceAttributesView,
    pub energy: MemoryEnergyAttributesView,
    pub diagnostics: MemoryDiagnosticsAttributesView,
    pub bus_unit: MemoryBusUnitAttributesView,
    pub luminaire_info: Option<Box<MemoryLuminaireAttributesView>>,
    pub memory_banks: Vec<MemoryBankSummaryView>,
    pub state: PhysicalDeviceStateView,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
}

#[derive(Clone, Debug)]
pub struct PhysicalDeviceSummaryView {
    pub short_address: u8,
    pub random_address: Option<u32>,
    pub name: String,
    pub device_type_effective: &'static str,
    pub color_mode_effective: &'static str,
    pub capabilities: CapabilityFlagsView,
    pub state: PhysicalDeviceStateView,
    pub groups_membership: Option<u16>,
    pub gtin: Option<u64>,
    pub identification_number: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct PhysicalDeviceCoreView {
    pub adapter_id: u8,
    pub short_address: u8,
    pub random_address: Option<u32>,
    pub name: String,
    pub notes: Option<String>,
    pub device_type_discovered: &'static str,
    pub device_type_override: Option<&'static str>,
    pub device_type_effective: &'static str,
    pub device_type_source: &'static str,
    pub supported_device_types: Option<DeviceTypeSet>,
    pub color_mode_discovered: &'static str,
    pub color_mode_override: Option<&'static str>,
    pub color_mode_effective: &'static str,
    pub color_mode_source: &'static str,
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub state: PhysicalDeviceStateView,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
}

macro_rules! declare_attribute_sections {
    ( $( $variant:ident, $wire:literal, $view:ident );+ $(;)? ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum AttributeSectionKind {
            $( $variant, )+
        }

        impl AttributeSectionKind {
            pub const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            pub const fn wire_name(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire, )+
                }
            }

            pub fn from_wire_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|k| k.wire_name() == name)
            }
        }

        #[derive(Clone, Debug)]
        pub enum AttributeSectionView {
            $( $variant(Box<$view>), )+
        }

        impl AttributeSectionView {
            pub const fn kind(&self) -> AttributeSectionKind {
                match self {
                    $( Self::$variant(_) => AttributeSectionKind::$variant, )+
                }
            }
        }

        #[cfg(test)]
        pub(crate) const BOXED_SECTION_TEMPORARIES: &[(&str, usize)] = &[
            $( (stringify!($view), core::mem::size_of::<$view>()), )+
        ];
    };
}

declare_attribute_sections! {
    Common102, "common_102", Common102AttributesView;
    Dt6Led, "dt6_led", Dt6LedAttributesView;
    Dt8Color, "dt8_color", Dt8ColorAttributesView;
    Extended, "extended", ExtendedAttributesView;
    Groups, "groups", GroupsAttributesView;
    MemoryDiagnostics, "memory_diagnostics", MemoryDiagnosticsAttributesView;
    MemoryEnergy, "memory_energy", MemoryEnergyAttributesView;
    MemoryBusUnit, "memory_bus_unit", MemoryBusUnitAttributesView;
    MemoryIdentity, "memory_identity", MemoryIdentityAttributesView;
    MemoryLuminaire, "memory_luminaire", MemoryLuminaireAttributesView;
    MemoryProfile, "memory_profile", MemoryProfileAttributesView;
    Scenes, "scenes", ScenesAttributesView;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorTemperatureRangeView {
    pub min_kelvin: u16,
    pub max_kelvin: u16,
}

impl ColorTemperatureRangeView {
    pub fn from_mirek(coolest: Option<u16>, warmest: Option<u16>) -> Option<Self> {
        let (coolest, warmest) = (coolest?, warmest?);
        if coolest == 0 || warmest == 0 || coolest >= warmest {
            return None;
        }
        Some(Self {
            min_kelvin: mirek_to_kelvin(warmest)?,
            max_kelvin: mirek_to_kelvin(coolest)?,
        })
    }
}

pub fn mirek_to_kelvin(mirek: u16) -> Option<u16> {
    const MICRO: u32 = 1_000_000;
    if mirek == 0 {
        return None;
    }
    Some((MICRO / u32::from(mirek)).min(u32::from(u16::MAX)) as u16)
}

pub fn kelvin_to_mirek(kelvin: u16) -> Option<u16> {
    if kelvin == 0 {
        return None;
    }
    let mirek = (1_000_000u32 + (u32::from(kelvin) / 2)) / u32::from(kelvin);
    Some(mirek.min(u32::from(u16::MAX)) as u16)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VirtualLampView {
    pub adapter_id: u8,
    pub virtual_lamp_id: u8,
    pub name: String,
    pub device_type_effective: String,
    pub device_type_source: String,
    pub color_mode_effective: String,
    pub color_mode_source: String,
    pub binding_short: Option<u8>,
    pub ha_entity_enabled: bool,
    pub state: PhysicalDeviceStateView,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    pub adapter_id: u8,
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities_summary: CapabilityFlagsView,
    pub dirty: bool,
    pub member_count_desired: u8,
    pub member_count_applied: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMatrixGroupView {
    pub group_id: u8,
    pub name: String,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembershipMatrixRowView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub desired: [bool; 16],
    pub applied: [bool; 16],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembershipMatrixView {
    pub adapter_id: u8,
    pub groups: Vec<GroupMatrixGroupView>,
    pub rows: Vec<GroupMembershipMatrixRowView>,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneView {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
    pub row_count_included: u8,
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HclScheduleView {
    pub schedule_id: String,
    pub enabled: bool,
    pub algorithm: dali2rust_contracts::msg::HclAlgorithm,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: Vec<dali2rust_contracts::msg::HclTargetRow>,
    pub points: Vec<dali2rust_contracts::msg::HclSchedulePointRow>,
}

pub trait HclScheduleReadPort: Send + Sync {
    fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView>;
    fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView>;

    fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool {
        self.hcl_schedule_view(schedule_id).is_some()
    }
}

pub trait HclSchedulerReadPort: HclScheduleReadPort + GroupReadPort {}
impl<T: HclScheduleReadPort + GroupReadPort> HclSchedulerReadPort for T {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HclOverrideTargetView {
    pub adapter_id: u8,
    pub scope: dali2rust_contracts::msg::HclTargetScope,
    pub group_id: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HclOverrideView {
    pub suspended: bool,
    pub targets: Vec<HclOverrideTargetView>,
    pub since_local_minutes: Option<u16>,
}

pub trait HclOverrideReadPort: Send + Sync {
    fn hcl_override_view(&self, schedule_id: &str) -> HclOverrideView;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollerSettingsView {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_mask: u8,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
}

pub trait PollerSettingsReadPort: Send + Sync {
    fn poller_settings_view(&self) -> PollerSettingsView;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaliSettingsView {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationActiveMover {
    #[default]
    Unmoved,
    Command,
    Wire,
}

pub trait DaliSettingsReadPort: Send + Sync {
    fn dali_settings_view(&self) -> DaliSettingsView;

    fn application_active_moved_by(&self) -> (bool, ApplicationActiveMover) {
        (
            self.dali_settings_view().application_active,
            ApplicationActiveMover::Unmoved,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedundancySettingsView {
    pub enabled: bool,
    pub standby_role: bool,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

pub trait RedundancySettingsReadPort: Send + Sync {
    fn redundancy_settings_view(&self) -> RedundancySettingsView;
}

pub trait RedundancySettingsApplyWatchPort: Send + Sync {
    fn redundancy_settings_applied_load(&self) -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PoliciesView {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
}

impl PoliciesView {
    #[must_use]
    pub fn manages_anything(&self) -> bool {
        self.system_failure_level.is_some() || self.power_on_level.is_some()
    }
}

pub trait PoliciesReadPort: Send + Sync {
    fn policies_view(&self) -> PoliciesView;
}

pub trait PoliciesApplyWatchPort: Send + Sync {
    fn policies_applied_load(&self) -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollTargetView {
    pub short_address: u8,
    pub is_dt8: bool,
    pub is_dt6: bool,
    pub declares_energy: bool,
    pub declares_diagnostics: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollTargetSelection {
    pub targets: Vec<PollTargetView>,
    pub excluded_unbound: u16,
}

pub trait PollTargetReadPort: Send + Sync {
    fn list_poll_targets(&self, adapter_id: u8) -> PollTargetSelection;
}

pub trait PollerReadPort: PollerSettingsReadPort + PollTargetReadPort {}
impl<T: PollerSettingsReadPort + PollTargetReadPort> PollerReadPort for T {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaInputInstanceView {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaInputView {
    pub short_address: u8,
    pub name: Option<String>,
    pub ha_expose: bool,
    pub instances: Vec<HaInputInstanceView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaLampView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
    pub unreachable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HaGroupStateView {
    pub commanded: bool,
    pub any_on: bool,
    pub brightness: Option<u8>,
    pub color_mode: Option<ColorMode>,
    pub color_temperature_kelvin: Option<u16>,
    pub rgb: Option<(u8, u8, u8)>,
    pub xy: Option<(f64, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HaGroupView {
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities: CapabilityFlagsView,
    pub state: HaGroupStateView,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaSceneView {
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeAssistantSettingsView {
    pub enabled: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_username: String,
    pub broker_password_set: bool,
    pub discovery_prefix: String,
    pub state_topic_prefix: String,
    pub controller_id: String,
    pub publish_qos: u8,
    pub retain_state: bool,
    pub retain_discovery: bool,
    pub expose_input_devices: bool,
}

impl HomeAssistantSettingsView {
    pub fn broker_url_view(&self) -> String {
        if self.broker_host.is_empty() {
            String::new()
        } else {
            format!("mqtt://{}:{}", self.broker_host, self.broker_port)
        }
    }
}

pub trait HomeAssistantSettingsReadPort: Send + Sync {
    fn home_assistant_settings_view(&self) -> HomeAssistantSettingsView;
}

pub trait HaPublishReadPort: Send + Sync {
    fn ha_lamp_views(&self, adapter_id: u8) -> Vec<HaLampView>;
    fn ha_input_views(&self, adapter_id: u8) -> Vec<HaInputView>;
    fn ha_input_view(&self, adapter_id: u8, short_address: u8) -> Option<HaInputView>;
    fn ha_group_views(&self, adapter_id: u8) -> Vec<HaGroupView>;
    fn ha_scene_views(&self, adapter_id: u8) -> Vec<HaSceneView>;
    fn ha_lamp_view(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<HaLampView>;
    fn ha_group_view(&self, adapter_id: u8, group_id: u8) -> Option<HaGroupView>;
    fn ha_lamp_ids_for_short(&self, adapter_id: u8, short_address: u8) -> Vec<u8>;
    fn ha_retracted_lamp_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn ha_retracted_group_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn ha_active_scene(&self, adapter_id: u8) -> Option<u8>;
}

pub trait HomeAssistantSecretReadPort: Send + Sync {
    fn home_assistant_broker_password(&self) -> String;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRowStateView {
    pub included: bool,
    pub power: Option<String>,
    pub level: Option<u8>,
    pub color_mode: Option<String>,
    pub color_temperature_kelvin: Option<u16>,
    pub xy: Option<(u16, u16)>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMatrixRowView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub capabilities: CapabilityFlagsView,
    pub desired: SceneRowStateView,
    pub applied: SceneRowStateView,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMatrixView {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub rows: Vec<SceneMatrixRowView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneApplyRowView {
    pub virtual_lamp_id: u8,
    pub desired_included: bool,
    pub desired_target: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub applied_included: bool,
    pub applied_target: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub binding_short: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneApplySnapshot {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub rows: Vec<SceneApplyRowView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneApplyDiffRow {
    pub virtual_lamp_id: u8,
    pub scene_id: u8,
    pub action: dali2rust_contracts::msg::SceneProgramAction,
    pub target_state: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub binding_short: Option<u8>,
}

pub fn collect_scene_apply_diff(snapshot: &SceneApplySnapshot) -> Vec<SceneApplyDiffRow> {
    use dali2rust_contracts::msg::SceneProgramAction;
    let mut diff = Vec::new();
    for row in &snapshot.rows {
        let action = match (row.desired_included, row.applied_included) {
            (true, false) => SceneProgramAction::Write,
            (true, true) if row.desired_target != row.applied_target => SceneProgramAction::Update,
            (false, true) => SceneProgramAction::Clear,
            _ => continue,
        };
        diff.push(SceneApplyDiffRow {
            virtual_lamp_id: row.virtual_lamp_id,
            scene_id: snapshot.scene_id,
            action,
            target_state: (action != SceneProgramAction::Clear)
                .then_some(row.desired_target)
                .flatten(),
            binding_short: row.binding_short,
        });
    }
    insertion_sort_by(&mut diff, |a, b| a.virtual_lamp_id > b.virtual_lamp_id);
    diff
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupApplyRowView {
    pub virtual_lamp_id: u8,
    pub desired_groups_mask: u16,
    pub applied_groups_mask: u16,
    pub binding_short: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupApplySnapshot {
    pub adapter_id: u8,
    pub rows: Vec<GroupApplyRowView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupApplyDiffCell {
    pub virtual_lamp_id: u8,
    pub group_id: u8,
    pub action: dali2rust_contracts::msg::GroupMembershipAction,
    pub binding_short: Option<u8>,
}

pub fn collect_group_apply_diff(snapshot: &GroupApplySnapshot) -> Vec<GroupApplyDiffCell> {
    use dali2rust_contracts::msg::GroupMembershipAction;
    const GROUP_COUNT: u8 = 16;
    let mut diff = Vec::new();
    for row in &snapshot.rows {
        let changed = row.desired_groups_mask ^ row.applied_groups_mask;
        for group_id in 0..GROUP_COUNT {
            let bit = 1u16 << group_id;
            if changed & bit == 0 {
                continue;
            }
            diff.push(GroupApplyDiffCell {
                virtual_lamp_id: row.virtual_lamp_id,
                group_id,
                action: if row.desired_groups_mask & bit != 0 {
                    GroupMembershipAction::Add
                } else {
                    GroupMembershipAction::Remove
                },
                binding_short: row.binding_short,
            });
        }
    }
    insertion_sort_by(&mut diff, |a, b| {
        (a.virtual_lamp_id, a.group_id) > (b.virtual_lamp_id, b.group_id)
    });
    diff
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationErrorView {
    pub code: Cow<'static, str>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationGroupApplyOutcomeView {
    pub virtual_lamp_id: u8,
    pub group_id: u8,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_short_address: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationGroupApplyResultView {
    pub programmed: Vec<OperationGroupApplyOutcomeView>,
    pub skipped: Vec<OperationGroupApplyOutcomeView>,
    pub failed: Vec<OperationGroupApplyOutcomeView>,
    #[serde(default)]
    pub programmed_total: u16,
    #[serde(default)]
    pub skipped_total: u16,
    #[serde(default)]
    pub failed_total: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAttributeReadOutcomesView {
    pub identity: String,
    pub runtime_status: String,
    pub common_102: String,
    pub dt8_color: String,
    pub dt6_led: String,
    pub groups: String,
    pub scenes: String,
    pub extended: String,
    pub memory_banks: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSceneApplyOutcomeView {
    pub virtual_lamp_id: u8,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_short_address: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSceneApplyResultView {
    pub written: Vec<OperationSceneApplyOutcomeView>,
    pub updated: Vec<OperationSceneApplyOutcomeView>,
    pub cleared: Vec<OperationSceneApplyOutcomeView>,
    pub skipped: Vec<OperationSceneApplyOutcomeView>,
    pub failed: Vec<OperationSceneApplyOutcomeView>,
    #[serde(default)]
    pub written_total: u16,
    #[serde(default)]
    pub updated_total: u16,
    #[serde(default)]
    pub cleared_total: u16,
    #[serde(default)]
    pub skipped_total: u16,
    #[serde(default)]
    pub failed_total: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OperationApplyResultView {
    GroupApply(OperationGroupApplyResultView),
    SceneApply(OperationSceneApplyResultView),
    Identify(OperationIdentifyResultView),
    AddressChange(OperationAddressChangeResultView),
    ReplaceDevice(OperationReplaceDeviceResultView),
    HaDiscoveryPublish(OperationHaDiscoveryResultView),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationHaDiscoveryResultView {
    pub entities_published: u16,
    pub entities_failed: u16,
}

impl OperationApplyResultView {
    pub fn as_ha_discovery_publish(&self) -> Option<&OperationHaDiscoveryResultView> {
        match self {
            Self::HaDiscoveryPublish(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_group_apply(&self) -> Option<&OperationGroupApplyResultView> {
        match self {
            Self::GroupApply(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_scene_apply(&self) -> Option<&OperationSceneApplyResultView> {
        match self {
            Self::SceneApply(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_identify(&self) -> Option<&OperationIdentifyResultView> {
        match self {
            Self::Identify(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_address_change(&self) -> Option<&OperationAddressChangeResultView> {
        match self {
            Self::AddressChange(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_replace_device(&self) -> Option<&OperationReplaceDeviceResultView> {
        match self {
            Self::ReplaceDevice(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationIdentifyResultView {
    pub short_address: u8,
    pub identify_mechanism: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAddressChangeResultView {
    pub old_short_address: u8,
    pub new_short_address: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRestoredSlicesView {
    pub metadata_and_overrides: bool,
    pub attributes: bool,
    pub groups: bool,
    pub scenes: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationReplaceDeviceResultView {
    pub failed_short_address: u8,
    pub replacement_short_address: u8,
    pub restored: OperationRestoredSlicesView,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationView {
    pub operation_id: String,
    #[serde(rename = "type")]
    pub operation_type: Cow<'static, str>,
    pub status: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationErrorView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<OperationApplyResultView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute_read_outcomes: Option<OperationAttributeReadOutcomesView>,
}

pub trait RegistryReadPort: Send + Sync {
    fn application_controller_active(&self) -> bool;
    fn virtual_lamp_snapshot(&self, adapter_id: u8, virtual_lamp_id: u8) -> VirtualLampSnapshot;
    fn virtual_lamp_binding_short(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<u8>;
    fn adapter_snapshot(&self, adapter_id: u8) -> AdapterSnapshot;
    fn known_physical_short_addresses(&self, adapter_id: u8) -> Vec<u8>;
    fn first_free_short_address(&self, adapter_id: u8) -> Option<u8>;
    fn physical_dt8_gear_features(&self, adapter_id: u8, short_address: u8) -> Option<u8>;
    fn physical_dt8_auto_activation_repair_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool;
    fn apply_on_discovery_armed(&self, adapter_id: u8) -> bool;
    fn physical_dt8_rgbwaf_control_assert_allowed(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> bool;
}

pub trait AdapterReadPort: Send + Sync {
    fn adapter_count(&self) -> u8;
    fn adapter_view(&self, adapter_id: u8) -> Option<AdapterView>;
    fn list_adapter_views(&self) -> Vec<AdapterView>;
}

pub trait PhysicalDeviceReadPort: AdapterReadPort {
    fn physical_device_summary_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceSummaryView>;

    fn physical_device_core_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceCoreView>;

    fn physical_device_attribute_section(
        &self,
        adapter_id: u8,
        short_address: u8,
        kind: AttributeSectionKind,
    ) -> Option<AttributeSectionView>;

    fn physical_device_memory_banks(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<Vec<MemoryBankSummaryView>>;

    fn physical_device_capabilities(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<CapabilityFlagsView>;

    fn physical_device_supported_types(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<DeviceTypeSet>;

    fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool;

    fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8>;
}

pub trait VirtualLampReadPort: PhysicalDeviceReadPort {
    fn virtual_lamp_view(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampView;
    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8>;
    fn list_virtual_lamp_views(&self, adapter_id: u8) -> Vec<VirtualLampView>;
    fn physical_short_on_other_adapter(&self, adapter_id: u8, short_address: u8) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualLampCapabilityView {
    pub virtual_lamp_id: u8,
    pub binding_short: Option<u8>,
    pub capabilities: CapabilityFlagsView,
}

pub trait VirtualLampCapabilityReadPort: Send + Sync {
    fn virtual_lamp_capability_view(
        &self,
        adapter_id: u8,
        lamp_id: u8,
    ) -> VirtualLampCapabilityView;
    fn list_virtual_lamp_capability_views(&self, adapter_id: u8)
        -> Vec<VirtualLampCapabilityView>;
}

pub trait GroupReadPort: AdapterReadPort {
    fn group_view(&self, adapter_id: u8, group_id: u8) -> Option<GroupView>;
    fn list_group_views(&self, adapter_id: u8) -> Vec<GroupView>;
    fn group_membership_matrix_view(&self, adapter_id: u8) -> Option<GroupMembershipMatrixView>;
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot>;

    fn applied_group_member_mask(&self, adapter_id: u8, group_id: u8) -> Option<u64> {
        if group_id >= GROUP_COUNT {
            return None;
        }
        let bit = 1u16 << group_id;
        let snapshot = self.group_apply_snapshot(adapter_id)?;
        Some(snapshot.rows.iter().fold(0u64, |mask, row| {
            if row.virtual_lamp_id < VIRTUAL_LAMP_COUNT && row.applied_groups_mask & bit != 0 {
                mask | (1u64 << row.virtual_lamp_id)
            } else {
                mask
            }
        }))
    }
}

pub trait OperationReadPort: Send + Sync {
    fn operation_status(&self, operation_id: &str) -> Option<Cow<'static, str>>;
    fn operation_view(&self, operation_id: &str) -> Option<OperationView>;
    fn list_operation_keys(&self) -> Vec<String>;
    fn has_active_operation(&self, operation_type: OperationType, adapter_id: u8) -> bool;
}

pub trait AdapterApplyWatchPort: Send + Sync {
    fn adapter_settings_applied_load(&self) -> u32;
}

pub trait PollerSettingsApplyWatchPort: Send + Sync {
    fn poller_settings_applied_load(&self) -> u32;
}

pub trait DaliSettingsApplyWatchPort: Send + Sync {
    fn dali_settings_applied_load(&self) -> u32;
}

pub trait HomeAssistantSettingsApplyWatchPort: Send + Sync {
    fn home_assistant_settings_applied_load(&self) -> u32;
}

pub trait PhysicalDeviceApplyWatchPort: Send + Sync {
    fn physical_override_applied_load(&self) -> u32;
}

pub trait VirtualLampMetadataWatchPort: Send + Sync {
    fn virtual_lamp_metadata_applied_load(&self) -> u32;
}

pub trait VirtualLampBindingWatchPort: Send + Sync {
    fn virtual_lamp_binding_applied_load(&self) -> u32;
}

pub trait GroupMetadataWatchPort: Send + Sync {
    fn group_metadata_applied_load(&self) -> u32;
}

pub trait SceneReadPort: AdapterReadPort {
    fn scene_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneView>;
    fn list_scene_views(&self, adapter_id: u8) -> Vec<SceneView>;
    fn scene_matrix_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixView>;
    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot>;
}

pub trait ApplyReadPort: GroupReadPort + SceneReadPort + PolicyApplyReadPort {}
impl<T: GroupReadPort + SceneReadPort + PolicyApplyReadPort> ApplyReadPort for T {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyApplyCell {
    pub short_address: u8,
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
}

pub trait PolicyApplyReadPort: Send + Sync {
    fn policy_apply_targets(&self, adapter_id: u8) -> Option<Vec<PolicyApplyCell>>;
}

pub trait ProjectorReadPort:
    VirtualLampCapabilityReadPort + GroupReadPort + SceneReadPort
{
}
impl<T: VirtualLampCapabilityReadPort + GroupReadPort + SceneReadPort> ProjectorReadPort for T {}

pub trait InputInstanceTypeReadPort: Send + Sync {
    fn input_instance_type(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> Option<u8>;
}

pub trait SceneMetadataWatchPort: Send + Sync {
    fn scene_metadata_applied_load(&self) -> u32;
}

#[cfg(test)]
mod scene_apply_diff_tests;

#[cfg(test)]
mod group_apply_diff_tests;

#[cfg(test)]
mod view_size_tests;

#[cfg(test)]
mod product_limit_tests {
    use super::*;

    #[test]
    fn the_product_lamp_limit_and_the_wire_address_space_are_separate_facts() {
        assert_eq!(usize::from(VIRTUAL_LAMP_COUNT), MAX_SHORT_ADDRESSES);
        assert!(u32::from(GROUP_COUNT) <= u16::BITS);
    }

    #[test]
    fn groups_and_scenes_are_each_four_bits_wide() {
        assert_eq!(GROUP_COUNT, 16);
        assert_eq!(SCENE_COUNT, 16);
    }

    #[test]
    fn a_lamp_id_is_below_the_count_not_below_the_type() {
        assert!(VIRTUAL_LAMP_COUNT < u8::MAX, "MAX would be a valid id");
        assert!(
            (0..VIRTUAL_LAMP_COUNT).all(|id| usize::from(id) < MAX_SHORT_ADDRESSES),
            "every lamp row must be able to name a short address"
        );
    }

    #[test]
    fn both_conversions_saturate_rather_than_truncate() {
        assert_eq!(kelvin_to_mirek(15), Some(u16::MAX));
        assert_eq!(kelvin_to_mirek(1), Some(u16::MAX));
        assert_eq!(mirek_to_kelvin(15), Some(u16::MAX));
        assert_eq!(mirek_to_kelvin(1), Some(u16::MAX));
        assert_eq!(kelvin_to_mirek(0), None);
        assert_eq!(mirek_to_kelvin(0), None);
    }

    #[test]
    fn the_admitted_kelvin_band_round_trips() {
        for kelvin in [1_000u16, 2_700, 4_000, 6_500, 20_000] {
            let mirek = kelvin_to_mirek(kelvin).expect("a real temperature");
            let back = mirek_to_kelvin(mirek).expect("a real mirek");
            let drift = i32::from(back) - i32::from(kelvin);
            assert!(
                drift.abs() <= i32::from(kelvin) / 100,
                "{kelvin} K -> {mirek} mirek -> {back} K drifted {drift}"
            );
        }
    }
}

