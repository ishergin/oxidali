use std::sync::Arc;

use dali2rust_domain::registry::{
    AttributeSectionKind, AttributeSectionView, PhysicalDeviceApplyWatchPort,
    PhysicalDeviceReadPort,
};

use super::banks::MemoryBankSummaryDto;
use super::device_core::PhysicalDeviceCoreDto;
use super::runtime_state::CapabilityFlagsDto;
use super::summary::PhysicalDeviceSummaryDto;

pub trait PhysicalDevicePatchWatch: Send + Sync {
    fn physical_override_applied_load(&self) -> u32;
}

pub struct PhysicalDevicePatchWatchBridge {
    port: Arc<dyn PhysicalDeviceApplyWatchPort>,
}

impl PhysicalDevicePatchWatchBridge {
    pub fn new(port: Arc<dyn PhysicalDeviceApplyWatchPort>) -> Self {
        Self { port }
    }
}

impl PhysicalDevicePatchWatch for PhysicalDevicePatchWatchBridge {
    fn physical_override_applied_load(&self) -> u32 {
        self.port.physical_override_applied_load()
    }
}

pub trait PhysicalDeviceHttpState: Send + Sync {
    fn adapter_count(&self) -> u8;

    fn physical_device_summary_dto(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceSummaryDto>;

    fn physical_device_core_dto(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceCoreDto>;

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
    ) -> Option<Vec<MemoryBankSummaryDto>>;

    fn physical_device_capabilities(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<CapabilityFlagsDto>;

    fn physical_device_supported_types(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<Vec<u8>>;

    fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool;

    fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8>;
}

pub struct PhysicalDeviceHttpStateBridge {
    port: Arc<dyn PhysicalDeviceReadPort>,
}

impl PhysicalDeviceHttpStateBridge {
    pub fn new(port: Arc<dyn PhysicalDeviceReadPort>) -> Self {
        Self { port }
    }
}

macro_rules! impl_physical_device_http_state {
    ($t:ty) => {
        impl $crate::http::physical_device_state::PhysicalDeviceHttpState for $t {
            fn adapter_count(&self) -> u8 {
                self.port.adapter_count()
            }

            fn physical_device_summary_dto(
                &self,
                adapter_id: u8,
                short_address: u8,
            ) -> Option<$crate::http::physical_device_state::PhysicalDeviceSummaryDto> {
                self.port
                    .physical_device_summary_view(adapter_id, short_address)
                    .map($crate::http::physical_device_state::summary_view_to_dto)
            }

            fn physical_device_core_dto(
                &self,
                adapter_id: u8,
                short_address: u8,
            ) -> Option<$crate::http::physical_device_state::PhysicalDeviceCoreDto> {
                self.port
                    .physical_device_core_view(adapter_id, short_address)
                    .map($crate::http::physical_device_state::core_view_to_dto)
            }

            fn physical_device_attribute_section(
                &self,
                adapter_id: u8,
                short_address: u8,
                kind: dali2rust_domain::registry::AttributeSectionKind,
            ) -> Option<dali2rust_domain::registry::AttributeSectionView> {
                self.port
                    .physical_device_attribute_section(adapter_id, short_address, kind)
            }

            fn physical_device_memory_banks(
                &self,
                adapter_id: u8,
                short_address: u8,
            ) -> Option<Vec<$crate::http::physical_device_state::MemoryBankSummaryDto>> {
                self.port
                    .physical_device_memory_banks(adapter_id, short_address)
                    .map(|banks| {
                        banks
                            .iter()
                            .map($crate::http::physical_device_state::memory_bank_view_to_dto)
                            .collect()
                    })
            }

            fn physical_device_capabilities(
                &self,
                adapter_id: u8,
                short_address: u8,
            ) -> Option<$crate::http::physical_device_state::CapabilityFlagsDto> {
                self.port
                    .physical_device_capabilities(adapter_id, short_address)
                    .map(|caps| {
                        $crate::http::physical_device_state::capabilities_view_to_dto(&caps)
                    })
            }

            fn physical_device_supported_types(
                &self,
                adapter_id: u8,
                short_address: u8,
            ) -> Option<Vec<u8>> {
                self.port
                    .physical_device_supported_types(adapter_id, short_address)
                    .map(|set| set.iter().collect())
            }

            fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool {
                self.port.physical_device_exists(adapter_id, short_address)
            }

            fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8> {
                self.port.list_physical_device_short_addresses(adapter_id)
            }
        }
    };
}
pub(crate) use impl_physical_device_http_state;

impl_physical_device_http_state!(PhysicalDeviceHttpStateBridge);
