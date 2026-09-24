use dali2rust_domain::registry::{
    CapabilityFlagsView, HaGroupView, HaLampView, HaSceneView,
};
use serde_json::{json, Map, Value};

use super::brightness::HA_BRIGHTNESS_SCALE;
use super::topics::HaTopics;

pub struct HaDevice<'a> {
    pub controller_id: &'a str,
    pub version: &'a str,
}

impl HaDevice<'_> {
    fn to_json(&self) -> Value {
        json!({
            "identifiers": [self.controller_id],
            "name": format!("DALI controller {}", self.controller_id),
            "manufacturer": "dali2rust",
            "model": "DALI-2 controller",
            "sw_version": self.version,
        })
    }
}

pub fn supported_color_modes(caps: &CapabilityFlagsView) -> Vec<&'static str> {
    let mut modes = Vec::new();
    if caps.cct {
        modes.push("color_temp");
    }
    if caps.xy {
        modes.push("xy");
    }
    if caps.rgb {
        modes.push("rgb");
    }
    if modes.is_empty() {
        modes.push("brightness");
    }
    modes
}

fn entity_label(name: &str, fallback: impl FnOnce() -> String) -> String {
    if name.trim().is_empty() {
        fallback()
    } else {
        name.to_string()
    }
}

fn base_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    object_id: &str,
    name: &str,
) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(name));
    m.insert("unique_id".into(), json!(topics.unique_id(object_id)));
    m.insert("object_id".into(), json!(object_id));
    m.insert(
        "availability_topic".into(),
        json!(topics.availability_topic()),
    );
    m.insert("device".into(), device.to_json());
    m
}

fn light_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    object_id: &str,
    name: &str,
    caps: &CapabilityFlagsView,
    state_topic: String,
    command_topic: String,
) -> Map<String, Value> {
    let mut m = base_payload(topics, device, object_id, name);
    m.insert("schema".into(), json!("json"));
    m.insert("state_topic".into(), json!(state_topic));
    m.insert("command_topic".into(), json!(command_topic));
    m.insert("brightness".into(), json!(true));
    m.insert("brightness_scale".into(), json!(HA_BRIGHTNESS_SCALE));
    m.insert("color_temp_kelvin".into(), json!(true));
    m.insert("supported_color_modes".into(), json!(supported_color_modes(caps)));
    m
}

pub fn input_instance_discovery_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    device_name: Option<&str>,
    short_address: u8,
    instance_number: u8,
    instance_type: u8,
) -> Option<(&'static str, Value)> {
    let object_id = HaTopics::input_object_id(adapter_id, short_address, instance_number);
    let label = match device_name {
        Some(name) => format!("{name} {instance_number}"),
        None => format!("Input {short_address}/{instance_number}"),
    };
    let mut m = base_payload(topics, device, &object_id, &label);
    m.insert(
        "state_topic".into(),
        json!(topics.input_state_topic(adapter_id, short_address, instance_number)),
    );
    let component = match instance_type {
        1 => {
            m.insert(
                "event_types".into(),
                json!(dali2rust_domain::dali::dev103::ButtonEvent::ALL
                    .iter()
                    .map(|e| e.name())
                    .collect::<Vec<_>>()),
            );
            m.insert("device_class".into(), json!("button"));
            "event"
        }
        3 => {
            m.insert("device_class".into(), json!("occupancy"));
            m.insert("payload_on".into(), json!("ON"));
            m.insert("payload_off".into(), json!("OFF"));
            "binary_sensor"
        }
        2 | 4 => {
            m.insert("state_class".into(), json!("measurement"));
            "sensor"
        }
        _ => return None,
    };
    Some((component, Value::Object(m)))
}

pub fn lamp_discovery_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    lamp: &HaLampView,
) -> Value {
    let object_id = HaTopics::lamp_object_id(adapter_id, lamp.virtual_lamp_id);
    let mut m = light_payload(
        topics,
        device,
        &object_id,
        &entity_label(&lamp.name, || format!("Lamp {}", lamp.virtual_lamp_id)),
        &lamp.capabilities,
        topics.lamp_state_topic(adapter_id, lamp.virtual_lamp_id),
        topics.lamp_command_topic(adapter_id, lamp.virtual_lamp_id),
    );
    m.remove("availability_topic");
    m.insert(
        "availability".into(),
        json!([
            { "topic": topics.availability_topic() },
            { "topic": topics.lamp_availability_topic(adapter_id, lamp.virtual_lamp_id) },
        ]),
    );
    m.insert("availability_mode".into(), json!("all"));
    if let Some(range) = lamp.color_temperature_range {
        m.insert("min_kelvin".into(), json!(range.min_kelvin));
        m.insert("max_kelvin".into(), json!(range.max_kelvin));
    }
    Value::Object(m)
}

pub fn group_discovery_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    group: &HaGroupView,
) -> Value {
    let object_id = HaTopics::group_object_id(adapter_id, group.group_id);
    Value::Object(light_payload(
        topics,
        device,
        &object_id,
        &entity_label(&group.name, || format!("Group {}", group.group_id)),
        &group.capabilities,
        topics.group_state_topic(adapter_id, group.group_id),
        topics.group_command_topic(adapter_id, group.group_id),
    ))
}

pub const DISCOVERY_RETRACTION: &str = "";

pub const SCENE_SELECT_NONE: &str = "None";

pub fn scene_select_discovery_payload(
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
    scenes: &[HaSceneView],
) -> Value {
    let object_id = HaTopics::scene_select_object_id(adapter_id);
    let mut m = base_payload(topics, device, &object_id, "Scenes");
    m.insert(
        "state_topic".into(),
        json!(topics.scene_select_state_topic(adapter_id)),
    );
    m.insert(
        "command_topic".into(),
        json!(topics.scene_select_command_topic(adapter_id)),
    );
    let mut options = vec![Value::String(SCENE_SELECT_NONE.to_string())];
    options.extend(
        scenes
            .iter()
            .filter(|s| s.ha_select_enabled)
            .map(|s| json!(scene_option_label(s))),
    );
    m.insert("options".into(), Value::Array(options));
    Value::Object(m)
}

pub fn scene_option_label(scene: &HaSceneView) -> String {
    if scene.name.trim().is_empty() {
        format!("Scene {}", scene.scene_id)
    } else {
        scene.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_domain::registry::ColorTemperatureRangeView;

    fn topics() -> HaTopics {
        HaTopics::new("homeassistant", "dali", "ctl1")
    }

    fn device() -> HaDevice<'static> {
        HaDevice {
            controller_id: "ctl1",
            version: "0.1.0",
        }
    }

    fn caps(cct: bool, rgb: bool) -> CapabilityFlagsView {
        CapabilityFlagsView {
            brightness: true,
            cct,
            rgb,
            ..Default::default()
        }
    }

    fn lamp() -> HaLampView {
        HaLampView {
            virtual_lamp_id: 12,
            name: "Kitchen 1".to_string(),
            ha_entity_enabled: true,
            capabilities: caps(true, false),
            color_temperature_range: Some(ColorTemperatureRangeView {
                min_kelvin: 2700,
                max_kelvin: 6500,
            }),
            unreachable: false,
        }
    }

    #[test]
    fn a_lamp_payload_carries_the_documented_identity_and_topics() {
        let v = lamp_discovery_payload(&topics(), &device(), 0, &lamp());
        assert_eq!(v["unique_id"], "ctl1_a0_vl_12");
        assert_eq!(v["object_id"], "a0_vl_12");
        assert_eq!(v["schema"], "json");
        assert_eq!(v["state_topic"], "dali/ctl1/a0/vl/12/state");
        assert_eq!(v["command_topic"], "dali/ctl1/a0/vl/12/set");
        assert!(v.get("availability_topic").is_none());
        assert_eq!(
            v["availability"],
            json!([
                { "topic": "dali/ctl1/availability" },
                { "topic": "dali/ctl1/a0/vl/12/availability" },
            ])
        );
        assert_eq!(v["availability_mode"], "all");
        assert_eq!(v["brightness_scale"], 254);
        assert_eq!(v["color_temp_kelvin"], true);
        assert_eq!(v["supported_color_modes"], json!(["color_temp"]));
        assert_eq!(v["min_kelvin"], 2700);
        assert_eq!(v["device"]["identifiers"], json!(["ctl1"]));
    }

    #[test]
    fn a_lamp_with_no_limits_read_omits_the_range_rather_than_inventing_one() {
        let mut l = lamp();
        l.color_temperature_range = None;
        let v = lamp_discovery_payload(&topics(), &device(), 0, &l);
        assert!(v.get("min_kelvin").is_none());
        assert!(v.get("max_kelvin").is_none());
    }

    #[test]
    fn colour_modes_replace_the_brightness_fallback_instead_of_joining_it() {
        assert_eq!(supported_color_modes(&caps(false, false)), ["brightness"]);
        assert_eq!(supported_color_modes(&caps(true, false)), ["color_temp"]);
        assert_eq!(supported_color_modes(&caps(true, true)), ["color_temp", "rgb"]);
        assert_eq!(
            supported_color_modes(&CapabilityFlagsView::default()),
            ["brightness"]
        );
    }

    #[test]
    fn an_unnamed_entity_gets_a_label_rather_than_inheriting_the_device_name() {
        let mut l = lamp();
        l.name = String::new();
        let v = lamp_discovery_payload(&topics(), &device(), 0, &l);
        assert_eq!(v["name"], "Lamp 12");

        let group = HaGroupView {
            group_id: 7,
            name: "   ".to_string(),
            ha_entity_enabled: true,
            capabilities: caps(false, false),
            ..Default::default()
        };
        let v = group_discovery_payload(&topics(), &device(), 0, &group);
        assert_eq!(v["name"], "Group 7");
    }

    #[test]
    fn a_group_exposes_the_union_its_registry_view_already_carries() {
        let group = HaGroupView {
            group_id: 7,
            name: "Kitchen".to_string(),
            ha_entity_enabled: true,
            capabilities: caps(true, true),
            ..Default::default()
        };
        let v = group_discovery_payload(&topics(), &device(), 0, &group);
        assert_eq!(v["unique_id"], "ctl1_a0_group_7");
        assert_eq!(v["supported_color_modes"], json!(["color_temp", "rgb"]));
        assert!(v.get("min_kelvin").is_none(), "a mixed group has no honest range");
    }

    #[test]
    fn the_scene_select_lists_only_exposed_scenes_and_always_offers_none() {
        let scenes = vec![
            HaSceneView {
                scene_id: 0,
                name: "Evening".to_string(),
                ha_select_enabled: true,
            },
            HaSceneView {
                scene_id: 1,
                name: "Hidden".to_string(),
                ha_select_enabled: false,
            },
            HaSceneView {
                scene_id: 2,
                name: "  ".to_string(),
                ha_select_enabled: true,
            },
        ];
        let v = scene_select_discovery_payload(&topics(), &device(), 0, &scenes);
        assert_eq!(v["unique_id"], "ctl1_a0_scene_select");
        assert_eq!(v["options"], json!(["None", "Evening", "Scene 2"]));
    }
}
