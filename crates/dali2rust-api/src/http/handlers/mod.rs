pub mod commissioning;
pub mod adapters;
pub mod common;
pub mod controller;
pub mod diagnostics;
pub mod firmware;
pub mod groups;
pub mod hcl;
pub mod input_devices;
pub mod rules;
pub mod hcl_override;
pub mod hcl_validate;
pub mod health;
pub mod json_command;
pub mod operation_dispatch;
pub mod operations;
pub mod physical_devices;
pub(crate) mod resource_surface;
pub mod scenes;
pub mod settings_home_assistant;
pub mod config_transfer;
pub mod policies;
pub mod redundancy;
pub mod settings_dali;
pub mod settings_redundancy;
pub mod settings_poller;
pub mod static_assets;
pub mod stats;
pub mod time;
pub mod virtual_lamps;

pub use adapters::{AdapterGetHandler, AdapterPatchHandler, AdaptersListHandler};
pub use controller::ControllerSummaryHandler;
pub use groups::{
    GroupApplyHandler, GroupGetHandler, GroupMembershipMatrixGetHandler,
    GroupMembershipMatrixWriteHandler, GroupPatchHandler, GroupTargetStateHandler,
    GroupsListHandler,
};
pub use json_command::{DaliCommandMapper, JsonCommandHandler, LevelMapper, RawMapper};
pub use operations::{OperationGetHandler, OperationsListHandler};
pub use scenes::{
    SceneApplyHandler, SceneGetHandler, SceneMatrixGetHandler, SceneMatrixWriteHandler,
    ScenePatchHandler, SceneRecallHandler, ScenesListHandler,
};
pub use physical_devices::{
    AdapterDiscoveryRunsHandler, PhysicalDeviceAttributeReadsHandler,
    PhysicalDeviceAttributesHandler, PhysicalDeviceGetHandler, PhysicalDeviceMemoryBanksHandler,
    PhysicalDevicePatchHandler, PhysicalDeviceTargetStateHandler,
    PhysicalDeviceWriteAttributesHandler, PhysicalDevicesListHandler,
};
pub use virtual_lamps::{
    VirtualLampBindingDeleteHandler, VirtualLampBindingPutHandler, VirtualLampGetHandler,
    VirtualLampPatchHandler, VirtualLampTargetStateHandler, VirtualLampsListHandler,
};
pub use time::TimeHandler;
