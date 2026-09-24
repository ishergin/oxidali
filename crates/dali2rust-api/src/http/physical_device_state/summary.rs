use dali2rust_domain::registry::PhysicalDeviceSummaryView;
use serde::Serialize;

use super::runtime_state::{
    capabilities_view_to_dto, state_view_to_dto, CapabilityFlagsDto, PhysicalDeviceStateDto,
};

#[derive(Clone, Debug, Serialize)]
pub struct PhysicalDeviceSummaryDto {
    pub short_address: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub random_address: Option<u32>,
    pub name: String,
    pub device_type_effective: &'static str,
    pub color_mode_effective: &'static str,
    pub capabilities: CapabilityFlagsDto,
    pub state: PhysicalDeviceStateDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups_membership: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtin: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identification_number: Option<u64>,
}

pub(crate) fn summary_view_to_dto(view: PhysicalDeviceSummaryView) -> PhysicalDeviceSummaryDto {
    PhysicalDeviceSummaryDto {
        short_address: view.short_address,
        random_address: view.random_address,
        name: view.name,
        device_type_effective: view.device_type_effective,
        color_mode_effective: view.color_mode_effective,
        capabilities: capabilities_view_to_dto(&view.capabilities),
        state: state_view_to_dto(view.state),
        groups_membership: view.groups_membership,
        gtin: view.gtin,
        identification_number: view.identification_number,
    }
}
