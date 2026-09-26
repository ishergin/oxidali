use std::time::Duration;

use cucumber::{given, then};
use dali2rust_test_support::wait_until;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::Value;

use crate::steps::physical_devices_steps::{fetch_json, special_frame, standard_frame};
use crate::DaliWorld;

const IDENTIFY_PAIR_FRAMES: usize = 2;

// COMM-030 COMM-032 COMM-034 COMM-038 COMM-092
#[given(regex = r"^an identify script for short address (\d+)$")]
async fn given_identify_script(world: &mut DaliWorld, short: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for _ in 0..IDENTIFY_PAIR_FRAMES {
        mock.expect_forward_frame(standard_frame(short, StandardCommand::IdentifyDevice));
    }
}

fn last_operation_result(world: &DaliWorld) -> Value {
    let port = world.server_port;
    let list = fetch_json(port, "/api/v1/operations").expect("operations list");
    let keys = list
        .get("operations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let key = keys
        .iter()
        .filter_map(Value::as_str)
        .find(|k| k.starts_with("comm-ident-"))
        .unwrap_or_else(|| panic!("no commissioning identify operation in {keys:?}"))
        .to_string();
    let detail =
        fetch_json(port, &format!("/api/v1/operations/{key}")).expect("operation detail");
    detail.get("result").cloned().unwrap_or(Value::Null)
}

// COMM-032
#[then(regex = r"^the operation result short_address should be (\d+)$")]
async fn then_result_short_address(world: &mut DaliWorld, expected: u64) {
    let result = last_operation_result(world);
    assert_eq!(
        result["short_address"].as_u64(),
        Some(expected),
        "operation result must name the located device, got {result}"
    );
}

// COMM-032
#[then(regex = r#"^the operation result identify_mechanism should be "([^"]+)"$"#)]
async fn then_result_identify_mechanism(world: &mut DaliWorld, expected: String) {
    let result = last_operation_result(world);
    assert_eq!(
        result["identify_mechanism"].as_str(),
        Some(expected.as_str()),
        "operation result must state the mechanism, got {result}"
    );
}

// COMM-038
#[then(regex = r"^the transport should observe an identify device pair for short address (\d+)$")]
async fn then_transport_observes_identify_pair(world: &mut DaliWorld, short: u8) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let identify = standard_frame(short, StandardCommand::IdentifyDevice);
    let first = frames
        .iter()
        .position(|&f| f == identify)
        .unwrap_or_else(|| panic!("no IDENTIFY DEVICE for short {short} in {frames:?}"));
    assert_eq!(
        frames.get(first + 1).copied(),
        Some(identify),
        "§12.1.3.3: a configuration command goes twice, back to back; frames: {frames:?}"
    );
    assert_eq!(
        frames.iter().filter(|&&f| f == identify).count(),
        IDENTIFY_PAIR_FRAMES,
        "one pair and no more — a third frame is a stray, not a repeat: {frames:?}"
    );
}

// COMM-092
#[then(regex = r"^the identify sequence for short address (\d+) should contain no level command$")]
async fn then_identify_sends_no_level_command(world: &mut DaliWorld, short: u8) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let identify = standard_frame(short, StandardCommand::IdentifyDevice);
    let start = frames
        .iter()
        .position(|&f| f == identify)
        .unwrap_or_else(|| panic!("no IDENTIFY DEVICE for short {short} in {frames:?}"));
    let forbidden = [
        StandardCommand::RecallMaxLevel,
        StandardCommand::RecallMinLevel,
        StandardCommand::Off,
        StandardCommand::GoToLastActiveLevel,
    ]
    .map(|command| standard_frame(short, command));
    for frame in &frames[start..] {
        assert!(
            !forbidden.contains(frame),
            "identify must move no variable (§9.14.3.1): {frames:?}"
        );
        let is_dapc = (0..=254u8)
            .any(|level| *frame == standard_frame(short, StandardCommand::DirectArcPower { level }));
        assert!(!is_dapc, "identify must not restore a level; frames: {frames:?}");
    }
}

const CONFIG_COMMAND_SENDS: usize = 2;

fn encoded_short(short: u8) -> u8 {
    ((short & 0x3F) << 1) | 0x01
}

// COMM-001 COMM-010
#[given(regex = r"^an address-change script from short address (\d+) to (\d+)$")]
async fn given_address_change_script(world: &mut DaliWorld, from: u8, to: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    let encoded = encoded_short(to);
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(encoded)));
    mock.expect_forward_frame_with_backward(
        standard_frame(from, StandardCommand::QueryContentDtr0),
        Some(encoded),
    );
    for _ in 0..CONFIG_COMMAND_SENDS {
        mock.expect_forward_frame(standard_frame(from, StandardCommand::SetShortAddress));
    }
    let _ = encoded;
    mock.expect_forward_frame_with_backward(
        standard_frame(to, StandardCommand::QueryStatus),
        Some(0x00),
    );
}

fn address_change_result(world: &DaliWorld) -> Value {
    let port = world.server_port;
    let list = fetch_json(port, "/api/v1/operations").expect("operations list");
    let keys = list
        .get("operations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let key = keys
        .iter()
        .filter_map(Value::as_str)
        .find(|k| k.starts_with("comm-addr-"))
        .unwrap_or_else(|| panic!("no address-change operation in {keys:?}"))
        .to_string();
    let detail =
        fetch_json(port, &format!("/api/v1/operations/{key}")).expect("operation detail");
    detail.get("result").cloned().unwrap_or(Value::Null)
}

// COMM-010
#[then(regex = r"^the operation result old_short_address should be (\d+)$")]
async fn then_result_old_short(world: &mut DaliWorld, expected: u64) {
    let result = address_change_result(world);
    assert_eq!(
        result["old_short_address"].as_u64(),
        Some(expected),
        "result must report the address moved FROM, got {result}"
    );
}

// COMM-010
#[then(regex = r"^the operation result new_short_address should be (\d+)$")]
async fn then_result_new_short(world: &mut DaliWorld, expected: u64) {
    let result = address_change_result(world);
    assert_eq!(
        result["new_short_address"].as_u64(),
        Some(expected),
        "result must report the address moved TO, got {result}"
    );
}

// COMM-010
#[then(regex = r"^physical device (\d+) should eventually exist on adapter (\d+)$")]
async fn then_device_exists(world: &mut DaliWorld, short: u8, adapter: u8) {
    let port = world.server_port;
    let path = format!("/api/v1/adapters/{adapter}/physical-devices/{short}");
    wait_until(
        || fetch_json(port, &path).is_some(),
        Duration::from_secs(5),
    );
}

// COMM-010
#[then(regex = r"^physical device (\d+) should eventually be absent on adapter (\d+)$")]
async fn then_device_absent(world: &mut DaliWorld, short: u8, adapter: u8) {
    let port = world.server_port;
    let path = format!("/api/v1/adapters/{adapter}/physical-devices/{short}");
    wait_until(
        || fetch_json(port, &path).is_none(),
        Duration::from_secs(5),
    );
}

// COMM-080
#[given("an expert step script for initialise unaddressed")]
async fn given_step_script_initialise(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0xFF)));
}

// COMM-081
#[given(regex = r"^an expert step script for search address (\d+)$")]
async fn given_step_script_search_address(world: &mut DaliWorld, addr: u32) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(
        ((addr >> 16) & 0xFF) as u8,
    )));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(
        ((addr >> 8) & 0xFF) as u8,
    )));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL((addr & 0xFF) as u8)));
}

// COMM-082
#[given("an expert step script for compare answering yes")]
async fn given_step_script_compare(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), Some(0xFF));
}

// COMM-089
#[given(regex = r"^an expert step script for query short address answering (\d+)$")]
async fn given_step_script_query_short(world: &mut DaliWorld, short: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(
        special_frame(SpecialCommand::QueryShortAddress),
        Some(encoded_short(short)),
    );
}

// COMM-093
#[given("an expert step script for compare answering with a violating frame")]
async fn given_step_script_compare_violation(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_corrupted_in_window(special_frame(SpecialCommand::Compare));
}

// COMM-094
#[given("an expert step script for query short address answering with a violating frame")]
async fn given_step_script_query_short_violation(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_corrupted_in_window(special_frame(SpecialCommand::QueryShortAddress));
}

// COMM-095
#[given("an expert step script for query short address answering mask")]
async fn given_step_script_query_short_mask(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(
        special_frame(SpecialCommand::QueryShortAddress),
        Some(0xFF),
    );
}

// COMM-094 COMM-095
#[then(regex = r#"^the commissioning step result answer should be "([a-z]+)"$"#)]
async fn then_step_answer_kind(world: &mut DaliWorld, expected: String) {
    let result = step_result(world);
    assert_eq!(
        result["answer"].as_str(),
        Some(expected.as_str()),
        "query-short-address must name which of the four answers it heard, got {result}"
    );
}

fn step_result(world: &DaliWorld) -> Value {
    let resp = world.last_response().expect("last response");
    serde_json::from_slice(&resp.body).expect("step result should be JSON")
}

// COMM-082 COMM-093
#[then(regex = r"^the commissioning step result match should be (true|false)$")]
async fn then_step_match(world: &mut DaliWorld, expected: String) {
    let result = step_result(world);
    assert_eq!(
        result["match"].as_bool(),
        Some(expected == "true"),
        "compare/verify must report a typed match, got {result}"
    );
}

// COMM-089
#[then(regex = r"^the commissioning step result short_address should be (\d+)$")]
async fn then_step_short_address(world: &mut DaliWorld, expected: u64) {
    let result = step_result(world);
    assert_eq!(
        result["short_address"].as_u64(),
        Some(expected),
        "query-short-address must decode the wire form, got {result}"
    );
}

// COMM-091
#[given("an expert step script for query short address answering nothing")]
async fn given_step_script_query_silent(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::QueryShortAddress), None);
}

// COMM-091 COMM-094 COMM-095
#[then("the commissioning step result short_address should be absent")]
async fn then_step_short_address_absent(world: &mut DaliWorld) {
    let result = step_result(world);
    assert!(
        result["short_address"].is_null(),
        "silence must not be reported as a short address, got {result}"
    );
}

// ADP-023 ADP-026 ADP-027
#[then("the DALI mock transport frame log should be cleared")]
async fn then_clear_mock_frame_log(world: &mut DaliWorld) {
    world.dali_mock().lock().expect("mock lock").clear();
}
