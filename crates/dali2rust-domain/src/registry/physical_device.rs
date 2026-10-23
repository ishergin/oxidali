use serde::{Deserialize, Serialize};
use dali2rust_contracts::msg::{
    ColorMode, DeviceType, DeviceTypeSet, ExtendedVersionEntry, MAX_EXTENDED_VERSIONS,
};

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

pub fn device_type_fits_declared(declared: Option<DeviceTypeSet>, device_type: DeviceType) -> bool {
    match (declared, device_type.dali_code()) {
        (Some(declared), Some(code)) => declared.contains(code),
        _ => true,
    }
}

pub fn known_device_types(
    declared: Option<DeviceTypeSet>,
    override_type: Option<DeviceType>,
) -> Option<DeviceTypeSet> {
    declared.or_else(|| {
        override_type
            .and_then(DeviceType::dali_code)
            .map(|code| [code].into_iter().collect())
    })
}

pub fn colour_mode_fits_declared(declared: Option<DeviceTypeSet>, mode: ColorMode) -> bool {
    !mode.needs_dt8() || device_type_fits_declared(declared, DeviceType::Dt8Color)
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
    pub extended_versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS],
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
