use serde_json::{json, Value};

use super::*;

const OWN: u8 = 0;
const NEIGHBOUR: u8 = 2;
const PRESSED_AGO_MS: u32 = 1_500;

struct Nodes {
    devices: u8,
    instances: u8,
}

impl Nodes {
    fn source(node: u8) -> ClusterInfoSource {
        if node == OWN {
            ClusterInfoSource::Own
        } else {
            ClusterInfoSource::Cached
        }
    }
}

impl ClusterInputsHttpState for Nodes {
    fn inputs_node(&self, node: u8) -> Option<ClusterInputsNodeDto> {
        let cached = node == NEIGHBOUR;
        (node == OWN || cached).then(|| ClusterInputsNodeDto {
            node,
            source: Self::source(node),
            set: cached.then_some(4),
            age_ms: cached.then_some(200),
            device_count: self.devices,
        })
    }

    fn input_owner(&self, node: u8) -> Option<ClusterInputOwnerDto> {
        (node == NEIGHBOUR).then(|| ClusterInputOwnerDto { controller_id: "barn".into(), discovery_prefix: "homeassistant".into() })
    }

    fn input_device(&self, node: u8, index: usize) -> Option<ClusterInputDeviceDto> {
        let short_address = u8::try_from(index).ok().filter(|short| *short < self.devices)?;
        Some(ClusterInputDeviceDto {
            node,
            source: Self::source(node),
            adapter_id: 0,
            short_address,
            name: (short_address == 0).then(|| "Hall panel".into()),
            present: true,
            instance_count: self.instances,
            ha_entity: node == NEIGHBOUR,
        })
    }

    fn input_instance(&self, _node: u8, _device_index: usize, index: usize) -> Option<ClusterInputInstanceDto> {
        let instance_number = u8::try_from(index).ok().filter(|number| *number < self.instances)?;
        let pressed = instance_number == 0;
        Some(ClusterInputInstanceDto {
            instance_number,
            instance_type: Some(1),
            instance_type_name: Some("push_button"),
            instance_groups: [Some(3), None, None],
            runtime: ClusterInputRuntimeDto {
                last_event_info: pressed.then_some(1),
                input_value: None,
                age_ms: pressed.then_some(PRESSED_AGO_MS),
            },
        })
    }
}

fn node_body(state: &Nodes, node: u8) -> Value {
    let summary = state.inputs_node(node).expect("a node with inputs");
    let mut out = Vec::new();
    write_node_inputs(state, &summary, &mut out).expect("written");
    serde_json::from_slice(&out).expect("valid JSON")
}

#[test]
fn a_cached_node_s_inputs_carry_the_owner_then_each_device_with_its_instances() {
    let body = node_body(&Nodes { devices: 1, instances: 2 }, NEIGHBOUR);
    let runtime = |info: Option<u16>, age: Option<u32>| json!({ "last_event_info": info, "input_value": null, "age_ms": age });
    let instance = |number: u8, runtime: Value| {
        json!({
            "instance_number": number,
            "instance_type": 1,
            "instance_type_name": "push_button",
            "instance_groups": [3, null, null],
            "runtime": runtime,
        })
    };
    let expected = json!({
        "node": NEIGHBOUR,
        "source": "cached",
        "set": 4,
        "age_ms": 200,
        "device_count": 1,
        "owner": { "controller_id": "barn", "discovery_prefix": "homeassistant" },
        "devices": [{
            "node": NEIGHBOUR,
            "source": "cached",
            "adapter_id": 0,
            "short_address": 0,
            "name": "Hall panel",
            "present": true,
            "instance_count": 2,
            "ha_entity": true,
            "instances": [instance(0, runtime(Some(1), Some(PRESSED_AGO_MS))), instance(1, runtime(None, None))],
        }],
    });
    assert_eq!(body, expected);
}

#[test]
fn a_node_without_input_devices_has_an_empty_list() {
    let body = node_body(&Nodes { devices: 0, instances: 0 }, OWN);
    assert_eq!(body["owner"], Value::Null);
    assert_eq!(body["devices"], json!([]));
    assert_eq!(body["source"], "own");
}

#[test]
fn a_full_node_streams_every_device_and_instance_slot() {
    let full = u8::try_from(CLUSTER_INPUT_DEVICES).expect("a device count");
    let body = node_body(&Nodes { devices: full + 1, instances: full + 1 }, NEIGHBOUR);
    let devices = body["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), CLUSTER_INPUT_DEVICES);
    assert!(devices.iter().all(|device| device["instances"].as_array().map(Vec::len) == Some(CLUSTER_INPUT_INSTANCES)));
}

#[test]
fn the_summary_lists_only_the_nodes_that_have_inputs() {
    let mut out = Vec::new();
    write_cluster_inputs(&Nodes { devices: 1, instances: 1 }, &mut out).expect("written");
    let body: Value = serde_json::from_slice(&out).expect("valid JSON");
    let nodes: Vec<u64> = body["nodes"].as_array().expect("nodes").iter().filter_map(|node| node["node"].as_u64()).collect();
    assert_eq!(nodes, vec![u64::from(OWN), u64::from(NEIGHBOUR)]);
}
