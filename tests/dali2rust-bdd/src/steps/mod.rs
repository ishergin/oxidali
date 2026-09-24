use serde_json::Value;

use crate::DaliWorld;

pub mod commissioning_steps;
pub mod contracts_steps;
pub mod diagnostic_steps;
pub mod fanout_steps;
pub mod firmware_steps;
pub mod groups_steps;
pub mod hcl_steps;
pub mod input_devices_steps;
pub mod rules_steps;
pub mod mqtt_steps;
pub mod operations_steps;
pub mod physical_devices_steps;
pub mod poller_steps;
pub mod policies_steps;
pub mod redundancy_steps;
pub mod scenes_steps;
pub mod stats_steps;
pub mod system_steps;
pub mod virtual_lamps_steps;
pub mod web_ui_steps;
pub mod websocket_steps;
pub mod wire;

pub fn response_json(response: &crate::TestResponse) -> Value {
    serde_json::from_slice(&response.body).expect("response should be JSON")
}

pub fn last_json(world: &DaliWorld) -> Value {
    response_json(world.last_response().expect("last response"))
}

pub fn assert_result_skips_lamp(world: &DaliWorld, virtual_lamp_id: u8, reason: &str) {
    let json = last_json(world);
    let skipped = json
        .pointer("/result/skipped")
        .and_then(Value::as_array)
        .expect("result.skipped array");
    let entry = skipped
        .iter()
        .find(|entry| {
            entry.get("virtual_lamp_id").and_then(Value::as_u64) == Some(u64::from(virtual_lamp_id))
        })
        .unwrap_or_else(|| panic!("skipped entry for vl{virtual_lamp_id} missing: {json:?}"));
    assert_eq!(
        entry.get("reason").and_then(Value::as_str),
        Some(reason),
        "unexpected skipped entry: {entry:?}"
    );
}

pub fn get_json(world: &mut DaliWorld, path: &str) -> Value {
    world.send_http_request("GET", path, None, "");
    let resp = world
        .last_response()
        .unwrap_or_else(|| panic!("GET {path}: no response"));
    assert_eq!(resp.status, 200, "GET {path}");
    serde_json::from_slice(&resp.body)
        .unwrap_or_else(|err| panic!("GET {path}: body should be JSON: {err}"))
}

pub fn await_config_write(world: &mut DaliWorld, what: &str) {
    let status = world
        .last_response()
        .unwrap_or_else(|| panic!("{what}: no response"))
        .status;
    assert_eq!(status, 202, "{what} should be accepted");
    physical_devices_steps::wait_for_operation_status(world, "succeeded");
}
