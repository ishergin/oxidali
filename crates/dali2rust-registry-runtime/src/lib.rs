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
    PersistablePollerSettingsSlice, PersistableRedundancySettingsSlice, PersistableSceneSlice,
    RegistryStore,
    ADAPTERS_SLICE_VERSION,
    DALI_SETTINGS_SLICE_VERSION, GROUPS_SLICE_VERSION, HCL_SCHEDULES_SLICE_VERSION,
    HOME_ASSISTANT_SETTINGS_SLICE_VERSION, PHYSICAL_DEVICES_SLICE_VERSION,
    POLLER_SETTINGS_SLICE_VERSION, REDUNDANCY_SETTINGS_SLICE_VERSION, SCENES_SLICE_VERSION,
    VIRTUAL_LAMPS_SLICE_VERSION,
};
pub use runtime::registry::input_devices::{InputDeviceDetail, InputDeviceSummary, InstanceView, ReadValue};
pub use runtime::registry::import_stage::{ImportStageRefusal, StagedSlice};
pub use runtime::registry::rules_world::{
    RulesDeviceRow, RulesGroupRow, RulesInputRow, RulesLampRow,
};
pub use runtime::registry::transfer::{
    slice_key_from_name, transferable_slices, ForeignSliceOwner, ImportWriteFailure,
    RegistryOwnedOnly, SliceManifestRow,
};
pub use runtime::registry::physical_devices::DISCOVERY_EVICT_MISS_THRESHOLD;
pub use runtime::registry_apply_watch::RegistryApplyWatch;
pub use runtime::registry_events_worker::REGISTRY_EVENTS_HANDLED_EVENTS;
pub use runtime::registry::publish::REGISTRY_REQUIRED_EVENTS;
pub use runtime::registry::replaced_slices::{AnnouncedEvent, ANNOUNCED_EVENTS};
pub use runtime::registry_worker::{
    spawn_registry_worker, RegistryCommandCounters, RegistryEventsCounters,
    RegistryWorkerCounters, SlicePersistence, ANNOUNCEMENTS_PER_PARK, ANNOUNCEMENT_INBOX_FLOOR,
    ANNOUNCEMENT_PARK_WINDOW, ANNOUNCEMENT_SHARE_OF_INBOX,
    HELD_SLICE_REREAD_INTERVAL,
    REGISTRY_WORKER_HANDLED_COMMANDS,
};
