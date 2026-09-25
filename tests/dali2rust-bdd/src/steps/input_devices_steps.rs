use std::time::Duration;

use cucumber::{then, when};
use dali2rust_platform::dali::TransferOutcome;
use dali2rust_platform::dali::ObservedRawFrameKind;
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::physical_devices_steps::fetch_json;
use crate::steps::wire::{priorities_of, priority_of_settle_us, RELEASE};
use crate::DaliWorld;

const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(2);

fn parse_hex_byte(text: &str) -> u8 {
    u8::from_str_radix(text, 16).unwrap_or_else(|_| panic!("not a hex byte: {text}"))
}

fn translator_counter(world: &DaliWorld, name: &str) -> Option<u64> {
    fetch_json(world.server_port(), "/api/v1/diagnostics")?
        .pointer(&format!("/sniffer_translator/{name}"))
        .and_then(Value::as_u64)
}

// INP-001 INP-002 INP-003 INP-004 INP-005 RULE-020 RULE-023
#[when(regex = r"^a 24-bit input event frame ([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2}) is observed on the bus$")]
async fn when_forward24_observed(world: &mut DaliWorld, b0: String, b1: String, b2: String) {
    let bytes = [parse_hex_byte(&b0), parse_hex_byte(&b1), parse_hex_byte(&b2)];
    let injected = world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .inject_observed_frame(bytes, ObservedRawFrameKind::Forward24);
    assert!(injected, "sniffer seam not attached or channel full");
}

// INP-001 INP-002 INP-003 INP-004 INP-005 RULE-020
#[then(regex = r"^diagnostics sniffer_translator ([a-z_0-9]+) should be at least (\d+)$")]
async fn then_translator_counter_at_least(world: &mut DaliWorld, name: String, expected: u64) {
    wait_until(
        || translator_counter(world, &name).is_some_and(|value| value >= expected),
        TRANSLATE_TIMEOUT,
    );
}

// INP-001 INP-003 INP-004
#[then(regex = r"^diagnostics sniffer_translator ([a-z_0-9]+) should be (\d+)$")]
async fn then_translator_counter_is(world: &mut DaliWorld, name: String, expected: u64) {
    let actual = translator_counter(world, &name);
    assert_eq!(
        actual,
        Some(expected),
        "sniffer_translator.{name} should be {expected}"
    );
}

// INP-005 RULE-021 RULE-023
#[then("the mock transport should have sent no frames")]
async fn then_no_frames_sent(world: &mut DaliWorld) {
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    assert!(
        guard.sent_frames().is_empty(),
        "the translator publishes facts, never commands: an observed event must \
         not put a frame on the wire"
    );
    assert!(
        guard.sent_frames24().is_empty(),
        "and that includes 24-bit frames"
    );
}

use cucumber::given;
use crate::steps::physical_devices_steps::wait_for_operation_status;

const PROJECT_TIMEOUT: Duration = Duration::from_secs(3);

fn seed_scan_with_feedback(
    world: &mut DaliWorld,
    types: &str,
    probe_opcode: u8,
    capability: u8,
    colour_capability: Option<u8>,
) {
    let types: Vec<u8> = types
        .split(',')
        .map(|t| t.trim().parse().expect("instance type byte"))
        .collect();
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    guard.enqueue_response(u8::try_from(types.len()).expect("count"));
    for t in &types {
        guard.enqueue_response(*t);
    }
    for instance in 0..u8::try_from(types.len()).expect("count") {
        let feature = 0x20 | instance;
        guard.script_frame24_answer([0x01, feature, probe_opcode], capability);
        if let Some(colour) = colour_capability {
            guard.script_frame24_answer([0x01, feature, 0x46], colour);
        }
    }
}

// INP-018
#[given(regex = r#"^the mock bus answers a control-device scan with a device at address 0 holding instance types "([0-9,]+)" declaring capabilities ([0-9A-Fa-f]{2}) status ([0-9A-Fa-f]{2}) version ([0-9A-Fa-f]{2})$"#)]
async fn given_scan_with_declarations(
    world: &mut DaliWorld,
    types: String,
    capabilities: String,
    status: String,
    version: String,
) {
    let types: Vec<u8> = types
        .split(',')
        .map(|t| t.trim().parse().expect("instance type byte"))
        .collect();
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    guard.enqueue_response(u8::try_from(types.len()).expect("count"));
    for t in &types {
        guard.enqueue_response(*t);
    }
    guard.script_frame24_answer([0x01, 0xFE, 0x46], parse_hex_byte(&capabilities));
    guard.script_frame24_answer([0x01, 0xFE, 0x30], parse_hex_byte(&status));
    guard.script_frame24_answer([0x01, 0xFE, 0x34], parse_hex_byte(&version));
}

// INP-070 INP-071 INP-072 INP-079 INP-080 INP-081
#[given(regex = r#"^the mock bus answers a control-device scan with a device at address 0 holding instance types "([0-9,]+)" and a corrected-map feedback answering capability ([0-9A-Fa-f]{2}) and colour capability ([0-9A-Fa-f]{2})$"#)]
async fn given_scan_with_corrected_feedback(
    world: &mut DaliWorld,
    types: String,
    capability: String,
    colour: String,
) {
    seed_scan_with_feedback(
        world,
        &types,
        0x4F,
        parse_hex_byte(&capability),
        Some(parse_hex_byte(&colour)),
    );
}

// INP-073
#[given(regex = r#"^the mock bus answers a control-device scan with a device at address 0 holding instance types "([0-9,]+)" and an ed1-map feedback answering capability ([0-9A-Fa-f]{2})$"#)]
async fn given_scan_with_ed1_feedback(world: &mut DaliWorld, types: String, capability: String) {
    seed_scan_with_feedback(world, &types, 0x2F, parse_hex_byte(&capability), None);
}

// INP-019 INP-070 INP-071 INP-073 INP-076 INP-077 INP-079 INP-080 INP-081 RULE-020 RULE-023
#[when(regex = r#"^the mock bus answers 24-bit query "([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2})" with "([0-9A-Fa-f]{2})"$"#)]
#[given(regex = r#"^the mock bus answers 24-bit query "([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2}) ([0-9A-Fa-f]{2})" with "([0-9A-Fa-f]{2})"$"#)]
async fn given_frame24_answer(
    world: &mut DaliWorld,
    b0: String,
    b1: String,
    b2: String,
    answer: String,
) {
    world.dali_mock().lock().expect("mock lock").script_frame24_answer(
        [
            parse_hex_byte(&b0),
            parse_hex_byte(&b1),
            parse_hex_byte(&b2),
        ],
        parse_hex_byte(&answer),
    );
}

// INP-019 INP-070 INP-071 INP-073 INP-076 INP-079
#[when(regex = r#"^I PATCH JSON (.+?) to "([^"]+)" and the operation succeeds$"#)]
async fn when_patch_and_operation_succeeds(world: &mut DaliWorld, body: String, path: String) {
    let before = fetch_json(world.server_port(), &detail_path_of(&path)).map(|v| v.to_string());
    world.send_http_request("PATCH", &path, Some(body.as_bytes()), "application/json");
    let status = world.last_response().expect("no response").status;
    assert_eq!(status, 202, "a wire-bound PATCH answers 202 + operation");
    wait_for_operation_status(world, "succeeded");
    let port = world.server_port();
    let detail = detail_path_of(&path);
    wait_until(
        || fetch_json(port, &detail).map(|v| v.to_string()) != before,
        PROJECT_TIMEOUT,
    );
}

fn detail_path_of(patch_path: &str) -> String {
    match patch_path.find("/instances/") {
        Some(cut) => patch_path[..cut].to_string(),
        None => patch_path.to_string(),
    }
}

// INP-019 INP-070 INP-073 INP-076
#[then(regex = r#"^the mock transport 24-bit trace should be exactly "([0-9A-Fa-f ,]+)"$"#)]
async fn then_frames24_trace_exactly(world: &mut DaliWorld, trace: String) {
    let expected = parse_frames24(&trace);
    let sent = world.dali_mock().lock().expect("mock lock").sent_frames24();
    assert_eq!(
        sent, expected,
        "the wire order IS the contract: DTR proof, send-twice pair, read-back"
    );
}

fn parse_frames24(trace: &str) -> Vec<[u8; 3]> {
    trace
        .split(',')
        .map(|frame| {
            let bytes: Vec<u8> = frame.split_whitespace().map(parse_hex_byte).collect();
            <[u8; 3]>::try_from(bytes).expect("three bytes per frame")
        })
        .collect()
}

// INP-083
#[then(regex = r#"^the mock transport 24-bit trace should begin with "([0-9A-Fa-f ,]+)"$"#)]
async fn then_frames24_trace_begins_with(world: &mut DaliWorld, trace: String) {
    let expected = parse_frames24(&trace);
    let sent = world.dali_mock().lock().expect("mock lock").sent_frames24();
    assert!(
        sent.starts_with(&expected),
        "the trace must begin with {expected:02X?} — the frame the wire refused, sent again — got {sent:02X?}"
    );
}

// INP-083
#[given("the next 24-bit frame collides with another master's on the wire")]
async fn given_next_frame24_collides(world: &mut DaliWorld) {
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    guard.script_frame24_outcome(TransferOutcome::Collision);
}

// INP-010 INP-011 INP-013 INP-016 INP-017 INP-019 INP-030 INP-031 INP-032 INP-074 INP-075 INP-076 INP-077 INP-078 RULE-020 RULE-023 INP-083
#[given(regex = r#"^the mock bus answers a control-device scan with a device at address 0 holding instance types "([0-9,]+)"$"#)]
async fn given_scan_answers(world: &mut DaliWorld, types: String) {
    let types: Vec<u8> = types
        .split(',')
        .map(|t| t.trim().parse().expect("instance type byte"))
        .collect();
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    guard.enqueue_response(u8::try_from(types.len()).expect("count"));
    for t in &types {
        guard.enqueue_response(*t);
    }
}

// INP-010 INP-011 INP-013 INP-016 INP-017 INP-018 INP-019 INP-030 INP-031 INP-032 INP-070 INP-071 INP-072 INP-073 INP-074 INP-075 INP-076 INP-077 INP-078 INP-079 INP-080 INP-081 RULE-020 RULE-023 INP-083 INP-082
#[given("the segment answers no control-device scan at all")]
async fn given_empty_segment(world: &mut DaliWorld) {
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    guard.clear();
    guard.clear_frame24_answers();
}

// INP-006 INP-010 INP-011 INP-013 INP-016 INP-017 INP-018 INP-019 INP-030 INP-031 INP-032 INP-070 INP-071 INP-072 INP-073 INP-074 INP-075 INP-076 INP-077 INP-078 INP-079 INP-080 INP-081 INP-082 INP-083 INP-084 MQTT-025 RULE-020 RULE-023
#[when("input devices are scanned on adapter 0 and the scan succeeds")]
async fn when_scan_succeeds(world: &mut DaliWorld) {
    world.send_http_request("POST", "/api/v1/adapters/0/input-devices/scan", None, "");
    let status = world.last_response().expect("no response").status;
    assert_eq!(status, 202, "scan must answer 202 + operation");
    wait_for_operation_status(world, "succeeded");
    let port = world.server_port();
    wait_until(
        || {
            fetch_json(port, "/api/v1/adapters/0/input-devices")
                .and_then(|v| v.pointer("/input_devices/0/short_address").cloned())
                .is_some()
        },
        PROJECT_TIMEOUT,
    );
}

// INP-019 INP-030 INP-070 INP-072 INP-073 INP-076 INP-078
#[when("the mock transport 24-bit trace is cleared")]
async fn when_clear_frames24(world: &mut DaliWorld) {
    world.dali_mock().lock().expect("mock lock").clear_sent_frames24();
}

// INP-030 INP-072 INP-078 ADP-025
#[then("the mock transport should have sent no 24-bit frames")]
async fn then_no_frames24(world: &mut DaliWorld) {
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    assert!(
        guard.sent_frames24().is_empty(),
        "a request refused at ingress must not have reached the wire first: {:?}",
        guard.sent_frames24()
    );
}

// INP-084
#[then(regex = r"^the first 24-bit forward frame should be sent at priority (\d+)$")]
async fn first_frame24_priority(world: &mut DaliWorld, expected: u8) {
    let settle_us = world.dali_mock().lock().expect("mock lock").sent_frame24_settle_us();
    assert!(
        !settle_us.is_empty(),
        "no 24-bit forward frames reached the wire, so there is no priority to check"
    );
    let priorities = priorities_of(&settle_us);
    let first = priorities[0];
    assert!(
        first == expected || first == RELEASE,
        "first 24-bit frame should be priority {expected} (or a §9.2 bus release); \
         whole trace: {priorities:?}"
    );
}

// INP-084
#[then(regex = r"^24-bit forward frame (\d+) should be sent at priority (\d+)$")]
async fn frame24_priority(world: &mut DaliWorld, ordinal: usize, expected: u8) {
    let settle_us = world.dali_mock().lock().expect("mock lock").sent_frame24_settle_us();
    let index = ordinal.checked_sub(1).expect("frames are numbered from 1");
    assert!(
        index < settle_us.len(),
        "only {} 24-bit frames were sent, no frame {ordinal}",
        settle_us.len()
    );
    let priority = priority_of_settle_us(index, settle_us[index]);
    assert_eq!(
        priority, expected,
        "24-bit frame {ordinal} announced priority {priority}; whole trace: {:?}",
        priorities_of(&settle_us)
    );
}
