use std::thread;
use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_domain::dali::commands::is_query_opcode;
use serde_json::Value;

use dali2rust_api::contracts::dali::{
    DaliCommandRequestBuffer, DaliLevelRequestBuffer, DaliRawFrameRequestBuffer,
};
use dali2rust_api::contracts::BufferBytes;

use crate::steps::wire::{assert_frame_before, assert_nothing_between_frames};
use crate::DaliWorld;

// BUS-013 BUS-014 BUS-016 CONT-001 DALI-001 DALI-002 DALI-004 DALI-008 DALI-012 DALI-013 DALI-014 DALI-015 DALI-017 DALI-050 DIAG-031 SYS-001 SYS-002 SYS-003 SYS-004 SYS-005 SYS-007 SYS-011 SYS-012 SYS-017 DALI-020 DALI-021 DALI-022 DALI-030 DALI-031 DALI-032 DALI-040 DALI-051 DALI-300 DALI-301 DALI-304 STATS-003 STATS-010 WS-044
#[given(regex = r"a DALI mock transport with response (\d+)")]
async fn given_mock_transport_with_response(world: &mut DaliWorld, response: u64) {
    world.dali_mock().lock().unwrap().clear();
    world
        .dali_mock()
        .lock()
        .unwrap()
        .set_persistent_response(response as u8);
}

// DALI-003 DALI-009 DALI-016 DALI-052 DALI-053 DALI-113 GRP-030 GRP-063 GRP-070 GRP-072 OP-131 OP-133 SYS-006 SYS-018 SYS-210 SYS-211 SYS-213 SCN-083 SCN-084 SCN-085 SCN-086 SCN-087 SCN-088 DALI-023 DALI-033 DALI-041 DALI-110 DALI-111 GRP-073 REG-030 REG-031 SCN-060 SCN-062 SCN-063 SCN-065 SCN-080 SCN-081 SCN-082 SCN-089 SCN-090 SCN-091 SCN-092 SCN-093 SYS-241
#[given("a DALI mock transport with no response")]
async fn given_mock_transport_no_response(world: &mut DaliWorld) {
    world.dali_mock().lock().unwrap().clear();
}

fn json_command_body(wire_address: u8, command: u8) -> Vec<u8> {
    DaliCommandRequestBuffer::new(wire_address, command, 1).to_vec()
}

fn wire_for_address_command(address: u8, command: u8) -> u8 {
    let wire = address << 1;
    if is_query_opcode(command) {
        wire | 0x01
    } else {
        wire
    }
}

// CONT-001 DALI-001 DALI-002 DALI-003 DALI-004 DALI-008 DALI-009 DALI-012 DALI-013 DALI-014 DALI-015 DALI-016 DALI-017 SYS-003 SYS-005 SYS-006 SYS-007 SYS-011 SYS-012 SYS-017 SYS-018 BUS-013 BUS-014 BUS-016 DALI-041
#[when(regex = r"I send a JSON DALI command with address (\d+) and command (\d+)")]
async fn send_dali_command(world: &mut DaliWorld, address: u64, command: u64) {
    let command = command as u8;
    let wire = wire_for_address_command(address as u8, command);
    let body = json_command_body(wire, command as u8);
    world.send_http_request(
        "POST",
        "/api/v1/dali/command",
        Some(&body),
        "application/json",
    );
}

// DALI-050 DALI-052 DALI-053 DALI-110 DALI-111 DALI-112 DALI-113 DALI-300 DALI-301 DALI-307 DALI-040 DALI-051 DALI-200
#[when(regex = r"I send a DALI command with wire_address (\d+) and command (\d+)")]
async fn send_dali_command_wire(world: &mut DaliWorld, wire_address: u64, command: u64) {
    let body = json_command_body(wire_address as u8, command as u8);
    world.send_http_request(
        "POST",
        "/api/v1/dali/command",
        Some(&body),
        "application/json",
    );
}

// DALI-004
#[when("I send an invalid DALI command request")]
async fn send_invalid_dali_command(world: &mut DaliWorld) {
    world.send_http_request(
        "POST",
        "/api/v1/dali/command",
        Some(br#"{"bad":"json"}"#),
        "application/json",
    );
}

fn parse_json_response(world: &DaliWorld) -> Value {
    let resp = world.last_response().expect("no response");
    serde_json::from_slice(&resp.body).expect("failed to parse JSON response")
}

// BUS-014 DALI-002 DALI-003 DALI-008 DALI-009 DALI-012 DALI-013 DALI-015 DALI-016 DALI-017 DALI-052 SYS-003 SYS-005 SYS-006 SYS-007 SYS-011 SYS-017 SYS-018 DALI-040 DALI-050
#[then(regex = r"the JSON DaliCommandResponse success should be (true|false)")]
async fn dali_response_success(world: &mut DaliWorld, expected: String) {
    let val = parse_json_response(world);
    let expected_bool = expected == "true";
    assert_eq!(
        val["success"].as_bool(),
        Some(expected_bool),
        "expected success={}, got {:?}",
        expected_bool,
        val["success"]
    );
}

// DALI-015 DALI-017 DALI-307 DALI-040
#[then(regex = r"the JSON DaliCommandResponse backward_frame should be (\d+)")]
async fn dali_response_backward_frame(world: &mut DaliWorld, expected: u64) {
    let val = parse_json_response(world);
    assert_eq!(
        val["backward_frame"].as_u64(),
        Some(expected),
        "backward_frame mismatch"
    );
}

// GRP-073
#[then(regex = r"the DALI mock transport should have sent forward frame 0x([0-9a-fA-F]+)$")]
async fn dali_mock_sent_frame(world: &mut DaliWorld, hex: String) {
    let wanted = u16::from_str_radix(&hex, 16).expect("hex frame");
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    assert!(
        frames.contains(&wanted),
        "expected forward frame 0x{wanted:04X} on the wire, got {frames:04X?}"
    );
}

// DALI-050
#[then(
    regex = r"the DALI mock transport should have sent forward frame 0x([0-9a-fA-F]+) before 0x([0-9a-fA-F]+)"
)]
async fn dali_mock_two_frames_order(world: &mut DaliWorld, first_hex: String, second_hex: String) {
    let first = u16::from_str_radix(&first_hex, 16).expect("first hex frame");
    let second = u16::from_str_radix(&second_hex, 16).expect("second hex frame");
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    assert_frame_before(&frames, first, second);
}

// DALI-051 DALI-052 DALI-053 DALI-113
#[then(
    regex = r"forward frame 0x([0-9a-fA-F]+) should be immediately followed by 0x([0-9a-fA-F]+)"
)]
async fn dali_mock_frames_adjacent(world: &mut DaliWorld, first_hex: String, second_hex: String) {
    let first = u16::from_str_radix(&first_hex, 16).expect("first hex frame");
    let second = u16::from_str_radix(&second_hex, 16).expect("second hex frame");
    let trace = world.dali_mock().lock().unwrap().bus_trace();
    assert_nothing_between_frames(&trace, first, second);
}

// ADP-023 ADP-024 BUS-013 BUS-014 BUS-016 DALI-012 DALI-013 DALI-014 DALI-020 DALI-023 DALI-030 DALI-033 DALI-040 DALI-041 DALI-052 DALI-053 DALI-110 DALI-111 DALI-113 DALI-300 DALI-302 DALI-303 DALI-304 GRP-001 GRP-020 GRP-030 GRP-061 GRP-063 GRP-072 OP-100 PD-032 PD-033 PD-042 PD-043 PD-182 PD-187 PD-244 PD-255 SCN-001 SCN-010 SCN-020 SCN-030 SCN-040 SCN-041 SCN-046 SCN-050 SCN-061 SCN-063 SYS-003 SYS-005 SYS-006 SYS-007 SYS-212 SYS-213 SYS-214 SYS-215 VL-001 VL-010 WS-001 ADP-025 ADP-026
#[then(regex = r"the DALI mock transport should have received (\d+) forward frame")]
async fn dali_mock_forward_frames(world: &mut DaliWorld, expected: u64) {
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    assert_eq!(
        frames.len(),
        expected as usize,
        "expected {} forward frames, got {}",
        expected,
        frames.len()
    );
}

// ADP-001 DALI-001 INP-075 RULE-003 RULE-005 RULE-007 RULE-008 RULE-009
#[then(regex = r#"the response body should contain "([^"]+)""#)]
async fn response_body_contains(world: &mut DaliWorld, expected: String) {
    let resp = world.last_response().expect("no response");
    let body = String::from_utf8_lossy(&resp.body);
    assert!(
        body.contains(&expected),
        "response body should contain '{}', got: {}",
        expected,
        body
    );
}

// DALI-112
#[given("a DALI mock transport that records timestamps")]
async fn given_timestamp_transport(world: &mut DaliWorld) {
    world.dali_mock().lock().unwrap().clear();
}

// DALI-112
#[then(regex = r"forward frames (\d+) and (\d+) should be at least (\d+) milliseconds apart")]
async fn then_frames_apart(world: &mut DaliWorld, a: usize, b: usize, min_ms: u128) {
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let gap_ms = world
        .dali_mock()
        .lock()
        .unwrap()
        .forward_frame_gap_ms(a, b)
        .unwrap_or_else(|| {
            panic!("no send timestamps for forward frames {a} and {b}; trace: {frames:?}")
        });
    assert!(
        gap_ms >= min_ms,
        "expected at least {min_ms} ms between forward frames {a} and {b}, got {gap_ms} ms \
         (frames: {frames:?})"
    );
}

// DALI-200 PD-038 COMM-090
#[given(regex = r"^a bus with confirmation timeout of (\d+) milliseconds$")]
async fn given_confirmation_timeout(world: &mut DaliWorld, timeout_ms: u64) {
    use dali2rust_bus::BusConfig;
    let config = BusConfig {
        confirmation_timeout_ms: timeout_ms,
        ..BusConfig::default()
    };
    world.restart_server_with_config(config);
}

// DALI-200 PD-038
#[then(regex = r"^the response arrives within (\d+) milliseconds$")]
async fn then_response_arrives_within(world: &mut DaliWorld, max_ms: u64) {
    assert!(
        world.last_response().is_some(),
        "expected a response but none was received"
    );
    let elapsed = world
        .last_request_elapsed()
        .expect("request timing was not recorded");
    assert!(
        elapsed <= Duration::from_millis(max_ms),
        "response took {} ms, expected within {} ms",
        elapsed.as_millis(),
        max_ms
    );
}

// DALI-001 DALI-200 PD-038 SYS-050 COMM-090 SCN-065 SCN-082
#[given("the DALI transport blocks indefinitely")]
async fn given_transport_blocks(world: &mut DaliWorld) {
    world.dali_mock().lock().unwrap().block_next_send();
}

// SYS-230 SYS-231 SYS-232 SYS-233 SYS-234
#[given(regex = r"^the DALI transport parks the next exchange for (\d+) milliseconds$")]
async fn given_transport_parks_for(world: &mut DaliWorld, ms: u64) {
    world.dali_mock().lock().unwrap().block_next_send_for(ms);
}

// SYS-050 SCN-065
#[when("the DALI transport unblocks")]
async fn when_transport_unblocks(world: &mut DaliWorld) {
    world.release_held_send();
}

// DALI-020 DALI-021 DALI-022 DALI-023
#[when(regex = r"I send a JSON level command with wire_address (\d+) and level (\d+)")]
async fn send_json_level_command(world: &mut DaliWorld, wire_address: u64, level: u64) {
    let body = DaliLevelRequestBuffer::new(wire_address as u8, level as u8).to_vec();
    world.send_http_request(
        "POST",
        "/api/v1/dali/level",
        Some(&body),
        "application/json",
    );
}

// DALI-020 DALI-021 DALI-030 DALI-031 DALI-307
#[then(regex = r"the JSON response success should be (true|false)")]
async fn json_response_success(world: &mut DaliWorld, expected: String) {
    let val = parse_json_response(world);
    let expected_bool = expected == "true";
    assert_eq!(
        val["success"].as_bool(),
        Some(expected_bool),
        "expected JSON success={}, got {:?}",
        expected_bool,
        val["success"]
    );
}

// DALI-030 DALI-031 DALI-032 DALI-033 DIAG-031 STATS-003 STATS-010 WS-044
#[when(regex = r"I send a JSON raw command with frame (\d+) and expects_backward (true|false)")]
async fn send_json_raw_command(world: &mut DaliWorld, frame: u64, expects_backward: String) {
    let expects = expects_backward == "true";
    let body = DaliRawFrameRequestBuffer::new(frame as u16, expects).to_vec();
    world.send_http_request("POST", "/api/v1/dali/raw", Some(&body), "application/json");
}

// DALI-305 DALI-306 DALI-307
#[given(regex = r"the DALI transport responds with 0x([0-9a-fA-F]+)")]
async fn given_transport_responds_hex(world: &mut DaliWorld, hex: String) {
    let val = u8::from_str_radix(&hex, 16).expect("valid hex byte");
    world.dali_mock().lock().unwrap().clear();
    world
        .dali_mock()
        .lock()
        .unwrap()
        .set_persistent_response(val);
}

// DALI-304
#[when("I send a DALI command with missing fields")]
async fn when_send_command_missing_fields(world: &mut DaliWorld) {
    world.send_http_request(
        "POST",
        "/api/v1/dali/command",
        Some(br#"{"wire_address":2}"#),
        "application/json",
    );
}

// SYS-050
#[when("I send a DALI command in background")]
async fn when_send_dali_command_background(world: &mut DaliWorld) {
    let body = DaliCommandRequestBuffer::new(2, 254, 1).to_vec();
    world.send_http_request_background(
        "POST",
        "/api/v1/dali/command",
        Some(&body),
        "application/json",
    );
}

// DALI-302 POLICY-010 POLICY-011
#[when(regex = r#"I send a POST request to "([^"]+)" with empty body"#)]
async fn when_send_post_empty_body(world: &mut DaliWorld, path: String) {
    world.send_http_request("POST", &path, Some(b""), "");
}

// DALI-301
#[then("the response content indicates failure")]
async fn then_response_indicates_failure(world: &mut DaliWorld) {
    let resp = world.last_response().expect("no response");
    let val: serde_json::Value = serde_json::from_slice(&resp.body).expect("parse JSON");
    let failure = match val.get("success").and_then(|v| v.as_bool()) {
        Some(false) => true,
        _ => val.get("error").and_then(|v| v.as_str()).is_some(),
    };
    assert!(failure, "expected failure indication, got {:?}", val);
}

// DALI-305
#[when(regex = r#"I send two POST requests to "([^"]+)" concurrently"#)]
async fn when_send_two_concurrent(world: &mut DaliWorld, path: String) {
    world.stored_responses.clear();
    let port = world.server_port;
    let p1 = path.clone();
    let body1 = DaliCommandRequestBuffer::new(3, 144, 1).to_vec();
    let body2 = DaliCommandRequestBuffer::new(3, 145, 1).to_vec();
    let h1 = thread::spawn(move || {
        DaliWorld::send_http_request_raw(port, "POST", &p1, Some(&body1), "application/json")
    });
    let h2 = thread::spawn(move || {
        DaliWorld::send_http_request_raw(port, "POST", &path, Some(&body2), "application/json")
    });
    world.stored_responses.push(h1.join().expect("thread 1"));
    world.stored_responses.push(h2.join().expect("thread 2"));
}

// DALI-305
#[then("both responses have status 200")]
async fn then_both_status_200(world: &mut DaliWorld) {
    assert_eq!(world.stored_responses.len(), 2);
    for (i, r) in world.stored_responses.iter().enumerate() {
        assert_eq!(
            r.status, 200,
            "response {}: expected 200, got {}",
            i, r.status
        );
    }
}

// DALI-305
#[then("both responses contain backward_frame 0x42")]
async fn then_both_backward_0x42(world: &mut DaliWorld) {
    assert_eq!(world.stored_responses.len(), 2);
    for (i, r) in world.stored_responses.iter().enumerate() {
        let val: serde_json::Value = serde_json::from_slice(&r.body)
            .unwrap_or_else(|_| panic!("response {}: parse JSON", i));
        assert_eq!(
            val["backward_frame"].as_u64(),
            Some(0x42),
            "response {}: expected backward_frame 0x42, got {:?}",
            i,
            val["backward_frame"]
        );
    }
}

// DALI-306
#[when("I send 5 DALI commands in sequence without delay")]
async fn when_send_5_commands(world: &mut DaliWorld) {
    world.stored_responses.clear();
    let body = DaliCommandRequestBuffer::new(2, 200, 1).to_vec();
    for _ in 0..5 {
        let resp = DaliWorld::send_http_request_raw(
            world.server_port,
            "POST",
            "/api/v1/dali/command",
            Some(&body),
            "application/json",
        );
        world.stored_responses.push(resp);
    }
}

// DALI-306
#[then("all 5 responses have status 200")]
async fn then_all_5_status_200(world: &mut DaliWorld) {
    assert_eq!(world.stored_responses.len(), 5);
    for (i, r) in world.stored_responses.iter().enumerate() {
        assert_eq!(
            r.status, 200,
            "response {}: expected 200, got {}",
            i, r.status
        );
    }
}

fn diagnostics_snapshot(world: &mut DaliWorld) -> Value {
    super::get_json(world, "/api/v1/diagnostics")
}

// DIAG-030
#[then("the JSON response should have the diagnostics blocks")]
async fn diagnostics_blocks_present(world: &mut DaliWorld) {
    let resp = world.last_response().expect("no response");
    let val: Value = serde_json::from_slice(&resp.body).expect("JSON body");
    for block in [
        "uptime_ms",
        "bus",
        "confirmation_bridge",
        "dali_worker",
        "dali_wire",
        "sniffer_translator",
        "rules",
        "projector",
        "apply_orchestrator",
        "operations",
        "registry",
        "persistence",
        "phy_sniffer",
        "hcl",
        "poller",
        "websocket",
        "mqtt",
        "redundancy",
    ] {
        assert!(val.get(block).is_some(), "missing diagnostics block {block}");
    }
    assert!(
        val["bus"]["commands"]["publish_queued"].is_u64(),
        "bus.commands.publish_queued must be a number"
    );
}

// DIAG-033
#[then("the diagnostics wire block should carry the occupancy figures")]
async fn diagnostics_wire_occupancy_present(world: &mut DaliWorld) {
    let resp = world.last_response().expect("no response");
    let val: Value = serde_json::from_slice(&resp.body).expect("JSON body");
    let wire = &val["dali_wire"];
    for tick in ["wire_ticks_total", "wire_ticks_active", "wire_ticks_tx"] {
        assert!(wire[tick].is_u64(), "dali_wire.{tick} must be a number: {wire}");
    }
    let load = wire["load_permille"].as_u64().expect("load_permille");
    let own = wire["load_own_permille"].as_u64().expect("load_own_permille");
    assert!(load <= 1000, "load_permille out of range: {load}");
    assert!(own <= load, "own share {own} exceeds total load {load}");
}

// DIAG-031
#[given("I remember the diagnostics bus commands queued total")]
async fn remember_bus_commands_queued(world: &mut DaliWorld) {
    let snapshot = diagnostics_snapshot(world);
    world.remembered_u64 = snapshot["bus"]["commands"]["publish_queued"].as_u64();
    assert!(world.remembered_u64.is_some(), "publish_queued missing");
}

// DIAG-031
#[then("the diagnostics bus commands queued total should have increased")]
async fn bus_commands_queued_increased(world: &mut DaliWorld) {
    let before = world.remembered_u64.expect("remembered queued total");
    let snapshot = diagnostics_snapshot(world);
    let now = snapshot["bus"]["commands"]["publish_queued"]
        .as_u64()
        .expect("publish_queued");
    assert!(
        now > before,
        "expected bus.commands.publish_queued to grow: before={before} now={now}"
    );
}

// PERS-005
#[then(regex = r"^the diagnostics persistence flush total should settle at exactly (\d+)$")]
async fn diagnostics_flush_total_settles(world: &mut DaliWorld, expected: u64) {
    let mut last: Option<u64> = None;
    let mut settled: u64 = 0;
    dali2rust_test_support::wait_until(
        || {
            let now = diagnostics_snapshot(world)["persistence"]["flush_success_total"].as_u64();
            let done = now.is_some() && now == last;
            last = now;
            if done {
                settled = now.expect("settled total");
            }
            done
        },
        Duration::from_secs(2),
    );
    assert_eq!(
        settled, expected,
        "flush_success_total settled at the wrong count"
    );
}

// DIAG-032
#[given("I remember the diagnostics uptime")]
async fn remember_diagnostics_uptime(world: &mut DaliWorld) {
    let snapshot = diagnostics_snapshot(world);
    world.remembered_u64 = snapshot["uptime_ms"].as_u64();
    assert!(world.remembered_u64.is_some(), "uptime_ms missing");
}

// DIAG-032
#[then("the diagnostics uptime should not decrease")]
async fn diagnostics_uptime_monotonic(world: &mut DaliWorld) {
    let before = world.remembered_u64.expect("remembered uptime");
    let resp = world.last_response().expect("no response");
    let val: Value = serde_json::from_slice(&resp.body).expect("JSON body");
    let now = val["uptime_ms"].as_u64().expect("uptime_ms");
    assert!(
        now >= before,
        "uptime went backwards: before={before} now={now}"
    );
}
