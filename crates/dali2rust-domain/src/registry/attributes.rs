use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use serde::{Deserialize, Serialize};
use dali2rust_contracts::msg::{FixedText32, FixedText64};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributeSource {
    #[default]
    Readback,
    WriteConfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValue<T> {
    pub value: T,
    pub source: AttributeSource,
    pub last_read_ms: Option<u64>,
    pub last_write_confirmed_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupsAttributesView {
    pub membership: Option<ObservedValue<u16>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenesAttributesView {
    pub levels: [Option<ObservedValue<u8>>; 16],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dt8ColorAttributesView {
    pub color_type: Option<ObservedValue<u8>>,
    pub color_value_0: Option<ObservedValue<u16>>,
    pub color_value_1: Option<ObservedValue<u16>>,
    pub color_value_2: Option<ObservedValue<u16>>,
    pub gear_features: Option<ObservedValue<u8>>,
    pub rgbwaf_control: Option<ObservedValue<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedAttributesView {
    pub fade_time_ms: Option<ObservedValue<u16>>,
    pub version_number: Option<ObservedValue<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBusUnitAttributesView {
    pub configuration: Option<ObservedValue<u8>>,
    pub implemented_parts: Option<ObservedValue<u8>>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyBankView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub energy_scale: Option<ObservedValue<i8>>,
    pub energy: Option<ObservedValue<BankReading>>,
    pub power_scale: Option<ObservedValue<i8>>,
    pub power: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEnergyAttributesView {
    pub active: EnergyBankView,
    pub apparent: EnergyBankView,
    pub loadside: EnergyBankView,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConditionView {
    pub flag: Option<ObservedValue<BankReading>>,
    pub counter: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuminaireMaintenanceView {
    pub bank_version: Option<ObservedValue<u8>>,
    pub rated_life_kilohours: Option<ObservedValue<BankReading>>,
    pub reference_temperature_offset60: Option<ObservedValue<BankReading>>,
    pub rated_starts_hundreds: Option<ObservedValue<BankReading>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
