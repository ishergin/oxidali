use std::sync::Arc;

use super::physical_device_state::{
    capabilities_view_to_dto, state_view_to_dto, CapabilityFlagsDto, ColorTemperatureRangeDto,
    PhysicalDeviceHttpState,
    PhysicalDeviceStateDto,
};
use dali2rust_domain::registry::{
    VirtualLampBindingWatchPort, VirtualLampMetadataWatchPort, VirtualLampReadPort, VirtualLampView,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct VirtualLampBindingDto {
    pub physical_short_address: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct VirtualLampDto {
    pub adapter_id: u8,
    pub virtual_lamp_id: u8,
    pub name: String,
    pub device_type_effective: String,
    pub device_type_source: String,
    pub color_mode_effective: String,
    pub color_mode_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<VirtualLampBindingDto>,
    pub ha_entity_enabled: bool,
    pub state: PhysicalDeviceStateDto,
    pub capabilities: CapabilityFlagsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_temperature_range: Option<ColorTemperatureRangeDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VirtualLampsListBody {
    pub adapter_id: u8,
    pub virtual_lamps: Vec<VirtualLampDto>,
}

pub fn virtual_lamp_view_to_dto(view: VirtualLampView) -> VirtualLampDto {
    VirtualLampDto {
        adapter_id: view.adapter_id,
        virtual_lamp_id: view.virtual_lamp_id,
        name: view.name,
        device_type_effective: view.device_type_effective,
        device_type_source: view.device_type_source,
        color_mode_effective: view.color_mode_effective,
        color_mode_source: view.color_mode_source,
        binding: view
            .binding_short
            .map(|physical_short_address| VirtualLampBindingDto {
                physical_short_address,
            }),
        ha_entity_enabled: view.ha_entity_enabled,
        state: state_view_to_dto(view.state),
        capabilities: capabilities_view_to_dto(&view.capabilities),
        color_temperature_range: view.color_temperature_range.map(|r| ColorTemperatureRangeDto {
            min_kelvin: r.min_kelvin,
            max_kelvin: r.max_kelvin,
        }),
    }
}

pub trait VirtualLampPatchWatch: Send + Sync {
    fn virtual_lamp_metadata_applied_load(&self) -> u32;
}

pub struct VirtualLampPatchWatchBridge {
    port: Arc<dyn VirtualLampMetadataWatchPort>,
}

impl VirtualLampPatchWatchBridge {
    pub fn new(port: Arc<dyn VirtualLampMetadataWatchPort>) -> Self {
        Self { port }
    }
}

impl VirtualLampPatchWatch for VirtualLampPatchWatchBridge {
    fn virtual_lamp_metadata_applied_load(&self) -> u32 {
        self.port.virtual_lamp_metadata_applied_load()
    }
}

pub trait VirtualLampBindingApplyWatch: Send + Sync {
    fn virtual_lamp_binding_applied_load(&self) -> u32;
}

pub struct VirtualLampBindingApplyWatchBridge {
    port: Arc<dyn VirtualLampBindingWatchPort>,
}

impl VirtualLampBindingApplyWatchBridge {
    pub fn new(port: Arc<dyn VirtualLampBindingWatchPort>) -> Self {
        Self { port }
    }
}

impl VirtualLampBindingApplyWatch for VirtualLampBindingApplyWatchBridge {
    fn virtual_lamp_binding_applied_load(&self) -> u32 {
        self.port.virtual_lamp_binding_applied_load()
    }
}

pub trait VirtualLampHttpState: PhysicalDeviceHttpState {
    fn virtual_lamp_dto(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampDto;

    fn virtual_lamp_binding_short(&self, adapter_id: u8, lamp_id: u8) -> Option<u8> {
        self.virtual_lamp_dto(adapter_id, lamp_id)
            .binding
            .map(|b| b.physical_short_address)
    }

    fn physical_short_on_other_adapter(&self, _adapter_id: u8, _short_address: u8) -> bool {
        false
    }

    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        self.list_virtual_lamp_dtos(adapter_id)
            .into_iter()
            .map(|d| d.virtual_lamp_id)
            .collect()
    }

    fn list_virtual_lamp_dtos(&self, adapter_id: u8) -> Vec<VirtualLampDto> {
        (0..64)
            .map(|id| self.virtual_lamp_dto(adapter_id, id))
            .collect()
    }
}

pub struct VirtualLampHttpStateBridge {
    port: Arc<dyn VirtualLampReadPort>,
}

impl VirtualLampHttpStateBridge {
    pub fn new(port: Arc<dyn VirtualLampReadPort>) -> Self {
        Self { port }
    }
}

super::physical_device_state::impl_physical_device_http_state!(VirtualLampHttpStateBridge);

impl VirtualLampHttpState for VirtualLampHttpStateBridge {
    fn virtual_lamp_dto(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampDto {
        let view = self.port.virtual_lamp_view(adapter_id, lamp_id);
        virtual_lamp_view_to_dto(view)
    }

    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        self.port.list_virtual_lamp_ids(adapter_id)
    }

    fn list_virtual_lamp_dtos(&self, adapter_id: u8) -> Vec<VirtualLampDto> {
        self.port
            .list_virtual_lamp_views(adapter_id)
            .into_iter()
            .map(virtual_lamp_view_to_dto)
            .collect()
    }

    fn physical_short_on_other_adapter(&self, adapter_id: u8, short_address: u8) -> bool {
        self.port
            .physical_short_on_other_adapter(adapter_id, short_address)
    }
}
