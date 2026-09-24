mod adapters;
pub(crate) mod config_write_stage;
mod conversions;
mod groups;
mod ha_publish;
pub(crate) mod hcl_schedules;
pub mod input_devices;
pub(crate) mod home_assistant_settings;
mod memory_bank_parser;
mod memory_banks;
mod persistence;
pub(crate) mod persistence_slices;
pub(crate) mod persistence_stream;
pub(crate) mod physical_devices;
pub(crate) mod dali_settings;
pub(crate) mod policies;
pub(crate) mod redundancy_settings;
pub mod transfer;
pub(crate) mod poller_settings;
pub mod publish;
mod resolve;
mod rules_world;
mod runtime_fields;
mod scenes;
mod store;
mod views;
mod virtual_lamps;

pub use persistence_slices::{
    decode_persistence_blob, decode_versioned_slice, encode_persistence_blob, PersistableAdapterRow,
    PersistableAdapterSlice, PersistableDaliSettingsSlice, PersistableGroupMatrixRow,
    PersistableRedundancySettingsSlice,
    PersistableGroupRecord, PersistableGroupsSlice, PersistableHomeAssistantSettingsSlice,
    PersistablePhysicalDeviceRecord, PersistablePhysicalDevicesSlice, PersistablePollerSettingsSlice,
    PersistableVirtualLampsSlice, PersistableVlRecord, PersistenceEnvelope,
    ADAPTERS_SLICE_VERSION, DALI_SETTINGS_SLICE_VERSION, GROUPS_SLICE_VERSION,
    HCL_SCHEDULES_SLICE_VERSION, HOME_ASSISTANT_SETTINGS_SLICE_VERSION,
    PHYSICAL_DEVICES_SLICE_VERSION, POLLER_SETTINGS_SLICE_VERSION,
    REDUNDANCY_SETTINGS_SLICE_VERSION, SCENES_SLICE_VERSION,
    VIRTUAL_LAMPS_SLICE_VERSION,
};
pub use persistence::{HydrateCounts, PersistenceHydrateReport};
pub use store::{DisplayCounts, PersistenceCounters, RegistryStore};
