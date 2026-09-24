use cucumber::{given, then};
use serde_json::{json, Value};

use crate::steps::physical_devices_steps::{
    read_groups_membership_from_gear, script_scan_discovery, wait_for_operation_status,
};
use crate::{DaliWorld};
use crate::steps::last_json;

const TWO_DEVICE_RANDOM_ADDRESSES: [(u8, u32); 2] = [(0, 0x5C1D_C2), (1, 0x2A_0F13)];

const DISCOVERY_DT8_FEATURES: u8 = 0x02;

// VL-036 VL-102
#[given("adapter 0 has discovered physical devices 0 and 1")]
async fn given_two_discovered_devices(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_scan_discovery(&mock, &TWO_DEVICE_RANDOM_ADDRESSES, DISCOVERY_DT8_FEATURES);
    }
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(br#"{"mode":"scan_known_short_addresses"}"#),
        "application/json",
    );
    wait_for_operation_status(world, "succeeded");
}

// VL-036 VL-102
#[given(regex = r"^physical device (\d+) reports membership of group (\d+) read from the gear$")]
async fn given_device_reports_group_membership(world: &mut DaliWorld, short: u8, group_id: u8) {
    read_groups_membership_from_gear(world, short, 1u16 << group_id);
}

// VL-001
#[given(regex = r#"^virtual lamp (\d+) has name "([^"]+)"$"#)]
async fn given_virtual_lamp_named(world: &mut DaliWorld, lamp_id: u8, name: String) {
    let path = format!("/api/v1/adapters/0/virtual-lamps/{lamp_id}");
    let body = serde_json::to_vec(&json!({ "name": name })).expect("vl patch body");
    world.send_http_request("PATCH", &path, Some(&body), "application/json");
    assert_eq!(
        world.last_response().expect("vl patch response").status,
        200,
        "virtual lamp {lamp_id} name patch should succeed"
    );
}

// VL-001
#[then(regex = r#"^the virtual lamps list should contain lamp (\d+) named "([^"]+)"$"#)]
async fn then_vl_list_contains_lamp(world: &mut DaliWorld, lamp_id: u8, name: String) {
    let json = last_json(world);
    let lamps = json
        .get("virtual_lamps")
        .and_then(Value::as_array)
        .expect("virtual_lamps array");
    let lamp = lamps
        .iter()
        .find(|lamp| {
            lamp.get("virtual_lamp_id").and_then(Value::as_u64) == Some(u64::from(lamp_id))
        })
        .unwrap_or_else(|| panic!("virtual lamp {lamp_id} missing from list: {json:?}"));
    assert_eq!(
        lamp.get("name").and_then(Value::as_str),
        Some(name.as_str()),
        "unexpected name for virtual lamp {lamp_id}: {lamp:?}"
    );
}

// VL-100
#[then(regex = r"^the virtual lamps list should not contain lamp (\d+)$")]
async fn then_vl_list_omits_lamp(world: &mut DaliWorld, lamp_id: u8) {
    let json = last_json(world);
    let lamps = json
        .get("virtual_lamps")
        .and_then(Value::as_array)
        .expect("virtual_lamps array");
    assert!(
        !lamps.iter().any(|lamp| {
            lamp.get("virtual_lamp_id").and_then(Value::as_u64) == Some(u64::from(lamp_id))
        }),
        "virtual lamp {lamp_id} is still listed after a delete: {json:?}"
    );
}

const VL_STATE_CONTRACT_KEYS: &[&str] = &[
    "power",
    "level",
    "color_mode",
    "color_temperature_kelvin",
    "xy",
    "rgb",
    "status",
    "failure_status",
    "value_source",
    "last_seen_ms",
    "last_dapc_source",
    "error",
];

const VL_CAPABILITY_KEYS: &[&str] = &[
    "brightness",
    "cct",
    "xy",
    "rgb",
    "rgbwaf",
    "scenes",
    "groups",
];

// VL-010
#[then("the virtual lamp detail should expose the full runtime-state contract")]
async fn then_vl_detail_exposes_contract(world: &mut DaliWorld) {
    let json = last_json(world);
    let obj = json.as_object().expect("detail DTO object");
    for key in [
        "device_type_effective",
        "device_type_source",
        "color_mode_effective",
        "color_mode_source",
        "ha_entity_enabled",
    ] {
        assert!(obj.contains_key(key), "missing {key} in detail: {json:?}");
    }
    let state = obj
        .get("state")
        .and_then(Value::as_object)
        .expect("state object");
    for key in VL_STATE_CONTRACT_KEYS {
        assert!(state.contains_key(*key), "missing state.{key}: {json:?}");
    }
    let caps = obj
        .get("capabilities")
        .and_then(Value::as_object)
        .expect("capabilities object");
    for key in VL_CAPABILITY_KEYS {
        assert!(
            caps.get(*key).map(Value::is_boolean) == Some(true),
            "missing boolean capabilities.{key}: {json:?}"
        );
    }
}

// VL-011
#[then("all virtual lamp capability flags should be false")]
async fn then_vl_capabilities_all_false(world: &mut DaliWorld) {
    let json = last_json(world);
    let caps = json
        .get("capabilities")
        .and_then(Value::as_object)
        .expect("capabilities object");
    for key in VL_CAPABILITY_KEYS {
        assert_eq!(
            caps.get(*key).and_then(Value::as_bool),
            Some(false),
            "capability {key} should be false: {caps:?}"
        );
    }
}
