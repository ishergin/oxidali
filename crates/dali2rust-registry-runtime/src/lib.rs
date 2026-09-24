#![allow(
    clippy::too_many_arguments,
    reason = "Registry worker dispatch arms mirror the routed bus command surface."
)]

pub mod runtime;

#[cfg(test)]
mod test_support;

pub use runtime::registry::{
    decode_persistence_blob, encode_persistence_blob,     PersistableAdapterRow, PersistableAdapterSlice,
    PersistableGroupMatrixRow, PersistableGroupRecord, PersistableGroupsSlice,
    PersistablePhysicalDeviceRecord, PersistablePhysicalDevicesSlice, PersistableVirtualLampsSlice,
    decode_versioned_slice, DisplayCounts, PersistableVlRecord, PersistenceCounters,
    PersistenceEnvelope,
    HydrateCounts,
    PersistenceHydrateReport,
    PersistableDaliSettingsSlice, PersistableHomeAssistantSettingsSlice,
    PersistablePollerSettingsSlice, PersistableRedundancySettingsSlice, RegistryStore,
    ADAPTERS_SLICE_VERSION,
    DALI_SETTINGS_SLICE_VERSION, GROUPS_SLICE_VERSION, HCL_SCHEDULES_SLICE_VERSION,
    HOME_ASSISTANT_SETTINGS_SLICE_VERSION, PHYSICAL_DEVICES_SLICE_VERSION,
    POLLER_SETTINGS_SLICE_VERSION, REDUNDANCY_SETTINGS_SLICE_VERSION, SCENES_SLICE_VERSION,
    VIRTUAL_LAMPS_SLICE_VERSION,
};
pub use runtime::registry::input_devices::{InputDeviceDetail, InputDeviceSummary, InstanceView, ReadValue};
pub use runtime::registry::transfer::{slice_key_from_name, SliceManifestRow};
pub use runtime::registry_apply_watch::RegistryApplyWatch;
pub use runtime::registry_events_worker::REGISTRY_EVENTS_HANDLED_EVENTS;
pub use runtime::registry::publish::REGISTRY_REQUIRED_EVENTS;
pub use runtime::registry_worker::{
    spawn_registry_worker, RegistryCommandCounters, RegistryEventsCounters,
    RegistryWorkerCounters, REGISTRY_WORKER_HANDLED_COMMANDS,
};
