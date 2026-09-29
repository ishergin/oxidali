use cucumber::{given, then, when};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::{json, Value};

use crate::steps::frames::{special_frame, standard_frame};
use crate::steps::last_json;
use crate::steps::wire::assert_frame_before;
use crate::DaliWorld;

use super::discovery_scripts::{
    script_detect_dt8_cct, script_discovery, script_discovery_corrupted_window_retry,
    script_discovery_declares_only_dt6, script_discovery_multi_dt_mask, script_discovery_no_answers,
    script_discovery_partial_failure, script_discovery_permanent_multiple_responders,
    script_discovery_unterminated_type_walk, script_scan_discovery, script_six_channel_discovery,
};
use super::read_model::{wait_for_physical_device, wait_for_physical_devices};
use super::{TEST_RANDOM_ADDRESS, TEST_SHORT_ADDRESS};

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-012 MQTT-013 MQTT-015 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-190 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-241 PD-242 PD-243 PD-250 PD-252 PD-253 PD-254 PD-255 PERS-005 STATS-005 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 WS-003 WS-004 WS-010 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024 PD-267 PD-268 POLICY-010 POLICY-011 PD-270 ADP-027 COMM-097 COMM-099 PD-271 WS-059
#[given("a golden control-gear discovery script for short address 0")]
async fn given_golden_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery(&mock);
}

fn full_segment_random_addresses() -> Vec<(u8, u32)> {
    (0..=63u8)
        .map(|short| (short, 0x5C_0000 | (u32::from(short) << 8) | u32::from(short)))
        .collect()
}

// PD-240 COMM-098
#[given("a discovery script for all 64 short addresses")]
async fn given_full_segment_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scan_discovery(&mock, &full_segment_random_addresses(), 0x02);
}

// PD-167 PD-178 PD-251 PD-269
#[given("a six-channel DT8 discovery script for short address 0")]
async fn given_six_channel_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_six_channel_discovery(&mock);
}

// PD-106
#[given("a refresh detect-only script for short address 0")]
async fn given_refresh_detect_only_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_detect_dt8_cct(&mock, TEST_SHORT_ADDRESS);
}

// PD-107
#[given("a discovery script where no control gear answers the presence sweep")]
async fn given_empty_presence_sweep_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_no_answers(&mock);
}

// PD-103
#[given("a discovery script where short address 0 verifies before short address 1 times out")]
async fn given_partial_failure_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_partial_failure(&mock);
}

// PD-104
#[given("a discovery script where foreign-master activity corrupts the QueryShortAddress backward window")]
async fn given_corrupted_window_retry_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_corrupted_window_retry(&mock);
}

// PD-105 PD-189 PD-192
#[given("a discovery script where QueryDeviceType returns MASK before QueryNextDeviceType enumerates DT6 and DT8")]
async fn given_multi_dt_mask_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_multi_dt_mask(&mock);
}

// PD-193
#[given("a discovery script where short address 0 declares only DT6")]
async fn given_dt6_only_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_declares_only_dt6(&mock);
}

// PD-194
#[given("a discovery script where two gear answer one search address for the whole verify")]
async fn given_permanent_multiple_responders_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_permanent_multiple_responders(&mock);
}

// PD-191
#[given("a discovery script where the QueryNextDeviceType walk never reaches the 254 terminator")]
async fn given_unterminated_type_walk_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_unterminated_type_walk(&mock);
}

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-012 MQTT-013 MQTT-015 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-167 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-178 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-189 PD-190 PD-191 PD-192 PD-193 PD-194 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-240 PD-241 PD-242 PD-243 PD-250 PD-251 PD-252 PD-253 PD-254 PD-255 PERS-005 STATS-005 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 WS-003 WS-004 WS-010 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024 PD-267 PD-268 POLICY-010 POLICY-011 ADP-027 COMM-097 COMM-098 COMM-099 PD-271 WS-059
#[when("I start a discovery run for adapter 0")]
async fn when_start_discovery_run(world: &mut DaliWorld) {
    let body = br#"{"mode":"scan_known_short_addresses"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(body),
        "application/json",
    );
}

// PD-106
#[when("I start a refresh discovery run for adapter 0")]
async fn when_start_refresh_discovery_run(world: &mut DaliWorld) {
    let body = br#"{"mode":"refresh_known"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(body),
        "application/json",
    );
}

// PD-194
#[then("the operation error message should mention several responders")]
async fn then_operation_error_mentions_several_responders(world: &mut DaliWorld) {
    let json = last_json(world);
    let message = json
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("query_short_address_multiple"),
        "expected a several-responders diagnosis, got {message:?} in {json:?}"
    );
}

// PD-102
#[then("the discovery transport trace should match the golden control-gear identity flow")]
async fn then_discovery_trace_matches(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_frame_before(
        &frames,
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryRandomAddressH),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Initialise(0x00)),
        special_frame(SpecialCommand::SearchAddrH(0x5C)),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::SearchAddrL(0xC2)),
        special_frame(SpecialCommand::QueryShortAddress),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::QueryShortAddress),
        special_frame(SpecialCommand::Withdraw),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Terminate),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
    );
}

// PD-104
#[then("the discovery transport trace should re-arm before re-asking QueryShortAddress")]
async fn then_discovery_trace_rearms_before_requery(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let query_short = special_frame(SpecialCommand::QueryShortAddress);
    let query_positions: Vec<_> = frames
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| (*frame == query_short).then_some(index))
        .collect();
    assert_eq!(
        query_positions.len(),
        2,
        "expected exactly one re-ask after the violating answer; trace: {frames:?}"
    );
    let search_h = special_frame(SpecialCommand::SearchAddrH(0x5C));
    let rearm = frames[query_positions[0]..query_positions[1]]
        .iter()
        .any(|frame| *frame == search_h);
    assert!(
        rearm,
        "the second ask must be preceded by a fresh search address, not sent blind: {frames:?}"
    );
    assert_frame_before(&frames, query_short, special_frame(SpecialCommand::Withdraw));
}

// PD-105
#[then("the discovery transport trace should enumerate DT6 and DT8 after a MASK device-type advertisement")]
async fn then_discovery_trace_enumerates_mask(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let query_device_type = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType);
    let query_next = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryNextDeviceType);
    let next_positions: Vec<_> = frames
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| (*frame == query_next).then_some(index))
        .collect();
    assert_eq!(
        next_positions.len(),
        3,
        "expected three QueryNextDeviceType frames after MASK advertisement; trace: {frames:?}"
    );
    assert_frame_before(&frames, query_device_type, query_next);
    assert_frame_before(
        &frames,
        query_next,
        special_frame(SpecialCommand::EnableDeviceType(8)),
    );
}

// PD-102 PD-104 PD-105 PD-106 PD-189 PD-190 PD-192 PD-193
#[then(regex = r"^adapter 0 physical device 0 eventually declares device types ([0-9, ]+)$")]
async fn then_declares_device_types(world: &mut DaliWorld, list: String) {
    let expected: Vec<u64> = list
        .split(',')
        .map(|t| t.trim().parse::<u64>().expect("device type number"))
        .collect();
    let want = json!(expected);
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/supported_device_types") == Some(&want)
    });
    assert_eq!(
        json.pointer("/supported_device_types"),
        Some(&want),
        "declared device types: {json:?}"
    );
}

// PD-191
#[then("adapter 0 physical device 0 eventually declares no known device types")]
async fn then_declares_no_known_device_types(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/device_type_discovered").and_then(Value::as_str) == Some("unknown")
    });
    assert_eq!(
        json.pointer("/supported_device_types"),
        None,
        "an unfinished enumeration must report no set at all: {json:?}"
    );
}

// PD-102 PD-104 PD-105 PD-106
#[then("adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity")]
async fn then_discovery_state_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/random_address").and_then(Value::as_u64) == Some(u64::from(TEST_RANDOM_ADDRESS))
            && body.pointer("/device_type_discovered").and_then(Value::as_str) == Some("dt8_color")
            && body.pointer("/color_mode_discovered").and_then(Value::as_str) == Some("cct")
    });
    assert_eq!(json.pointer("/random_address").and_then(Value::as_u64), Some(u64::from(TEST_RANDOM_ADDRESS)));
    assert_eq!(json.pointer("/capabilities/cct"), Some(&json!(true)));
    assert_eq!(json.pointer("/capabilities/xy"), Some(&json!(false)));
    assert_eq!(json.pointer("/capabilities/rgb"), Some(&json!(false)));
}

// PD-240
#[then("adapter 0 physical devices eventually include all 64 short addresses")]
async fn then_full_segment_devices_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_devices(world, |body| {
        body.pointer("/physical_devices")
            .and_then(Value::as_array)
            .is_some_and(|devices| devices.len() == 64)
    });
    let devices = json
        .pointer("/physical_devices")
        .and_then(Value::as_array)
        .expect("physical devices array");
    let mut shorts: Vec<u64> = devices
        .iter()
        .filter_map(|d| d.pointer("/short_address").and_then(Value::as_u64))
        .collect();
    shorts.sort_unstable();
    assert_eq!(
        shorts,
        (0..64u64).collect::<Vec<_>>(),
        "every short address the scan announced should be in the registry"
    );
}

// PD-103 PD-107
#[then("adapter 0 physical devices eventually include only the verified device 0")]
async fn then_partial_discovery_state_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_devices(world, |body| {
        let Some(devices) = body.pointer("/physical_devices").and_then(Value::as_array) else {
            return false;
        };
        devices.len() == 1
            && devices[0].pointer("/short_address").and_then(Value::as_u64) == Some(0)
            && devices[0].pointer("/random_address").and_then(Value::as_u64)
                == Some(u64::from(TEST_RANDOM_ADDRESS))
    });
    let devices = json
        .pointer("/physical_devices")
        .and_then(Value::as_array)
        .expect("physical devices array");
    assert_eq!(devices.len(), 1, "expected one verified device: {json:?}");
    assert_eq!(
        devices[0].pointer("/short_address").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        devices[0].pointer("/random_address").and_then(Value::as_u64),
        Some(u64::from(TEST_RANDOM_ADDRESS))
    );
}
