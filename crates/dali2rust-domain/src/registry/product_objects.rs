use serde::{Deserialize, Serialize};

use super::physical_device::{
    CapabilityFlagsView, ColorTemperatureRangeView, PhysicalDeviceStateView,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualLampSnapshot {
    pub name: String,
    pub runtime_level: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterSnapshot {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AdapterView {
    pub adapter_id: u8,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VirtualLampView {
    pub adapter_id: u8,
    pub virtual_lamp_id: u8,
    pub name: String,
    pub device_type_effective: String,
    pub device_type_source: String,
    pub color_mode_effective: String,
    pub color_mode_source: String,
    pub binding_short: Option<u8>,
    pub ha_entity_enabled: bool,
    pub state: PhysicalDeviceStateView,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    pub adapter_id: u8,
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities_summary: CapabilityFlagsView,
    pub dirty: bool,
    pub member_count_desired: u8,
    pub member_count_applied: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMatrixGroupView {
    pub group_id: u8,
    pub name: String,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembershipMatrixRowView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub desired: [bool; 16],
    pub applied: [bool; 16],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembershipMatrixView {
    pub adapter_id: u8,
    pub groups: Vec<GroupMatrixGroupView>,
    pub rows: Vec<GroupMembershipMatrixRowView>,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneView {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
    pub row_count_included: u8,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRowStateView {
    pub included: bool,
    pub power: Option<String>,
    pub level: Option<u8>,
    pub color_mode: Option<String>,
    pub color_temperature_kelvin: Option<u16>,
    pub xy: Option<(u16, u16)>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMatrixRowView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub capabilities: CapabilityFlagsView,
    pub desired: SceneRowStateView,
    pub applied: SceneRowStateView,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMatrixView {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub rows: Vec<SceneMatrixRowView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualLampCapabilityView {
    pub virtual_lamp_id: u8,
    pub binding_short: Option<u8>,
    pub capabilities: CapabilityFlagsView,
}
