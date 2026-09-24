use std::sync::atomic::Ordering;
use std::sync::Arc;

use dali2rust_domain::registry::{
    AdapterApplyWatchPort, DaliSettingsApplyWatchPort, GroupMetadataWatchPort,
    HomeAssistantSettingsApplyWatchPort, PhysicalDeviceApplyWatchPort,
    PollerSettingsApplyWatchPort, SceneMetadataWatchPort, VirtualLampBindingWatchPort,
    VirtualLampMetadataWatchPort,
};

use crate::runtime::registry_worker::RegistryWorkerCounters;

#[derive(Clone)]
pub struct RegistryApplyWatch {
    counters: Arc<RegistryWorkerCounters>,
}

impl RegistryApplyWatch {
    pub fn new(counters: Arc<RegistryWorkerCounters>) -> Self {
        Self { counters }
    }
}

impl AdapterApplyWatchPort for RegistryApplyWatch {
    fn adapter_settings_applied_load(&self) -> u32 {
        self.counters
            .command
            .adapter_settings_applied
            .load(Ordering::Acquire)
    }
}

impl dali2rust_domain::registry::PoliciesApplyWatchPort for RegistryApplyWatch {
    fn policies_applied_load(&self) -> u32 {
        self.counters
            .command
            .policies_applied
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

impl dali2rust_domain::registry::RedundancySettingsApplyWatchPort for RegistryApplyWatch {
    fn redundancy_settings_applied_load(&self) -> u32 {
        self.counters
            .command
            .redundancy_settings_applied
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

impl DaliSettingsApplyWatchPort for RegistryApplyWatch {
    fn dali_settings_applied_load(&self) -> u32 {
        self.counters
            .command
            .dali_settings_applied
            .load(Ordering::Acquire)
    }
}

impl PollerSettingsApplyWatchPort for RegistryApplyWatch {
    fn poller_settings_applied_load(&self) -> u32 {
        self.counters
            .command
            .poller_settings_applied
            .load(Ordering::Acquire)
    }
}

impl HomeAssistantSettingsApplyWatchPort for RegistryApplyWatch {
    fn home_assistant_settings_applied_load(&self) -> u32 {
        self.counters
            .command
            .home_assistant_settings_applied
            .load(Ordering::Acquire)
    }
}

impl PhysicalDeviceApplyWatchPort for RegistryApplyWatch {
    fn physical_override_applied_load(&self) -> u32 {
        self.counters
            .command
            .physical_device_overrides_applied
            .load(Ordering::Acquire)
    }
}

impl VirtualLampMetadataWatchPort for RegistryApplyWatch {
    fn virtual_lamp_metadata_applied_load(&self) -> u32 {
        self.counters
            .command
            .virtual_lamp_metadata_patches_applied
            .load(Ordering::Acquire)
    }
}

impl VirtualLampBindingWatchPort for RegistryApplyWatch {
    fn virtual_lamp_binding_applied_load(&self) -> u32 {
        self.counters
            .command
            .virtual_lamp_bindings_applied
            .load(Ordering::Acquire)
    }
}

impl GroupMetadataWatchPort for RegistryApplyWatch {
    fn group_metadata_applied_load(&self) -> u32 {
        self.counters
            .command
            .group_metadata_patches_applied
            .load(Ordering::Acquire)
    }
}

impl SceneMetadataWatchPort for RegistryApplyWatch {
    fn scene_metadata_applied_load(&self) -> u32 {
        self.counters
            .command
            .scene_metadata_patches_applied
            .load(Ordering::Acquire)
    }
}

