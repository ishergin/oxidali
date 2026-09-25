use std::time::Duration;

use cucumber::{then, when};
use dali2rust_platform::dali::ObservedRawFrameKind;
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::physical_devices_steps::fetch_json;
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

fn virtual_lamp_path(lamp: u8) -> String {
    format!("/api/v1/adapters/0/virtual-lamps/{lamp}")
}

fn lamp_state_field(world: &DaliWorld, lamp: u8, pointer: &str) -> Option<Value> {
    fetch_json(world.server_port(), &virtual_lamp_path(lamp))
        .and_then(|json| json.pointer(pointer).cloned())
}

// SYS-212 SYS-214 SYS-217 WS-042
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

// SYS-213
#[when(regex = r"^a foreign broadcast recall of scene (\d+) is observed on the bus$")]
async fn when_foreign_scene_recall_observed(world: &mut DaliWorld, scene: u8) {
    const BROADCAST_INDIRECT: u8 = 0xFF;
    const GO_TO_SCENE_BASE: u8 = 0x10;
    inject_forward16(world, [BROADCAST_INDIRECT, GO_TO_SCENE_BASE | (scene & 0x0F)]);
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

// SYS-215
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

// SYS-210 SYS-211 SYS-212 SYS-213 SYS-214 SYS-241
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

// SYS-210 SYS-211 SYS-212 SYS-213 SYS-241
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
