use std::time::Duration;

use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::last_json;
use crate::steps::polling::fetch_json;
use crate::DaliWorld;

use super::{TEST_ADAPTER_ID, TEST_SHORT_ADDRESS};

pub(super) const READ_MODEL_TIMEOUT: Duration = Duration::from_secs(4);

pub(super) fn physical_device_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}")
}

pub(super) fn physical_devices_path(adapter_id: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices")
}

fn physical_device_attributes_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}/attributes")
}

fn physical_device_memory_banks_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}/memory-banks")
}

pub(super) fn fetch_physical_device_full(port: u16, adapter_id: u8, short_address: u8) -> Option<Value> {
    let mut core = fetch_json(port, &physical_device_path(adapter_id, short_address))?;
    let attributes = fetch_json(port, &physical_device_attributes_path(adapter_id, short_address))
        .and_then(|v| v.get("attributes").cloned())
        .unwrap_or_else(|| Value::Object(Default::default()));
    let banks = fetch_json(port, &physical_device_memory_banks_path(adapter_id, short_address))
        .and_then(|v| v.get("memory_banks").cloned())
        .unwrap_or_else(|| Value::Array(Vec::new()));
    core["attributes"] = attributes;
    core["memory_banks"] = banks;
    Some(core)
}

fn refresh_get(world: &mut DaliWorld, path: &str) -> Value {
    world.send_http_request("GET", path, None, "");
    last_json(world)
}

pub(super) fn wait_for_physical_device(world: &mut DaliWorld, predicate: impl Fn(&Value) -> bool) -> Value {
    let path = physical_device_path(TEST_ADAPTER_ID, TEST_SHORT_ADDRESS);
    let port = world.server_port();
    wait_until(
        || {
            fetch_physical_device_full(port, TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
                .as_ref()
                .is_some_and(&predicate)
        },
        READ_MODEL_TIMEOUT,
    );
    let core = refresh_get(world, &path);
    fetch_physical_device_full(world.server_port(), TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
        .unwrap_or(core)
}

pub(super) fn fetch_physical_device(world: &mut DaliWorld) -> Value {
    let path = physical_device_path(TEST_ADAPTER_ID, TEST_SHORT_ADDRESS);
    let core = refresh_get(world, &path);
    fetch_physical_device_full(world.server_port(), TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
        .unwrap_or(core)
}

pub(super) fn wait_for_physical_devices(world: &mut DaliWorld, predicate: impl Fn(&Value) -> bool) -> Value {
    let path = physical_devices_path(TEST_ADAPTER_ID);
    let port = world.server_port();
    wait_until(
        || fetch_json(port, &path).as_ref().is_some_and(&predicate),
        READ_MODEL_TIMEOUT,
    );
    refresh_get(world, &path)
}

pub(super) fn wait_pd_state_field(world: &DaliWorld, short: u8, pointer: &str, expected: &Value) {
    let port = world.server_port();
    let path = physical_device_path(TEST_ADAPTER_ID, short);
    wait_until(
        || fetch_json(port, &path).as_ref().and_then(|json| json.pointer(pointer)) == Some(expected),
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_json(port, &path);
    assert_eq!(
        json.as_ref().and_then(|j| j.pointer(pointer)),
        Some(expected),
        "physical device {short} {pointer}: {json:?}"
    );
}
