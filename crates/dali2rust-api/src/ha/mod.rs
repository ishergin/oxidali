pub mod brightness;
pub mod command;
pub mod discovery;
pub mod state;
pub mod topics;

use dali2rust_contracts::msg::LightSetpoint;
use dali2rust_domain::registry::{HaGroupView, HaLampView, HaSceneView};

pub fn input_instance_discovery_json(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    device_name: Option<&str>,
    short_address: u8,
    instance_number: u8,
    instance_type: u8,
) -> Option<(&'static str, String)> {
    discovery::input_instance_discovery_payload(
        topics,
        device,
        adapter_id,
        device_name,
        short_address,
        instance_number,
        instance_type,
    )
    .map(|(component, payload)| (component, payload.to_string()))
}

pub fn lamp_discovery_json(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    lamp: &HaLampView,
) -> String {
    lamp_discovery_payload(topics, device, adapter_id, lamp).to_string()
}

pub fn group_discovery_json(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    group: &HaGroupView,
) -> String {
    group_discovery_payload(topics, device, adapter_id, group).to_string()
}

pub fn scene_select_discovery_json(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    scenes: &[HaSceneView],
) -> String {
    scene_select_discovery_payload(topics, device, adapter_id, scenes).to_string()
}

pub fn light_state_json(setpoint: &LightSetpoint) -> String {
    light_state_payload(setpoint).to_string()
}

pub fn group_state_json(state: &dali2rust_domain::registry::HaGroupStateView) -> String {
    group_state_payload(state).to_string()
}

pub fn scene_select_state(active: Option<u8>, scenes: &[HaSceneView]) -> String {
    active
        .and_then(|id| scenes.iter().find(|s| s.scene_id == id))
        .map_or_else(|| SCENE_SELECT_NONE.to_string(), scene_option_label)
}

pub use brightness::{ha_to_level, level_to_ha, HA_BRIGHTNESS_SCALE};
pub use discovery::{
    group_discovery_payload, lamp_discovery_payload, scene_option_label,
    scene_select_discovery_payload, supported_color_modes, HaDevice, DISCOVERY_RETRACTION,
    SCENE_SELECT_NONE,
};
pub use command::{caps_accept, caps_accept_for_group, ha_light_command_to_setpoint};
pub use state::{input_event_state_payload, group_state_payload, light_state_payload};
pub use topics::{is_topic_safe, HaCommandTarget, HaTopics};
