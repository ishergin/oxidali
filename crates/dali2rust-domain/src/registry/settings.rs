use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HclScheduleView {
    pub schedule_id: String,
    pub enabled: bool,
    pub algorithm: dali2rust_contracts::msg::HclAlgorithm,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: Vec<dali2rust_contracts::msg::HclTargetRow>,
    pub points: Vec<dali2rust_contracts::msg::HclSchedulePointRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HclOverrideTargetView {
    pub adapter_id: u8,
    pub scope: dali2rust_contracts::msg::HclTargetScope,
    pub group_id: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HclOverrideView {
    pub suspended: bool,
    pub targets: Vec<HclOverrideTargetView>,
    pub since_local_minutes: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollerSettingsView {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_mask: u8,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaliSettingsView {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationActiveMover {
    #[default]
    Unmoved,
    Command,
    Wire,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedundancySettingsView {
    pub enabled: bool,
    pub standby_role: bool,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PoliciesView {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
}

impl PoliciesView {
    #[must_use]
    pub fn manages_anything(&self) -> bool {
        self.system_failure_level.is_some() || self.power_on_level.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollTargetView {
    pub short_address: u8,
    pub is_dt8: bool,
    pub is_dt6: bool,
    pub declares_energy: bool,
    pub declares_diagnostics: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollTargetSelection {
    pub targets: Vec<PollTargetView>,
    pub excluded_unbound: u16,
    pub adapter_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyApplyCell {
    pub short_address: u8,
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub supported_device_types: Option<dali2rust_contracts::msg::DeviceTypeSet>,
}
