use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_platform::dali::ObservedRawFrameKind;
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::polling::fetch_json;
use crate::steps::wire::{command_address, go_to_scene, group_command_address, SCENE_NUMBER_MASK};
use crate::DaliWorld;

const READ_MODEL_TIMEOUT: Duration = Duration::from_secs(4);
const DTR0: u8 = 0xA3;
const DTR1: u8 = 0xC3;
const DTR2: u8 = 0xC5;
const ENABLE_DEVICE_TYPE: u8 = 0xC1;
const DT8_DEVICE_TYPE: u8 = 8;
const DT8_SET_TEMPORARY_X_COORDINATE_OPCODE: u8 = 224;
const DT8_SET_TEMPORARY_Y_COORDINATE_OPCODE: u8 = 225;
const DT8_ACTIVATE_OPCODE: u8 = 226;
const DT8_SET_TEMPERATURE_TC_OPCODE: u8 = 231;
const DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE: u8 = 235;
const MIREK_KELVIN_NUMERATOR: u32 = 1_000_000;
const XY_RAW_FULL_SCALE: f64 = 65535.0;

fn inject_forward16(world: &mut DaliWorld, bytes: [u8; 2]) {
    let injected = world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .inject_observed_frame([bytes[0], bytes[1], 0], ObservedRawFrameKind::Forward16);
    assert!(injected, "sniffer seam not attached or channel full");
}

const SET_SCENE_BASE: u8 = 0x40;
const REMOVE_FROM_SCENE_BASE: u8 = 0x50;
const QUERY_STATUS_OPCODE: u8 = 0x90;

fn inject_pair(world: &mut DaliWorld, frame: [u8; 2]) {
    inject_forward16(world, frame);
    inject_forward16(world, frame);
}

// SYS-251 SYS-253
#[when(regex = r"^a foreign SET SCENE (\d+) pair for short address (\d+) is observed on the bus$")]
async fn when_foreign_set_scene_pair(world: &mut DaliWorld, scene: u8, short: u8) {
    inject_pair(world, [command_address(short), SET_SCENE_BASE | (scene & SCENE_NUMBER_MASK)]);
}

// SYS-252
#[when(regex = r"^a foreign REMOVE FROM SCENE (\d+) pair for short address (\d+) is observed on the bus$")]
async fn when_foreign_remove_from_scene_pair(world: &mut DaliWorld, scene: u8, short: u8) {
    let frame = [command_address(short), REMOVE_FROM_SCENE_BASE | (scene & SCENE_NUMBER_MASK)];
    inject_pair(world, frame);
}

// SYS-253
#[when(
    regex = r"^a foreign SET SCENE (\d+) pair for short address (\d+) split by another frame is observed on the bus$"
)]
async fn when_foreign_split_set_scene_pair(world: &mut DaliWorld, scene: u8, short: u8) {
    let frame = [command_address(short), SET_SCENE_BASE | (scene & SCENE_NUMBER_MASK)];
    inject_forward16(world, frame);
    inject_forward16(world, [command_address(short.wrapping_add(1)), QUERY_STATUS_OPCODE]);
    inject_forward16(world, frame);
}

fn virtual_lamp_path(lamp: u8) -> String {
    format!("/api/v1/adapters/0/virtual-lamps/{lamp}")
}

fn lamp_state_field(world: &DaliWorld, lamp: u8, pointer: &str) -> Option<Value> {
    fetch_json(world.server_port(), &virtual_lamp_path(lamp))
        .and_then(|json| json.pointer(pointer).cloned())
}

// SYS-212 SYS-214 SYS-217 WS-042 RULE-036 RULE-039
#[when(regex = r"^a foreign DAPC frame for short address (\d+) level (\d+) is observed on the bus$")]
async fn when_foreign_dapc_observed(world: &mut DaliWorld, short: u8, level: u8) {
    inject_forward16(world, [short << 1, level]);
}

// SYS-246
#[when(regex = r"^a foreign go-to-last-active-level for short address (\d+) is observed on the bus$")]
async fn when_foreign_last_active_observed(world: &mut DaliWorld, short: u8) {
    const GO_TO_LAST_ACTIVE_LEVEL: u8 = 0x0A;
    inject_forward16(world, [(short << 1) | 0x01, GO_TO_LAST_ACTIVE_LEVEL]);
}

// SYS-213 RULE-034 RULE-035 RULE-036 RULE-040
#[when(regex = r"^a foreign broadcast recall of scene (\d+) is observed on the bus$")]
async fn when_foreign_scene_recall_observed(world: &mut DaliWorld, scene: u8) {
    inject_forward16(world, [BROADCAST_INDIRECT, go_to_scene(scene)]);
}

const BROADCAST_INDIRECT: u8 = 0xFF;
const UNADDRESSED_BROADCAST_INDIRECT: u8 = 0xFD;

// RULE-035
#[when(regex = r"^a foreign unaddressed recall of scene (\d+) is observed on the bus$")]
async fn when_foreign_unaddressed_recall_observed(world: &mut DaliWorld, scene: u8) {
    inject_forward16(world, [UNADDRESSED_BROADCAST_INDIRECT, go_to_scene(scene)]);
}

const OFF_OPCODE: u8 = 0x00;

// RULE-039
#[when("a foreign unaddressed OFF is observed on the bus")]
async fn when_foreign_unaddressed_off_observed(world: &mut DaliWorld) {
    inject_forward16(world, [UNADDRESSED_BROADCAST_INDIRECT, OFF_OPCODE]);
}

// RULE-035 RULE-040
#[when(regex = r"^a foreign group (\d+) recall of scene (\d+) is observed on the bus$")]
async fn when_foreign_group_recall_observed(world: &mut DaliWorld, group: u8, scene: u8) {
    inject_forward16(world, [group_command_address(group), go_to_scene(scene)]);
}

// RULE-035
#[when(regex = r"^a foreign short address (\d+) recall of scene (\d+) is observed on the bus$")]
async fn when_foreign_short_recall_observed(world: &mut DaliWorld, short: u8, scene: u8) {
    inject_forward16(world, [command_address(short), go_to_scene(scene)]);
}

fn projector_counter(world: &mut DaliWorld, name: &str) -> u64 {
    let json = super::get_json(world, "/api/v1/diagnostics");
    json["projector"][name]
        .as_u64()
        .unwrap_or_else(|| panic!("projector.{name} missing or not a number in {json}"))
}

// RULE-036
#[given(regex = r#"^I remember the diagnostics projector counter "([a-z_]+)"$"#)]
async fn remember_projector_counter(world: &mut DaliWorld, name: String) {
    world.remembered_u64 = Some(projector_counter(world, &name));
}

// RULE-036
#[then(regex = r#"^the diagnostics projector counter "([a-z_]+)" should have grown by (\d+)$"#)]
async fn then_projector_counter_grew_by(world: &mut DaliWorld, name: String, delta: u64) {
    let before = world.remembered_u64.expect("remembered projector counter");
    assert_eq!(projector_counter(world, &name), before + delta, "projector.{name}");
}

// SYS-214
#[when("an unrecognized foreign frame is observed on the bus")]
async fn when_unrecognized_frame_observed(world: &mut DaliWorld) {
    inject_forward16(world, [0xA1, 0x05]);
}

fn inject_dt8_write(world: &mut DaliWorld, short: u8, value: u16, opcode: u8) {
    inject_forward16(world, [DTR0, (value & 0xFF) as u8]);
    inject_forward16(world, [DTR1, (value >> 8) as u8]);
    inject_forward16(world, [ENABLE_DEVICE_TYPE, DT8_DEVICE_TYPE]);
    inject_forward16(world, [(short << 1) | 1, opcode]);
}

// SYS-215 RULE-040
#[when(regex = r"^a foreign DT8 CCT (\d+)K write for short address (\d+) is observed on the bus$")]
async fn when_foreign_dt8_cct_observed(world: &mut DaliWorld, kelvin: u32, short: u8) {
    let mirek = (MIREK_KELVIN_NUMERATOR / kelvin) as u16;
    inject_dt8_write(world, short, mirek, DT8_SET_TEMPERATURE_TC_OPCODE);
}

// SYS-215
#[when(regex = r"^a foreign DT8 xy write of (\d+\.\d+) (\d+\.\d+) for short address (\d+) is observed on the bus$")]
async fn when_foreign_dt8_xy_observed(world: &mut DaliWorld, x: f64, y: f64, short: u8) {
    let raw_x = (x * XY_RAW_FULL_SCALE).round() as u16;
    let raw_y = (y * XY_RAW_FULL_SCALE).round() as u16;
    inject_dt8_write(world, short, raw_x, DT8_SET_TEMPORARY_X_COORDINATE_OPCODE);
    inject_dt8_write(world, short, raw_y, DT8_SET_TEMPORARY_Y_COORDINATE_OPCODE);
    inject_forward16(world, [ENABLE_DEVICE_TYPE, DT8_DEVICE_TYPE]);
    inject_forward16(world, [(short << 1) | 1, DT8_ACTIVATE_OPCODE]);
}

// SYS-215
#[when(regex = r"^a foreign DT8 RGB write of (\d+) (\d+) (\d+) for short address (\d+) is observed on the bus$")]
async fn when_foreign_dt8_rgb_observed(world: &mut DaliWorld, r: u8, g: u8, b: u8, short: u8) {
    inject_forward16(world, [DTR0, r]);
    inject_forward16(world, [DTR1, g]);
    inject_forward16(world, [DTR2, b]);
    inject_forward16(world, [ENABLE_DEVICE_TYPE, DT8_DEVICE_TYPE]);
    inject_forward16(world, [(short << 1) | 1, DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE]);
    inject_forward16(world, [ENABLE_DEVICE_TYPE, DT8_DEVICE_TYPE]);
    inject_forward16(world, [(short << 1) | 1, DT8_ACTIVATE_OPCODE]);
}

// SYS-210 SYS-211 SYS-212 SYS-213 SYS-214 SYS-241 RULE-031 RULE-036 RULE-038 RULE-039
#[then(regex = r"^the virtual lamp (\d+) runtime level should eventually be (\d+)$")]
async fn then_vl_runtime_level_eventually(world: &mut DaliWorld, lamp: u8, level: u8) {
    wait_until(
        || {
            lamp_state_field(world, lamp, "/state/level").and_then(|v| v.as_u64())
                == Some(u64::from(level))
        },
        READ_MODEL_TIMEOUT,
    );
    let json = lamp_state_field(world, lamp, "/state/level");
    assert_eq!(
        json.as_ref().and_then(Value::as_u64),
        Some(u64::from(level)),
        "virtual lamp {lamp} runtime level: {json:?}"
    );
}

// SYS-215
#[then(regex = r"^the virtual lamp (\d+) runtime color temperature should eventually be (\d+)K$")]
async fn then_vl_runtime_cct_eventually(world: &mut DaliWorld, lamp: u8, kelvin: u16) {
    wait_until(
        || {
            lamp_state_field(world, lamp, "/state/color_temperature_kelvin")
                .and_then(|v| v.as_u64())
                == Some(u64::from(kelvin))
        },
        READ_MODEL_TIMEOUT,
    );
    let json = lamp_state_field(world, lamp, "/state/color_temperature_kelvin");
    assert_eq!(
        json.as_ref().and_then(Value::as_u64),
        Some(u64::from(kelvin)),
        "virtual lamp {lamp} runtime cct: {json:?}"
    );
}

// SYS-210 SYS-211 SYS-212 SYS-213 SYS-241 RULE-031
#[then(regex = r#"^the virtual lamp (\d+) last_dapc_source should be "([^"]+)"$"#)]
async fn then_vl_last_dapc_source(world: &mut DaliWorld, lamp: u8, expected: String) {
    let json = lamp_state_field(world, lamp, "/state/last_dapc_source");
    assert_eq!(
        json.as_ref().and_then(Value::as_str),
        Some(expected.as_str()),
        "virtual lamp {lamp} last_dapc_source: {json:?}"
    );
}

// SYS-215
#[then(regex = r"^the virtual lamp (\d+) last_dapc_source should be null$")]
async fn then_vl_last_dapc_source_null(world: &mut DaliWorld, lamp: u8) {
    let json = lamp_state_field(world, lamp, "/state/last_dapc_source");
    assert!(
        json.is_none() || json == Some(Value::Null),
        "virtual lamp {lamp} last_dapc_source should stay null: {json:?}"
    );
}

// SYS-215
#[then(regex = r"^the virtual lamp (\d+) runtime xy should eventually be (\d+\.\d+) (\d+\.\d+)$")]
async fn then_vl_runtime_xy_eventually(world: &mut DaliWorld, lamp: u8, x: f64, y: f64) {
    let read_xy = |world: &DaliWorld| {
        Some((
            lamp_state_field(world, lamp, "/state/xy/x")?.as_f64()?,
            lamp_state_field(world, lamp, "/state/xy/y")?.as_f64()?,
        ))
    };
    wait_until(|| read_xy(world) == Some((x, y)), READ_MODEL_TIMEOUT);
    let got = read_xy(world);
    assert_eq!(got, Some((x, y)), "virtual lamp {lamp} runtime xy: {got:?}");
}

// SYS-215
#[then(regex = r"^the virtual lamp (\d+) runtime rgb should eventually be (\d+) (\d+) (\d+)$")]
async fn then_vl_runtime_rgb_eventually(world: &mut DaliWorld, lamp: u8, r: u8, g: u8, b: u8) {
    let expected = (u64::from(r), u64::from(g), u64::from(b));
    let read_rgb = |world: &DaliWorld| {
        Some((
            lamp_state_field(world, lamp, "/state/rgb/r")?.as_u64()?,
            lamp_state_field(world, lamp, "/state/rgb/g")?.as_u64()?,
            lamp_state_field(world, lamp, "/state/rgb/b")?.as_u64()?,
        ))
    };
    wait_until(|| read_rgb(world) == Some(expected), READ_MODEL_TIMEOUT);
    let got = read_rgb(world);
    assert_eq!(got, Some(expected), "virtual lamp {lamp} runtime rgb: {got:?}");
}
