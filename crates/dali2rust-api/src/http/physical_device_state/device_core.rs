use dali2rust_domain::registry::PhysicalDeviceCoreView;
use serde::Serialize;

use super::runtime_state::{
    capabilities_view_to_dto, state_view_to_dto, CapabilityFlagsDto, PhysicalDeviceStateDto,
};

#[derive(Clone, Debug, Serialize)]
pub struct PhysicalDeviceCoreDto {
    pub adapter_id: u8,
    pub short_address: u8,
    pub now_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub random_address: Option<u32>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub device_type_discovered: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type_override: Option<&'static str>,
    pub device_type_effective: &'static str,
    pub device_type_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_device_types: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extended_versions: Vec<ExtendedVersionDto>,
    pub color_mode_discovered: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_mode_override: Option<&'static str>,
    pub color_mode_effective: &'static str,
    pub color_mode_source: &'static str,
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub state: PhysicalDeviceStateDto,
    pub capabilities: CapabilityFlagsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_temperature_range: Option<ColorTemperatureRangeDto>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ExtendedVersionDto {
    pub device_type: u8,
    pub version_number: Option<u8>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ColorTemperatureRangeDto {
    pub min_kelvin: u16,
    pub max_kelvin: u16,
}

pub(crate) fn core_view_to_dto(view: PhysicalDeviceCoreView) -> PhysicalDeviceCoreDto {
    PhysicalDeviceCoreDto {
        adapter_id: view.adapter_id,
        short_address: view.short_address,
        now_ms: 0,
        random_address: view.random_address,
        name: view.name,
        notes: view.notes,
        device_type_discovered: view.device_type_discovered,
        device_type_override: view.device_type_override,
        device_type_effective: view.device_type_effective,
        device_type_source: view.device_type_source,
        supported_device_types: view.supported_device_types.map(|set| set.iter().collect()),
        extended_versions: view
            .extended_versions
            .iter()
            .flatten()
            .map(|e| ExtendedVersionDto {
                device_type: e.device_type,
                version_number: e.version_number,
            })
            .collect(),
        color_mode_discovered: view.color_mode_discovered,
        color_mode_override: view.color_mode_override,
        color_mode_effective: view.color_mode_effective,
        color_mode_source: view.color_mode_source,
        dt8_auto_activation_repair: view.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: view.dt8_rgbwaf_control_assert,
        state: state_view_to_dto(view.state),
        capabilities: capabilities_view_to_dto(&view.capabilities),
        color_temperature_range: view.color_temperature_range.map(|r| ColorTemperatureRangeDto {
            min_kelvin: r.min_kelvin,
            max_kelvin: r.max_kelvin,
        }),
    }
}
