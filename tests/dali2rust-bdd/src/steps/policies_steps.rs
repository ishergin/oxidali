use std::cell::RefCell;
use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_MASK;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::{json, Value};

use crate::steps::frames::standard_frame;
use crate::steps::last_json;
use crate::steps::physical_devices::discovery_scripts::script_scan_discovery;
use crate::steps::physical_devices::write_scripts::{
    script_level_store, POWER_ON_STORE, SYSTEM_FAILURE_STORE,
};
use crate::steps::polling::{fetch_json, wait_for_operation_status};
use crate::steps::virtual_lamps_steps::{DISCOVERY_DT8_FEATURES, TWO_DEVICE_RANDOM_ADDRESSES};
use crate::steps::wire::assert_frame_before;
use crate::DaliWorld;

const WRITE_ATTEMPTS: usize = 3;
const POLICY_SHORT_ADDRESS: u8 = 0;
const DISCOVERY_POLICY_APPLY_PREFIX: &str = "policy-apply-0-";
const OPERATION_LISTING_TIMEOUT: Duration = Duration::from_secs(8);

// POLICY-007 POLICY-020
#[given(regex = r"^a policy write script setting both levels to (\d+) on short addresses 0 and 1$")]
async fn given_policy_write_script(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for short in [0u8, 1] {
        script_policy_pair(&mock, short, level);
    }
}

fn script_policy_pair(mock: &MockDaliTransport, short: u8, level: u8) {
    for store in [POWER_ON_STORE, SYSTEM_FAILURE_STORE] {
        script_level_store(mock, short, store, level, (Some(COLOUR_TYPE_BYTE_MASK), Some(level)));
    }
}

// POLICY-010
#[given(regex = r"^a policy write of power-on level (\d+) that short address 0 answers with (\d+) on every attempt$")]
async fn given_policy_write_refused(world: &mut DaliWorld, level: u8, answer: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for _ in 0..WRITE_ATTEMPTS {
        script_power_on_write(&mock, level, Some(answer));
    }
}

// POLICY-011
#[given(regex = r"^a policy write of power-on level (\d+) that short address 0 leaves unanswered$")]
async fn given_policy_write_unanswered(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_power_on_write(&mock, level, None);
}

fn script_power_on_write(mock: &MockDaliTransport, level: u8, answer: Option<u8>) {
    script_level_store(mock, POLICY_SHORT_ADDRESS, POWER_ON_STORE, level, (Some(COLOUR_TYPE_BYTE_MASK), answer));
}

// POLICY-017
#[given(regex = r"^a policy write script setting both levels to (\d+) on short address 0 only$")]
async fn given_policy_write_script_first_only(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_policy_pair(&mock, POLICY_SHORT_ADDRESS, level);
}

// POLICY-018
#[given(regex = r"^a discovery script for physical devices 0 and 1 followed by a policy write setting both levels to (\d+) on short address 0$")]
async fn given_discovery_then_policy_write(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scan_discovery(&mock, &TWO_DEVICE_RANDOM_ADDRESSES, DISCOVERY_DT8_FEATURES);
    script_policy_pair(&mock, POLICY_SHORT_ADDRESS, level);
}

// POLICY-016
#[when(regex = r"^I PATCH a policy exception list naming (\d+) devices$")]
async fn when_patch_exception_list_of_length(world: &mut DaliWorld, count: u8) {
    let devices: Vec<Value> =
        (0..count).map(|short| json!({ "adapter_id": 0, "short_address": short })).collect();
    let body = serde_json::to_vec(&json!({ "exceptions": devices })).expect("exceptions body");
    world.send_http_request("PATCH", "/api/v1/policies", Some(&body), "application/json");
}

// POLICY-018
#[then("the policy apply started by the discovery eventually succeeds")]
async fn then_discovery_policy_apply_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status(world, "succeeded");
    let port = world.server_port();
    let key = RefCell::new(None);
    wait_until(
        || {
            *key.borrow_mut() = policy_apply_key(port);
            key.borrow().is_some()
        },
        OPERATION_LISTING_TIMEOUT,
    );
    let key = key.into_inner().expect("a policy-apply operation started by the discovery");
    world.send_http_request("GET", &format!("/api/v1/operations/{key}"), None, "");
    wait_for_operation_status(world, "succeeded");
}

fn policy_apply_key(port: u16) -> Option<String> {
    let list = fetch_json(port, "/api/v1/operations")?;
    list.get("operations")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|key| key.starts_with(DISCOVERY_POLICY_APPLY_PREFIX))
        .map(String::from)
}

// POLICY-017 POLICY-018
#[then(regex = r"^the policy-apply operation result should list written short addresses \[([\d, ]*)\] and skipped short addresses \[([\d, ]*)\]$")]
async fn then_policy_apply_written_and_skipped(world: &mut DaliWorld, written: String, skipped: String) {
    let json = last_json(world);
    assert_eq!(result_shorts(&json, "written"), parse_shorts(&written), "written: {json:?}");
    assert_eq!(result_shorts(&json, "skipped"), parse_shorts(&skipped), "skipped: {json:?}");
    assert_eq!(result_shorts(&json, "failed"), Vec::<u64>::new(), "failed: {json:?}");
    for (bucket, expected) in [("written_total", parse_shorts(&written)), ("skipped_total", parse_shorts(&skipped))] {
        let total = json.pointer(&format!("/result/{bucket}")).and_then(Value::as_u64);
        assert_eq!(total, u64::try_from(expected.len()).ok(), "{bucket}: {json:?}");
    }
}

// POLICY-017
#[then(regex = r#"^every skipped short address in the policy-apply operation result should give the reason "([^"]+)"$"#)]
async fn then_policy_apply_skip_reason(world: &mut DaliWorld, reason: String) {
    let json = last_json(world);
    let skipped = json.pointer("/result/skipped").and_then(Value::as_array).expect("result.skipped array");
    assert!(!skipped.is_empty(), "nothing skipped: {json:?}");
    for entry in skipped {
        assert_eq!(entry.get("reason").and_then(Value::as_str), Some(reason.as_str()), "{json:?}");
    }
}

fn result_shorts(json: &Value, bucket: &str) -> Vec<u64> {
    let entries = json
        .pointer(&format!("/result/{bucket}"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("result.{bucket} array in {json:?}"));
    let mut shorts: Vec<u64> =
        entries.iter().filter_map(|entry| entry.get("short_address").and_then(Value::as_u64)).collect();
    shorts.sort_unstable();
    shorts
}

fn parse_shorts(list: &str) -> Vec<u64> {
    list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| s.parse().expect("short address")).collect()
}

// POLICY-017 POLICY-018
#[then(regex = r"^the DALI mock transport should have stored the power-on level before the system-failure level on short address (\d+)$")]
async fn then_power_on_stored_before_system_failure(world: &mut DaliWorld, short: u8) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_frame_before(
        &frames,
        standard_frame(short, StandardCommand::SetPowerOnLevel),
        standard_frame(short, StandardCommand::SetSystemFailureLevel),
    );
}

// POLICY-017 POLICY-018
#[then(regex = r"^the DALI mock transport should have sent no SET POWER ON LEVEL or SET SYSTEM FAILURE LEVEL to short address (\d+)$")]
async fn then_no_policy_store_to(world: &mut DaliWorld, short: u8) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    for command in [StandardCommand::SetPowerOnLevel, StandardCommand::SetSystemFailureLevel] {
        let store = standard_frame(short, command);
        assert!(!frames.contains(&store), "0x{store:04X} reached short address {short}: {frames:?}");
    }
}
