use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::{json, Value};

use crate::steps::groups_steps::bind_discovered_vl1;
use crate::steps::physical_devices_steps::{
    fetch_json, script_discovery_with_features, script_staged_dtrs, special_frame, standard_frame,
};
use crate::DaliWorld;
use crate::steps::{assert_result_skips_lamp, last_json};

const DT8_SET_TEMPERATURE_TC: u8 = 231;
const DT8_SET_TEMPORARY_RGB_DIMLEVEL: u8 = 235;
const DT8_SET_TEMPORARY_WAF_DIMLEVEL: u8 = 236;
const DT8_FEATURES_XY_AND_TC: u8 = 0x03;
const DT8_FEATURES_SIX_CHANNELS: u8 = 0xC0;
const OPERATION_POLL_TIMEOUT: Duration = Duration::from_secs(10);

fn patch_scene_name(world: &mut DaliWorld, scene_id: u8, name: &str) {
    let path = format!("/api/v1/adapters/0/scenes/{scene_id}");
    let body = serde_json::to_vec(&json!({ "name": name })).expect("scene patch body");
    world.send_http_request("PATCH", &path, Some(&body), "application/json");
    assert_eq!(
        world.last_response().expect("scene patch response").status,
        200,
        "scene patch for {scene_id} should succeed"
    );
}

fn patch_scene_matrix_row(world: &mut DaliWorld, scene_id: u8, virtual_lamp_id: u8, desired: Value) {
    let path = format!("/api/v1/adapters/0/scenes/{scene_id}/matrix");
    let body = serde_json::to_vec(&json!({
        "rows": [{ "virtual_lamp_id": virtual_lamp_id, "desired": desired }]
    }))
    .expect("scene matrix patch body");
    world.send_http_request("PATCH", &path, Some(&body), "application/json");
    crate::steps::await_config_write(
        world,
        &format!("scene matrix patch for vl{virtual_lamp_id}"),
    );
}

fn level_desired(level: u8) -> Value {
    json!({ "included": true, "power": "on", "level": level })
}

fn rgb_desired(rgb: (u8, u8, u8), level: u8) -> Value {
    json!({
        "included": true,
        "power": "on",
        "level": level,
        "color_mode": "rgb",
        "rgb": { "r": rgb.0, "g": rgb.1, "b": rgb.2 },
    })
}

fn cct_desired(kelvin: u16, level: u8) -> Value {
    json!({
        "included": true,
        "power": "on",
        "level": level,
        "color_mode": "cct",
        "color_temperature_kelvin": kelvin,
    })
}

fn kelvin_to_mirek(kelvin: u16) -> u16 {
    dali2rust_domain::registry::kelvin_to_mirek(kelvin).expect("scripted CCT is never 0 K")
}

fn dt8_command_frame(short: u8, opcode: u8) -> u16 {
    let address = DaliAddress::short(short).expect("short address");
    ((u16::from(address.encode_address_byte() | 0x01)) << 8) | u16::from(opcode)
}

fn script_scene_write(mock: &MockDaliTransport, short: u8, scene_id: u8, level: u8) {
    script_scene_write_bare(mock, short, scene_id, level);
    script_scene_colour_verify(mock, short, scene_id, level, None);
}

fn script_scene_write_bare(mock: &MockDaliTransport, short: u8, scene_id: u8, level: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(level)));
    let set_scene = standard_frame(short, StandardCommand::SetScene { scene: scene_id });
    mock.expect_forward_frame(set_scene);
    mock.expect_forward_frame(set_scene);
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QuerySceneLevel { scene: scene_id }),
        Some(level),
    );
}

fn script_scene_cct_write(mock: &MockDaliTransport, short: u8, scene_id: u8, kelvin: u16, level: u8) {
    let mirek = kelvin_to_mirek(kelvin);
    script_staged_dtrs(
        mock,
        short,
        &[(mirek & 0x00FF) as u8, (mirek >> 8) as u8],
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_command_frame(short, DT8_SET_TEMPERATURE_TC));
    script_scene_write_bare(mock, short, scene_id, level);
    script_scene_colour_verify(mock, short, scene_id, level, Some(mirek));
}

fn script_scene_rgb_write(
    mock: &MockDaliTransport,
    short: u8,
    scene_id: u8,
    wire: [u8; 3],
    level: u8,
    readback: [u8; 3],
) {
    script_staged_dtrs(mock, short, &wire);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_command_frame(short, DT8_SET_TEMPORARY_RGB_DIMLEVEL));
    script_staged_dtrs(mock, short, &[0, 0, 0]);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_command_frame(short, DT8_SET_TEMPORARY_WAF_DIMLEVEL));
    script_scene_write_bare(mock, short, scene_id, level);
    script_scene_rgb_verify(mock, short, scene_id, level, readback);
}

fn script_scene_rgb_verify(
    mock: &MockDaliTransport,
    short: u8,
    scene_id: u8,
    slot_level: u8,
    readback: [u8; 3],
) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QuerySceneLevel { scene: scene_id }),
        Some(slot_level),
    );
    script_report_selector(mock, short, DT8_SELECTOR_REPORT_COLOUR_TYPE);
    mock.expect_forward_frame_with_backward(
        dt8_command_frame(short, DT8_QUERY_COLOUR_VALUE),
        Some(DT8_COLOUR_TYPE_BYTE_RGBWAF),
    );
    for slot in 0..6u8 {
        script_report_selector(mock, short, DT8_SELECTOR_REPORT_RED + slot);
        let answer = readback.get(usize::from(slot)).copied().unwrap_or(0);
        mock.expect_forward_frame_with_backward(
            dt8_command_frame(short, DT8_QUERY_COLOUR_VALUE),
            Some(answer),
        );
    }
}

fn script_report_selector(mock: &MockDaliTransport, short: u8, selector: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(selector)));
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        Some(selector),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
}

const DT8_QUERY_COLOUR_VALUE: u8 = 250;
const DT8_SELECTOR_REPORT_COLOUR_TYPE: u8 = 240;
const DT8_SELECTOR_REPORT_TC: u8 = 226;
const DT8_SELECTOR_REPORT_RED: u8 = 233;
const DT8_COLOUR_TYPE_BYTE_RGBWAF: u8 = 0x80;

fn script_scene_colour_verify(
    mock: &MockDaliTransport,
    short: u8,
    scene_id: u8,
    slot_level: u8,
    mirek: Option<u16>,
) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QuerySceneLevel { scene: scene_id }),
        Some(slot_level),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(
        DT8_SELECTOR_REPORT_COLOUR_TYPE,
    )));
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        Some(DT8_SELECTOR_REPORT_COLOUR_TYPE),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    let Some(mirek) = mirek else {
        mock.expect_forward_frame_with_backward(
            dt8_command_frame(short, DT8_QUERY_COLOUR_VALUE),
            Some(0xFF),
        );
        return;
    };
    mock.expect_forward_frame_with_backward(
        dt8_command_frame(short, DT8_QUERY_COLOUR_VALUE),
        Some(0x20),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(DT8_SELECTOR_REPORT_TC)));
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        Some(DT8_SELECTOR_REPORT_TC),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        dt8_command_frame(short, DT8_QUERY_COLOUR_VALUE),
        Some((mirek >> 8) as u8),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        Some(mirek as u8),
    );
}

fn scene_matrix_row(world: &DaliWorld, virtual_lamp_id: u8) -> Value {
    let json = last_json(world);
    json.get("rows")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| {
                    row.get("virtual_lamp_id").and_then(Value::as_u64)
                        == Some(u64::from(virtual_lamp_id))
                })
                .cloned()
        })
        .unwrap_or_else(|| panic!("virtual lamp {virtual_lamp_id} row missing: {json:?}"))
}

fn assert_state_excluded(state: &Value) {
    assert_eq!(state.get("included").and_then(Value::as_bool), Some(false), "{state:?}");
    for key in ["power", "level", "color_mode", "color_temperature_kelvin", "xy", "rgb"] {
        assert!(
            state.get(key).is_some_and(Value::is_null),
            "excluded field {key} must serialize as null: {state:?}"
        );
    }
}

fn wait_for_scene_apply_status(world: &DaliWorld, matches: impl Fn(&str) -> bool) {
    let port = world.server_port();
    wait_until(
        || {
            let Some(list) = fetch_json(port, "/api/v1/operations") else {
                return false;
            };
            let Some(keys) = list.get("operations").and_then(Value::as_array) else {
                return false;
            };
            keys.iter()
                .filter_map(Value::as_str)
                .filter(|key| key.starts_with("scn-apply-"))
                .any(|key| {
                    fetch_json(port, &format!("/api/v1/operations/{key}"))
                        .and_then(|op| op.get("status").and_then(Value::as_str).map(String::from))
                        .is_some_and(|status| matches(&status))
                })
        },
        OPERATION_POLL_TIMEOUT,
    );
}

// SCN-001 SCN-020
#[given("adapter 0 has scenes 1 and 3 configured")]
async fn given_scenes_configured(world: &mut DaliWorld) {
    patch_scene_name(world, 1, "Scene 1");
    patch_scene_name(world, 3, "Scene 3");
}

// SCN-010
#[given(regex = r#"^scene (\d+) on adapter 0 is named "([^"]+)"$"#)]
async fn given_scene_named(world: &mut DaliWorld, scene_id: u8, name: String) {
    patch_scene_name(world, scene_id, &name);
}

// SCN-010 SCN-060 SCN-062 SCN-063 SCN-065 SCN-080 REG-031 SYS-211 SYS-213 SYS-241 SCN-040 SCN-050
#[given(regex = r"^adapter 0 scene (\d+) desired row for virtual lamp (\d+) has level (\d+)$")]
async fn given_desired_row_level(world: &mut DaliWorld, scene_id: u8, virtual_lamp_id: u8, level: u8) {
    patch_scene_matrix_row(world, scene_id, virtual_lamp_id, level_desired(level));
}

// SCN-030 SCN-040 SCN-041 SCN-060 SCN-083 SCN-084 SCN-085
#[given(regex = r"^adapter 0 scene (\d+) desired row for virtual lamp (\d+) has CCT (\d+)K and level (\d+)$")]
async fn given_desired_row_cct(
    world: &mut DaliWorld,
    scene_id: u8,
    virtual_lamp_id: u8,
    kelvin: u16,
    level: u8,
) {
    patch_scene_matrix_row(world, scene_id, virtual_lamp_id, cct_desired(kelvin, level));
}

// SCN-092 SCN-093
#[given(
    regex = r"^adapter 0 scene (\d+) desired row for virtual lamp (\d+) has rgb (\d+),(\d+),(\d+) and level (\d+)$"
)]
async fn given_desired_row_rgb(
    world: &mut DaliWorld,
    scene_id: u8,
    virtual_lamp_id: u8,
    r: u8,
    g: u8,
    b: u8,
    level: u8,
) {
    patch_scene_matrix_row(world, scene_id, virtual_lamp_id, rgb_desired((r, g, b), level));
}

// SCN-092 SCN-093
#[given(
    regex = r"^adapter 0 scene (\d+) rgb write for short (\d+) level (\d+) is scripted staging (\d+),(\d+),(\d+) with readback (\d+),(\d+),(\d+)$"
)]
#[allow(clippy::too_many_arguments, reason = "one scripted wire sequence, named end to end")]
async fn given_scene_rgb_write_scripted(
    world: &mut DaliWorld,
    scene_id: u8,
    short: u8,
    level: u8,
    sr: u8,
    sg: u8,
    sb: u8,
    rr: u8,
    rg: u8,
    rb: u8,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scene_rgb_write(&mock, short, scene_id, [sr, sg, sb], level, [rr, rg, rb]);
}

// SCN-062 REG-031
#[when(regex = r"^adapter 0 scene (\d+) desired row for virtual lamp (\d+) changes level to (\d+)$")]
async fn when_desired_row_level_changes(
    world: &mut DaliWorld,
    scene_id: u8,
    virtual_lamp_id: u8,
    level: u8,
) {
    patch_scene_matrix_row(world, scene_id, virtual_lamp_id, level_desired(level));
}

// SCN-060
#[when(regex = r"^adapter 0 scene (\d+) desired row for virtual lamp (\d+) is excluded$")]
async fn when_desired_row_excluded(world: &mut DaliWorld, scene_id: u8, virtual_lamp_id: u8) {
    patch_scene_matrix_row(world, scene_id, virtual_lamp_id, json!({ "included": false }));
}

// SCN-041 SCN-048
#[given("adapter 0 has a discovered and bound xy-capable virtual lamp 1 on physical device 0")]
async fn given_discovered_xy_capable_vl1(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_discovery_with_features(&mock, DT8_FEATURES_XY_AND_TC);
    }
    bind_discovered_vl1(world);
}

// SCN-046 SCN-047 SCN-092 SCN-093
#[given("adapter 0 has a discovered and bound rgbwaf-capable virtual lamp 1 on physical device 0")]
async fn given_discovered_rgbwaf_capable_vl1(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_discovery_with_features(&mock, DT8_FEATURES_SIX_CHANNELS);
    }
    bind_discovered_vl1(world);
}

// SCN-060 SCN-062 SCN-063 REG-031 SYS-211 SYS-213 SYS-241
#[given(regex = r"^adapter 0 scene (\d+) write for short (\d+) level (\d+) is scripted$")]
async fn given_scene_write_scripted(world: &mut DaliWorld, scene_id: u8, short: u8, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scene_write(&mock, short, scene_id, level);
}

// SCN-060 SCN-083
#[given(regex = r"^adapter 0 scene (\d+) CCT (\d+)K write for short (\d+) level (\d+) is scripted$")]
async fn given_scene_cct_write_scripted(
    world: &mut DaliWorld,
    scene_id: u8,
    kelvin: u16,
    short: u8,
    level: u8,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scene_cct_write(&mock, short, scene_id, kelvin, level);
}

// SCN-084
#[given(
    regex = r"^adapter 0 scene (\d+) CCT (\d+)K write for short (\d+) level (\d+) is scripted with clamped readback (\d+) mirek$"
)]
async fn given_scene_cct_write_clamped(
    world: &mut DaliWorld,
    scene_id: u8,
    kelvin: u16,
    short: u8,
    level: u8,
    readback_mirek: u16,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let mirek = kelvin_to_mirek(kelvin);
    script_staged_dtrs(&mock, short, &[(mirek & 0x00FF) as u8, (mirek >> 8) as u8]);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_command_frame(short, DT8_SET_TEMPERATURE_TC));
    script_scene_write_bare(&mock, short, scene_id, level);
    script_scene_colour_verify(&mock, short, scene_id, level, Some(readback_mirek));
}

// SCN-085
#[given(
    regex = r"^a scene-colours audit script for short (\d+) with scene (\d+) holding (\d+) mirek at level (\d+)$"
)]
async fn given_scene_colours_audit_script(
    world: &mut DaliWorld,
    short: u8,
    scene: u8,
    mirek: u16,
    level: u8,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    crate::steps::physical_devices_steps::script_attribute_read_prelude(&mock, short);
    for s in 0..16u8 {
        if s == scene {
            script_scene_colour_verify(&mock, short, s, level, Some(mirek));
        } else {
            script_scene_colour_verify(&mock, short, s, 0xFF, None);
        }
    }
}

// SCN-060
#[given(regex = r"^adapter 0 scene (\d+) clear for short (\d+) is scripted$")]
async fn given_scene_clear_scripted(world: &mut DaliWorld, scene_id: u8, short: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let remove = standard_frame(short, StandardCommand::RemoveScene { scene: scene_id });
    mock.expect_forward_frame(remove);
    mock.expect_forward_frame(remove);
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QuerySceneLevel { scene: scene_id }),
        Some(0xFF),
    );
    script_scene_colour_verify(&mock, short, scene_id, 0xFF, None);
}

// SCN-062
#[given(regex = r"^adapter 0 scene (\d+) write for short (\d+) level (\d+) fails on the DALI bus$")]
async fn given_scene_write_fails(world: &mut DaliWorld, _scene_id: u8, _short: u8, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.expect_forward_frame_send_error(special_frame(SpecialCommand::Dtr0(level)));
}

// SCN-050
#[when(regex = r"^I PUT a complete scene (\d+) matrix with virtual lamp (\d+) at level (\d+) and 63 excluded rows$")]
async fn when_put_complete_matrix(world: &mut DaliWorld, scene_id: u8, virtual_lamp_id: u8, level: u8) {
    let rows: Vec<Value> = (0u8..64)
        .map(|vl| {
            let desired = if vl == virtual_lamp_id {
                level_desired(level)
            } else {
                json!({ "included": false })
            };
            json!({ "virtual_lamp_id": vl, "desired": desired })
        })
        .collect();
    let body = serde_json::to_vec(&json!({ "rows": rows })).expect("matrix put body");
    let path = format!("/api/v1/adapters/0/scenes/{scene_id}/matrix");
    world.send_http_request("PUT", &path, Some(&body), "application/json");
}

// SCN-001
#[then("the scenes list should contain scenes 1 and 3")]
async fn then_scenes_list_contains(world: &mut DaliWorld) {
    let json = last_json(world);
    let scenes = json
        .get("scenes")
        .and_then(Value::as_array)
        .expect("scenes array");
    let ids: Vec<u64> = scenes
        .iter()
        .filter_map(|scene| scene.get("scene_id").and_then(Value::as_u64))
        .collect();
    assert!(
        ids.contains(&1) && ids.contains(&3),
        "expected scenes 1 and 3, got {json:?}"
    );
}

// SCN-030
#[then("the scene matrix should expose 64 rows with virtual_lamp_id, name, capabilities and dirty")]
async fn then_matrix_exposes_rows(world: &mut DaliWorld) {
    let json = last_json(world);
    let rows = json.get("rows").and_then(Value::as_array).expect("rows array");
    assert_eq!(rows.len(), 64, "expected 64 rows, got {json:?}");
    let first = rows.first().expect("first row");
    for key in ["virtual_lamp_id", "name", "capabilities", "desired", "applied", "dirty"] {
        assert!(first.get(key).is_some(), "row missing {key}: {first:?}");
    }
}

// SCN-030 SCN-040 SCN-041 SCN-050 REG-031 SCN-046
#[then(regex = r#"^the scene matrix desired row for virtual lamp (\d+) should be included with level (\d+) and color_mode "([^"]+)"$"#)]
async fn then_desired_row_with_color(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    level: u8,
    color_mode: String,
) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let desired = row.get("desired").expect("desired state");
    assert_eq!(desired.get("included").and_then(Value::as_bool), Some(true), "{row:?}");
    assert_eq!(desired.get("level").and_then(Value::as_u64), Some(u64::from(level)), "{row:?}");
    assert_eq!(
        desired.get("color_mode").and_then(Value::as_str),
        Some(color_mode.as_str()),
        "{row:?}"
    );
}

// SCN-040 SCN-050 REG-031
#[then(regex = r"^the scene matrix desired row for virtual lamp (\d+) should be included with level (\d+) and no color$")]
async fn then_desired_row_no_color(world: &mut DaliWorld, virtual_lamp_id: u8, level: u8) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let desired = row.get("desired").expect("desired state");
    assert_eq!(desired.get("included").and_then(Value::as_bool), Some(true), "{row:?}");
    assert_eq!(desired.get("level").and_then(Value::as_u64), Some(u64::from(level)), "{row:?}");
    assert!(
        desired.get("color_mode").is_some_and(Value::is_null),
        "expected color_mode null: {row:?}"
    );
}

// SCN-030 SCN-040 SCN-050 VL-104
#[then(regex = r"^the scene matrix desired row for virtual lamp (\d+) should serialize excluded setpoint fields as null$")]
async fn then_desired_row_excluded(world: &mut DaliWorld, virtual_lamp_id: u8) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    assert_state_excluded(row.get("desired").expect("desired state"));
}

// SCN-041
#[then(regex = r"^the scene matrix desired row for virtual lamp (\d+) should have inactive cct and rgb fields null$")]
async fn then_desired_row_inactive_nulls(world: &mut DaliWorld, virtual_lamp_id: u8) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let desired = row.get("desired").expect("desired state");
    assert!(
        desired.get("xy").is_some_and(|value| !value.is_null()),
        "xy target expected: {row:?}"
    );
    for key in ["color_temperature_kelvin", "rgb"] {
        assert!(
            desired.get(key).is_some_and(Value::is_null),
            "inactive {key} must be null: {row:?}"
        );
    }
}

// SCN-046
#[then(regex = r"^the scene matrix desired row for virtual lamp (\d+) should have rgb (\d+),(\d+),(\d+) and waf (\d+),(\d+),(\d+)$")]
#[allow(clippy::too_many_arguments, reason = "six channels are the assertion")]
async fn then_desired_row_six_channels(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    r: u8,
    g: u8,
    b: u8,
    w: u8,
    a: u8,
    f: u8,
) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let desired = row.get("desired").expect("desired state");
    for (key, channels, expected) in [
        ("rgb", ["r", "g", "b"], [r, g, b]),
        ("waf", ["w", "a", "f"], [w, a, f]),
    ] {
        let object = desired.get(key).expect("colour object");
        for (channel, value) in channels.iter().zip(expected) {
            assert_eq!(
                object.get(*channel).and_then(Value::as_u64),
                Some(u64::from(value)),
                "{key}.{channel}: {row:?}"
            );
        }
    }
}

// SCN-030
#[then("scene matrix rows should not expose RuntimeObservation fields")]
async fn then_rows_have_no_runtime_fields(world: &mut DaliWorld) {
    let json = last_json(world);
    let rows = json.get("rows").and_then(Value::as_array).expect("rows array");
    for row in rows {
        for side in ["desired", "applied"] {
            let state = row.get(side).expect("row state");
            for key in ["status", "failure_status", "value_source", "last_seen_ms", "last_dapc_source", "error"] {
                assert!(
                    state.get(key).is_none(),
                    "runtime field {key} leaked into {side}: {row:?}"
                );
            }
        }
    }
}

// SCN-060 SCN-062 REG-031
#[then(regex = r"^the scene matrix applied row for virtual lamp (\d+) should have included true and level (\d+)$")]
async fn then_applied_row_level(world: &mut DaliWorld, virtual_lamp_id: u8, level: u8) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let applied = row.get("applied").expect("applied state");
    assert_eq!(applied.get("included").and_then(Value::as_bool), Some(true), "{row:?}");
    assert_eq!(applied.get("level").and_then(Value::as_u64), Some(u64::from(level)), "{row:?}");
}

// SCN-083 SCN-084 SCN-085
#[then(
    regex = r#"^the scene matrix row for virtual lamp (\d+) should read back applied CCT (\d+)K dirty (true|false)$"#
)]
async fn then_applied_row_readback_cct(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    kelvin: u16,
    dirty: String,
) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let applied = row.get("applied").expect("applied state");
    assert_eq!(
        applied.get("color_mode").and_then(Value::as_str),
        Some("cct"),
        "{row:?}"
    );
    assert_eq!(
        applied
            .get("color_temperature_kelvin")
            .and_then(Value::as_u64),
        Some(u64::from(kelvin)),
        "applied Tc is the REPORT readback, not the programmed echo: {row:?}"
    );
    assert_eq!(
        row.get("dirty").and_then(Value::as_bool),
        Some(dirty == "true"),
        "{row:?}"
    );
}

// SCN-092 SCN-093
#[then(
    regex = r#"^the scene matrix row for virtual lamp (\d+) should read back applied rgb (\d+),(\d+),(\d+) dirty (true|false)$"#
)]
async fn then_applied_row_readback_rgb(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    r: u64,
    g: u64,
    b: u64,
    dirty: String,
) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let applied = row.get("applied").expect("applied state");
    assert_eq!(applied.get("color_mode").and_then(Value::as_str), Some("rgb"), "{row:?}");
    for (key, want) in [("r", r), ("g", g), ("b", b)] {
        assert_eq!(
            applied.pointer(&format!("/rgb/{key}")).and_then(Value::as_u64),
            Some(want),
            "applied {key} is the REPORT readback in sRGB, not the wire byte: {row:?}"
        );
    }
    assert_eq!(
        row.get("dirty").and_then(Value::as_bool),
        Some(dirty == "true"),
        "{row:?}"
    );
}

// SCN-060
#[then(regex = r#"^the scene matrix applied row for virtual lamp (\d+) should echo color_mode "([^"]+)"$"#)]
async fn then_applied_row_echo(world: &mut DaliWorld, virtual_lamp_id: u8, color_mode: String) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    let applied = row.get("applied").expect("applied state");
    assert_eq!(
        applied.get("color_mode").and_then(Value::as_str),
        Some(color_mode.as_str()),
        "{row:?}"
    );
}

// SCN-060
#[then(regex = r"^the scene matrix applied row for virtual lamp (\d+) should serialize excluded setpoint fields as null$")]
async fn then_applied_row_excluded(world: &mut DaliWorld, virtual_lamp_id: u8) {
    let row = scene_matrix_row(world, virtual_lamp_id);
    assert_state_excluded(row.get("applied").expect("applied state"));
}

// SCN-060
#[then(regex = r"^the scene-apply operation result should count (\d+) written, (\d+) updated and (\d+) cleared outcomes$")]
async fn then_scene_apply_counts(world: &mut DaliWorld, written: u64, updated: u64, cleared: u64) {
    let json = last_json(world);
    let result = json
        .get("result")
        .unwrap_or_else(|| panic!("result in operation view: {json:?}"));
    assert_eq!(result.get("written_total").and_then(Value::as_u64), Some(written), "result: {result:?}");
    assert_eq!(result.get("updated_total").and_then(Value::as_u64), Some(updated), "result: {result:?}");
    assert_eq!(result.get("cleared_total").and_then(Value::as_u64), Some(cleared), "result: {result:?}");
    assert_eq!(result.get("failed_total").and_then(Value::as_u64), Some(0), "result: {result:?}");
}

// SCN-063
#[then(regex = r#"^the scene-apply operation result should list skipped virtual lamp (\d+) with reason "([^"]+)"$"#)]
async fn then_scene_apply_lists_skipped(world: &mut DaliWorld, virtual_lamp_id: u8, reason: String) {
    assert_result_skips_lamp(world, virtual_lamp_id, &reason);
}

// SCN-062
#[then(regex = r"^the scene-apply operation result should list failed virtual lamp (\d+)$")]
async fn then_scene_apply_lists_failed(world: &mut DaliWorld, virtual_lamp_id: u8) {
    let json = last_json(world);
    let failed = json
        .pointer("/result/failed")
        .and_then(Value::as_array)
        .expect("result.failed array");
    assert!(
        failed.iter().any(|entry| {
            entry.get("virtual_lamp_id").and_then(Value::as_u64) == Some(u64::from(virtual_lamp_id))
        }),
        "failed entry for vl{virtual_lamp_id} missing: {json:?}"
    );
}

// SCN-065
#[then("the last apply operation eventually becomes active")]
async fn then_apply_operation_active(world: &mut DaliWorld) {
    wait_for_scene_apply_status(world, |status| matches!(status, "accepted" | "running"));
}

// SCN-065
#[then("the last apply operation eventually finishes")]
async fn then_apply_operation_finishes(world: &mut DaliWorld) {
    wait_for_scene_apply_status(world, |status| {
        matches!(status, "succeeded" | "failed" | "timed_out" | "cancelled")
    });
}

// SCN-081
#[then("the response should carry a numeric correlation_id")]
async fn then_response_carries_correlation_id(world: &mut DaliWorld) {
    let json = last_json(world);
    assert!(
        json.get("correlation_id").and_then(Value::as_u64).is_some(),
        "expected numeric correlation_id: {json:?}"
    );
}

// OP-101 OP-133 SCN-081
#[then("the operations list should be empty")]
async fn then_operations_list_empty(world: &mut DaliWorld) {
    let json = last_json(world);
    let operations = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    assert!(operations.is_empty(), "expected no operations, got {json:?}");
}

// MQTT-019
#[given(regex = r"^a broadcast scene recall script for scene (\d+)$")]
async fn given_broadcast_recall_script(world: &mut DaliWorld, scene_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::GoToScene { scene: scene_id },
        }
        .to_forward_frame()
        .raw(),
    );
}

// SCN-080
#[then(regex = r"^the DALI mock transport should have sent only a broadcast go-to-scene (\d+) frame$")]
async fn then_only_broadcast_recall_frame(world: &mut DaliWorld, scene_id: u8) {
    let expected = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::GoToScene { scene: scene_id },
    }
    .to_forward_frame()
    .raw();
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_eq!(frames, vec![expected], "unexpected forward frames: {frames:?}");
}

// SCN-086 SYS-241
#[then(regex = r"^the DALI mock transport should have sent only a group (\d+) go-to-scene (\d+) frame$")]
async fn then_only_group_recall_frame(world: &mut DaliWorld, group_id: u8, scene_id: u8) {
    let expected = DaliCommand::Standard {
        address: DaliAddress::group(group_id).expect("scenario group id in 0..=15"),
        command: StandardCommand::GoToScene { scene: scene_id },
    }
    .to_forward_frame()
    .raw();
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_eq!(frames, vec![expected], "unexpected forward frames: {frames:?}");
}
