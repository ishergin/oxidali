use serde::{Deserialize, Serialize};
use dali2rust_contracts::msg::ColorMode;

use super::physical_device::{CapabilityFlagsView, ColorTemperatureRangeView};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaInputInstanceView {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaInputView {
    pub short_address: u8,
    pub name: Option<String>,
    pub ha_expose: bool,
    pub instances: Vec<HaInputInstanceView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaLampView {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities: CapabilityFlagsView,
    pub color_temperature_range: Option<ColorTemperatureRangeView>,
    pub unreachable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HaGroupStateView {
    pub commanded: bool,
    pub any_on: bool,
    pub brightness: Option<u8>,
    pub color_mode: Option<ColorMode>,
    pub color_temperature_kelvin: Option<u16>,
    pub rgb: Option<(u8, u8, u8)>,
    pub xy: Option<(f64, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HaGroupView {
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities: CapabilityFlagsView,
    pub state: HaGroupStateView,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaSceneView {
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeAssistantSettingsView {
    pub enabled: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_username: String,
    pub broker_password_set: bool,
    pub discovery_prefix: String,
    pub state_topic_prefix: String,
    pub controller_id: String,
    pub publish_qos: u8,
    pub retain_state: bool,
    pub retain_discovery: bool,
    pub expose_input_devices: bool,
}

impl HomeAssistantSettingsView {
    pub fn broker_url_view(&self) -> String {
        if self.broker_host.is_empty() {
            String::new()
        } else {
            format!("mqtt://{}:{}", self.broker_host, self.broker_port)
        }
    }
}
