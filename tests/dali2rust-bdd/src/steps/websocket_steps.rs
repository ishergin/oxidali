use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_test_support::remains_false_for;
use serde_json::Value;

use crate::ws_client::WsTestClient;
use crate::DaliWorld;

const V1_CHANNELS: &[&str] = &[
    "adapters",
    "physical_devices",
    "virtual_lamps",
    "groups",
    "scenes",
    "operations",
    "stats",
    "diagnostics",
    "sniffer",
    "input",
    "rules",
    "logs",
];

const COUNTER_SETTLE: Duration = Duration::from_secs(3);

const SNIFFER_QUIET_WINDOW: Duration = Duration::from_millis(600);

const LOG_QUIET_WINDOW: Duration = Duration::from_millis(900);


fn open_client(world: &mut DaliWorld) {
    let client = WsTestClient::connect(world.server_port());
    world.ws_clients.push(client);
}

fn client(world: &mut DaliWorld, index: usize) -> &mut WsTestClient {
    world
        .ws_clients
        .get_mut(index)
        .expect("no WebSocket client for this step")
}

fn last_client(world: &mut DaliWorld) -> &mut WsTestClient {
    let index = world.ws_clients.len().saturating_sub(1);
    client(world, index)
}

fn channel_list(value: &Value) -> Vec<String> {
    value
        .get("channels")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn diagnostics_websocket(world: &mut DaliWorld) -> Value {
    super::get_json(world, "/api/v1/diagnostics")["websocket"].clone()
}

fn websocket_counter(world: &mut DaliWorld, name: &str) -> u64 {
    diagnostics_websocket(world)[name]
        .as_u64()
        .unwrap_or_else(|| panic!("diagnostics.websocket.{name} missing"))
}

fn wait_for_counter(world: &mut DaliWorld, name: &str, check: impl Fn(u64) -> bool) -> u64 {
    remains_false_for(|| check(websocket_counter(world, name)), COUNTER_SETTLE);
    websocket_counter(world, name)
}

// WS-001 WS-007 WS-011 WS-012 WS-031
#[when("I open a WebSocket connection")]
async fn when_open_ws(world: &mut DaliWorld) {
    open_client(world);
}

// WS-002 WS-003 WS-004 WS-005 WS-006 WS-008 WS-009 WS-010 WS-013 WS-030 WS-040 WS-041 WS-042 WS-043 WS-044 WS-045 WS-046
#[given("an open WebSocket connection")]
async fn given_open_ws(world: &mut DaliWorld) {
    open_client(world);
    last_client(world)
        .wait_for_op("hello")
        .expect("hello frame");
}

// WS-011 WS-031
#[given(regex = r"^(\d+) open WebSocket connections$")]
async fn given_n_open_ws(world: &mut DaliWorld, count: usize) {
    for _ in 0..count {
        open_client(world);
        last_client(world)
            .wait_for_op("hello")
            .expect("hello frame");
    }
}

// WS-032
#[given(regex = r#"^(\d+) open WebSocket connections subscribed to "([^"]+)"$"#)]
async fn given_n_open_ws_subscribed(world: &mut DaliWorld, count: usize, channels: String) {
    for _ in 0..count {
        open_client(world);
        let c = last_client(world);
        c.wait_for_op("hello").expect("hello frame");
        c.subscribe(&channels);
        c.wait_for_op("subscribed").expect("subscribed frame");
    }
}

// WS-012 WS-013
#[when("the WebSocket client sends a ping")]
async fn when_ws_ping(world: &mut DaliWorld) {
    let answered = last_client(world).ping_answered(b"hil");
    world.remembered_flag = Some(answered);
}

// WS-012 WS-013
#[then("the WebSocket client should receive a pong")]
async fn then_ws_pong(world: &mut DaliWorld) {
    assert!(
        world.remembered_flag.unwrap_or(false),
        "no pong within the frame timeout"
    );
}

// WS-001
#[then("the WebSocket connection should be established")]
async fn then_ws_established(world: &mut DaliWorld) {
    let frame = last_client(world).first_frame().expect("first frame");
    assert_eq!(frame["op"], "hello", "unexpected first frame: {frame}");
}

// WS-007 WS-031
#[then(regex = r#"^the first WebSocket frame should have op "([^"]+)"$"#)]
async fn then_first_frame_op(world: &mut DaliWorld, op: String) {
    let frame = last_client(world).first_frame().expect("first frame");
    assert_eq!(frame["op"], op.as_str(), "frame was {frame}");
    world.remembered_json = Some(frame);
}

// WS-007
#[then(regex = r"^the hello frame should advertise protocol (\d+)$")]
async fn then_hello_protocol(world: &mut DaliWorld, protocol: u64) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    assert_eq!(frame["protocol"].as_u64(), Some(protocol));
}

// WS-007
#[then("the hello frame should list exactly the v1 channels")]
async fn then_hello_channels(world: &mut DaliWorld) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    assert_eq!(channel_list(&frame), V1_CHANNELS);
}

// WS-002 WS-006 WS-008 WS-009 WS-041 WS-042 WS-043 WS-044
#[when(regex = r#"^the WebSocket client subscribes to "([^"]+)"$"#)]
async fn when_subscribe(world: &mut DaliWorld, channels: String) {
    last_client(world).subscribe(&channels);
}

// WS-003 WS-004 WS-005 WS-006 WS-008 WS-009 WS-030 WS-040 WS-041 WS-042 WS-043 WS-044 WS-045 WS-046 WS-010 WS-013
#[given(regex = r#"^the WebSocket client is subscribed to "([^"]+)"$"#)]
async fn given_subscribed(world: &mut DaliWorld, channels: String) {
    let c = last_client(world);
    c.subscribe(&channels);
    c.wait_for_op("subscribed").expect("subscribed frame");
}

// WS-005 WS-045
#[when(regex = r#"^the WebSocket client unsubscribes from "([^"]+)"$"#)]
async fn when_unsubscribe(world: &mut DaliWorld, channels: String) {
    last_client(world).unsubscribe(&channels);
}

// WS-002 WS-005 WS-006 WS-008 WS-009 WS-011 WS-045
#[then(regex = r#"^the WebSocket client should receive op "([^"]+)"$"#)]
async fn then_receive_op(world: &mut DaliWorld, op: String) {
    let frame = last_client(world)
        .wait_for_op(&op)
        .unwrap_or_else(|| panic!("no {op} frame arrived"));
    world.remembered_json = Some(frame);
}

// WS-002 WS-005 WS-006 WS-008 WS-009
#[then(regex = r#"^the acknowledged channels should be "([^"]*)"$"#)]
async fn then_ack_channels(world: &mut DaliWorld, expected: String) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    let mut got = channel_list(&frame);
    let mut want: Vec<String> = expected
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "frame was {frame}");
}

// WS-058
#[when(regex = r#"^I open a WebSocket connection with Origin "([^"]+)"$"#)]
async fn when_open_with_origin(world: &mut DaliWorld, origin: String) {
    let client = WsTestClient::connect_with_origin(world.server_port(), &origin);
    world.ws_clients.push(client);
}

// WS-058
#[then(regex = r"^the WebSocket connection should be closed with code (\d+)$")]
async fn then_closed_with_code(world: &mut DaliWorld, code: u16) {
    let got = last_client(world).wait_for_close_code();
    assert_eq!(got, Some(code), "the refusal must name itself in the CLOSE frame");
}

// WS-008 WS-011
#[then(regex = r#"^the WebSocket error code should be "([^"]+)"$"#)]
async fn then_error_code(world: &mut DaliWorld, code: String) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    assert_eq!(frame["error"]["code"], code.as_str(), "frame was {frame}");
}

// WS-003 WS-010 WS-030 WS-046 WS-013
#[then(regex = r#"^the WebSocket client should receive a "([^"]+)" frame on channel "([^"]+)"$"#)]
async fn then_receive_event(world: &mut DaliWorld, event_type: String, channel: String) {
    let frame = last_client(world)
        .wait_for_event(&event_type, &channel)
        .unwrap_or_else(|| panic!("no {event_type} frame on {channel}"));
    world.remembered_json = Some(frame);
}

// WS-032
#[then(regex = r#"^WebSocket client (\d+) should receive a "([^"]+)" frame on channel "([^"]+)"$"#)]
async fn then_client_n_receives(
    world: &mut DaliWorld,
    index: usize,
    event_type: String,
    channel: String,
) {
    let frame = client(world, index - 1)
        .wait_for_event(&event_type, &channel)
        .unwrap_or_else(|| panic!("client {index} received no {event_type} on {channel}"));
    world.remembered_json = Some(frame);
}

// WS-032
#[given(regex = r"^WebSocket client (\d+) stops reading its socket$")]
async fn given_client_stops_reading(world: &mut DaliWorld, index: usize) {
    client(world, index - 1).stop_reading();
}

// WS-004 WS-046
#[then(regex = r"^the WebSocket client should receive no frame within (\d+) ms$")]
async fn then_no_frame(world: &mut DaliWorld, millis: u64) {
    let quiet = last_client(world).expect_silence(Duration::from_millis(millis));
    assert!(quiet, "an unsubscribed channel delivered a frame");
}

// WS-003
#[then("the runtime state payload should expand the light setpoint inline")]
async fn then_setpoint_inline(world: &mut DaliWorld) {
    let state = remembered_state(world);
    for field in [
        "power",
        "level",
        "color_mode",
        "color_temperature_kelvin",
        "xy",
        "rgb",
    ] {
        assert!(state.get(field).is_some(), "missing {field} in {state}");
    }
    assert_eq!(state["power"], "on");
    assert_eq!(state["level"], 180);
}

// WS-003
#[then("the runtime state payload should expand the runtime observation inline")]
async fn then_observation_inline(world: &mut DaliWorld) {
    let state = remembered_state(world);
    for field in [
        "status",
        "failure_status",
        "value_source",
        "last_seen_ms",
        "last_dapc_source",
        "error",
    ] {
        assert!(state.get(field).is_some(), "missing {field} in {state}");
    }
}

fn remembered_state(world: &DaliWorld) -> Value {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    frame["payload"]["state"].clone()
}

// INP-006
#[then(regex = r#"^the input event payload should name the event "([^"]+)"$"#)]
async fn then_input_event_named(world: &mut DaliWorld, want: String) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    let name = frame["payload"]["event"].as_str();
    assert_eq!(
        name,
        Some(want.as_str()),
        "an unnamed row is the symptom, not a cosmetic gap: {frame}"
    );
}

// WS-010
#[then("the operation payload should carry a string operation_id")]
async fn then_operation_id_is_string(world: &mut DaliWorld) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    let id = frame["payload"]["operation_id"].as_str();
    assert!(id.is_some_and(|s| !s.is_empty()), "frame was {frame}");
}

// WS-010
#[then("the operation payload should carry snake_case type and status")]
async fn then_operation_snake_case(world: &mut DaliWorld) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    let payload = &frame["payload"];
    for key in ["type", "status"] {
        let value = payload[key].as_str().unwrap_or_default();
        assert!(!value.is_empty(), "missing {key} in {frame}");
        assert_eq!(
            value,
            value.to_lowercase(),
            "{key} is not snake_case: {value}"
        );
    }
    assert_eq!(payload["type"], "discovery");
}

// WS-010
#[then("the operation payload should not carry a correlation_id")]
async fn then_no_correlation_id(world: &mut DaliWorld) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    assert!(
        frame["payload"].get("correlation_id").is_none(),
        "frame was {frame}"
    );
}

// WS-041 WS-042 WS-043 WS-044
#[then("the WebSocket client should receive a sniffer batch")]
async fn then_receive_sniffer_batch(world: &mut DaliWorld) {
    let frame = last_client(world)
        .wait_for(|v| {
            v.get("type").and_then(Value::as_str) == Some("SnifferBatch")
                && v["payload"]["frames"]
                    .as_array()
                    .is_some_and(|list| !list.is_empty())
        })
        .expect("no non-empty sniffer batch arrived");
    world.remembered_json = Some(frame);
}

fn batch_frames(world: &DaliWorld) -> Vec<Value> {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    frame["payload"]["frames"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

// WS-041 WS-042 WS-043
#[then(regex = r#"^the sniffer batch should contain a "([^"]+)" frame named "([^"]+)"$"#)]
async fn then_batch_has_named_frame(world: &mut DaliWorld, direction: String, name: String) {
    let frames = batch_frames(world);
    assert!(
        frames
            .iter()
            .any(|f| f["dir"] == direction.as_str() && f["name"] == name.as_str()),
        "no {direction} frame named {name} in {frames:?}"
    );
}

// WS-042
#[then(regex = r"^the sniffer batch should contain a frame targeting short address (\d+)$")]
async fn then_batch_targets_short(world: &mut DaliWorld, short: u64) {
    let frames = batch_frames(world);
    assert!(
        frames.iter().any(|f| f["target"]["kind"] == "short"
            && f["target"]["id"].as_u64() == Some(short)),
        "no frame targeting short {short} in {frames:?}"
    );
}

// WS-044
#[then("the sniffer batch should contain a query frame")]
async fn then_batch_has_query(world: &mut DaliWorld) {
    let frames = batch_frames(world);
    assert!(
        frames.iter().any(|f| f["is_query"] == true),
        "no query frame in {frames:?}"
    );
}

// WS-043
#[when(regex = r"^a foreign raw forward frame 0x([0-9A-Fa-f]{2}) 0x([0-9A-Fa-f]{2}) is observed on the bus$")]
async fn when_foreign_raw_frame(world: &mut DaliWorld, address: String, command: String) {
    let address = u8::from_str_radix(&address, 16).expect("hex address");
    let command = u8::from_str_radix(&command, 16).expect("hex command");
    let injected = world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .inject_observed_frame(
            [address, command, 0],
            dali2rust_platform::dali::ObservedRawFrameKind::Forward16,
        );
    assert!(injected, "sniffer seam not attached or channel full");
}

fn provoke_warning(world: &mut DaliWorld) {
    world.send_http_request("HEAD", "/api/v1/health", None, "");
}

fn log_lines(frame: &Value) -> Vec<Value> {
    frame["payload"]["lines"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn batch_has_level(frame: &Value, level: &str) -> bool {
    log_lines(frame)
        .iter()
        .any(|line| line["level"].as_str() == Some(level))
}

// WS-051 WS-052 WS-054
#[when("the controller logs a warning")]
async fn when_controller_logs_warning(world: &mut DaliWorld) {
    provoke_warning(world);
}

// WS-053
#[given(regex = r#"^the controller logged a warning with nobody subscribed to "logs"$"#)]
async fn given_warning_before_subscribe(world: &mut DaliWorld) {
    assert!(
        world.ws_clients.is_empty(),
        "the point of this step is that the line predates every subscriber"
    );
    provoke_warning(world);
}

// WS-052 WS-053
#[given(regex = r#"^the WebSocket client is subscribed to "logs" at level "([^"]+)"$"#)]
async fn given_subscribed_logs_at(world: &mut DaliWorld, level: String) {
    let client = last_client(world);
    client.subscribe_logs_at(&level);
    client.wait_for_op("subscribed").expect("subscribed frame");
}

// WS-057
#[when(regex = r#"^the WebSocket client subscribes to "logs" at level "([^"]+)"$"#)]
async fn when_subscribe_logs_at(world: &mut DaliWorld, level: String) {
    last_client(world).subscribe_logs_at(&level);
}

// WS-052 WS-053
#[then(regex = r#"^the batch should carry a "([^"]+)" line$"#)]
async fn then_batch_has_level(world: &mut DaliWorld, level: String) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    assert!(
        batch_has_level(&frame, &level),
        "no {level} line in batch: {frame}"
    );
}

// WS-052
#[then("every line in the batch should carry a target and a sequence number")]
async fn then_lines_carry_identity(world: &mut DaliWorld) {
    let frame = world.remembered_json.clone().expect("no remembered frame");
    let lines = log_lines(&frame);
    assert!(!lines.is_empty(), "empty batch: {frame}");
    for line in lines {
        assert!(
            line["target"].as_str().is_some_and(|t| !t.is_empty()),
            "line without a target: {line}"
        );
        assert!(line["seq"].as_u64().is_some(), "line without a seq: {line}");
    }
}

// WS-054
#[then(regex = r#"^the WebSocket client should receive no LogBatch carrying a "([^"]+)" line$"#)]
async fn then_no_batch_with_level(world: &mut DaliWorld, level: String) {
    let client = last_client(world);
    let seen = client.collect_for(LOG_QUIET_WINDOW);
    let offender = seen
        .iter()
        .find(|frame| frame["type"] == "LogBatch" && batch_has_level(frame, &level));
    assert!(
        offender.is_none(),
        "a {level} line reached a subscriber that asked for less: {offender:?}"
    );
}

// WS-011 WS-030 WS-040
#[then(regex = r#"^the diagnostics websocket counter "([^"]+)" should be at least (\d+)$"#)]
async fn then_counter_at_least(world: &mut DaliWorld, name: String, minimum: u64) {
    let value = wait_for_counter(world, &name, |v| v >= minimum);
    assert!(value >= minimum, "websocket.{name} = {value} < {minimum}");
}

// WS-030 WS-040
#[then(regex = r#"^the diagnostics websocket counter "([^"]+)" should be (\d+)$"#)]
async fn then_counter_equals(world: &mut DaliWorld, name: String, expected: u64) {
    let value = wait_for_counter(world, &name, |v| v == expected);
    assert_eq!(value, expected, "websocket.{name}");
}

// WS-031
#[then(regex = r#"^the diagnostics websocket counter "([^"]+)" should eventually be (\d+)$"#)]
async fn then_counter_eventually(world: &mut DaliWorld, name: String, expected: u64) {
    let value = wait_for_counter(world, &name, |v| v == expected);
    assert_eq!(value, expected, "websocket.{name} never settled");
}

// WS-031
#[when("all WebSocket clients disconnect")]
async fn when_all_disconnect(world: &mut DaliWorld) {
    for client in &mut world.ws_clients {
        client.close();
    }
    world.ws_clients.clear();
}

// WS-045
#[then("I remember the diagnostics websocket sniffer records total")]
async fn then_remember_sniffer_total(world: &mut DaliWorld) {
    let value = websocket_counter(world, "sniffer_records_total");
    world.remembered_u64 = Some(value);
}

// WS-045
#[then("the diagnostics websocket sniffer records total should not have increased")]
async fn then_sniffer_total_unchanged(world: &mut DaliWorld) {
    let before = world.remembered_u64.expect("nothing remembered");
    let quiet = remains_false_for(
        || websocket_counter(world, "sniffer_records_total") != before,
        SNIFFER_QUIET_WINDOW,
    );
    assert!(quiet, "the tap kept recording after unsubscribe");
}
