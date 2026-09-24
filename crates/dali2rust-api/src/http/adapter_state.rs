use std::sync::Arc;

use dali2rust_domain::registry::{AdapterApplyWatchPort, AdapterReadPort, AdapterView};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct AdapterLimitsDto {
    pub virtual_lamps: u8,
    pub groups: u8,
    pub scenes: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdapterCountersDto {
    pub commands: u64,
    pub timeouts: u64,
    pub errors: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdapterDto {
    pub adapter_id: u8,
    pub name: String,
    pub enabled: bool,
    pub limits: AdapterLimitsDto,
    pub bus_status: String,
    pub counters: AdapterCountersDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdaptersListBody {
    pub adapters: Vec<AdapterDto>,
}

pub trait AdapterSettingsApplyWatch: Send + Sync {
    fn adapter_settings_applied_load(&self) -> u32;
}

pub trait AdapterHttpState: Send + Sync {
    fn adapter_count(&self) -> u8;
    fn adapter_dto(&self, id: u8) -> Option<AdapterDto>;
    fn list_adapter_dtos(&self) -> Vec<AdapterDto>;
}

macro_rules! impl_adapter_http_state_from_port {
    ($bridge:ty) => {
        impl $crate::http::adapter_state::AdapterHttpState for $bridge {
            fn adapter_count(&self) -> u8 {
                self.port.adapter_count()
            }

            fn adapter_dto(
                &self,
                adapter_id: u8,
            ) -> Option<$crate::http::adapter_state::AdapterDto> {
                self.port
                    .adapter_view(adapter_id)
                    .map($crate::http::adapter_state::adapter_view_to_dto)
            }

            fn list_adapter_dtos(&self) -> Vec<$crate::http::adapter_state::AdapterDto> {
                self.port
                    .list_adapter_views()
                    .into_iter()
                    .map($crate::http::adapter_state::adapter_view_to_dto)
                    .collect()
            }
        }
    };
}
pub(crate) use impl_adapter_http_state_from_port;

pub struct AdapterHttpStateBridge {
    port: Arc<dyn AdapterReadPort>,
}

impl AdapterHttpStateBridge {
    pub fn new(port: Arc<dyn AdapterReadPort>) -> Self {
        Self { port }
    }
}

impl_adapter_http_state_from_port!(AdapterHttpStateBridge);

pub struct AdapterSettingsApplyWatchBridge {
    port: Arc<dyn AdapterApplyWatchPort>,
}

impl AdapterSettingsApplyWatchBridge {
    pub fn new(port: Arc<dyn AdapterApplyWatchPort>) -> Self {
        Self { port }
    }
}

impl AdapterSettingsApplyWatch for AdapterSettingsApplyWatchBridge {
    fn adapter_settings_applied_load(&self) -> u32 {
        self.port.adapter_settings_applied_load()
    }
}

pub(crate) fn adapter_view_to_dto(view: AdapterView) -> AdapterDto {
    AdapterDto {
        adapter_id: view.adapter_id,
        name: view.name,
        enabled: view.enabled,
        limits: AdapterLimitsDto {
            virtual_lamps: 64,
            groups: 16,
            scenes: 16,
        },
        bus_status: if view.enabled { "idle" } else { "disabled" }.to_string(),
        counters: AdapterCountersDto {
            commands: view.commands,
            timeouts: view.timeouts,
            errors: view.errors,
        },
    }
}
