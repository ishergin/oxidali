use cucumber::{given, then, when};
use serde_json::{json, Value};

use crate::DaliWorld;
use crate::steps::last_json;

const HCL_SCHEDULES_PATH: &str = "/api/v1/hcl-schedules";

fn valid_schedule(schedule_id: &str) -> Value {
    json!({
        "schedule_id": schedule_id,
        "enabled": true,
        "algorithm": "stepped",
        "active_days": ["mon", "tue", "wed", "thu", "fri"],
        "location": null,
        "targets": [{ "adapter_id": 0, "scope": "group", "group_ids": [1, 5] }],
        "points": [
            { "time_ref": "absolute", "offset_minutes": 360, "level_mode": "absolute",
              "level": 80, "color_temperature_kelvin": 2700 }
        ]
    })
}

fn post_schedule(world: &mut DaliWorld, payload: &Value) {
    let body = serde_json::to_vec(payload).expect("schedule payload");
    world.send_http_request("POST", HCL_SCHEDULES_PATH, Some(&body), "application/json");
}

fn post_schedule_and_wait(world: &mut DaliWorld, payload: &Value) {
    post_schedule(world, payload);
    crate::steps::await_config_write(world, "schedule create");
}

fn get_schedule(world: &mut DaliWorld, schedule_id: &str) -> Value {
    let path = format!("{HCL_SCHEDULES_PATH}/{schedule_id}");
    world.send_http_request("GET", &path, None, "");
    assert_eq!(
        world.last_response().expect("detail response").status,
        200,
        "schedule {schedule_id} should be readable"
    );
    last_json(world)
}

// HCL-001 HCL-010 HCL-023 HCL-028 HCL-030 HCL-031 HCL-043 HCL-046 HCL-047 HCL-048 HCL-049 HCL-050 HCL-053 PERS-004 HCL-072 HCL-075 HCL-076 SYS-237
#[given(regex = r#"^HCL schedule "([^"]+)" exists$"#)]
async fn given_schedule_exists(world: &mut DaliWorld, schedule_id: String) {
    post_schedule_and_wait(world, &valid_schedule(&schedule_id));
}

// HCL-021 HCL-024 HCL-025 HCL-026 HCL-032 HCL-033 HCL-034 HCL-035 HCL-036 HCL-037 HCL-038 HCL-039 HCL-040 HCL-041 HCL-042 HCL-044 HCL-045 HCL-023
#[when(regex = r#"^I POST an HCL schedule whose "([^"]+)" is (.+)$"#)]
async fn when_post_schedule_with_override(world: &mut DaliWorld, field: String, raw: String) {
    let value: Value = serde_json::from_str(&raw).expect("override value should be JSON");
    let mut payload = valid_schedule("candidate");
    payload
        .as_object_mut()
        .expect("schedule object")
        .insert(field, value);
    post_schedule(world, &payload);
}

// HCL-027
#[when("I POST an HCL schedule without a schedule_id")]
async fn when_post_schedule_without_id(world: &mut DaliWorld) {
    let mut payload = valid_schedule("ignored");
    payload
        .as_object_mut()
        .expect("schedule object")
        .remove("schedule_id");
    post_schedule(world, &payload);
}

// HCL-001
#[then(regex = r#"^the HCL schedule list should be exactly "([^"]*)"$"#)]
async fn then_list_is_exactly(world: &mut DaliWorld, expected: String) {
    let body = last_json(world);
    let listed: Vec<String> = body["schedules"]
        .as_array()
        .expect("schedules array")
        .iter()
        .map(|item| item["schedule_id"].as_str().unwrap_or_default().to_string())
        .collect();
    let want: Vec<String> = expected
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    assert_eq!(listed, want, "listed schedules");
}

// HCL-020 HCL-022 HCL-028 HCL-030 PERS-004
#[then(regex = r#"^HCL schedule "([^"]+)" should have (\d+) targets? and (\d+) points?$"#)]
async fn then_schedule_shape(
    world: &mut DaliWorld,
    schedule_id: String,
    targets: usize,
    points: usize,
) {
    let dto = get_schedule(world, &schedule_id);
    assert_eq!(
        dto["targets"].as_array().map(Vec::len),
        Some(targets),
        "target count of {schedule_id}"
    );
    assert_eq!(
        dto["points"].as_array().map(Vec::len),
        Some(points),
        "point count of {schedule_id}"
    );
}

// HCL-020 HCL-022 HCL-028 HCL-030
#[then(regex = r#"^HCL schedule "([^"]+)" field "([^"]+)" should be (.+)$"#)]
async fn then_schedule_field(
    world: &mut DaliWorld,
    schedule_id: String,
    pointer: String,
    expected: String,
) {
    let dto = get_schedule(world, &schedule_id);
    let found = dto
        .pointer(&format!("/{}", pointer.replace('.', "/")))
        .cloned()
        .unwrap_or(Value::Null);
    let want: Value = serde_json::from_str(&expected).unwrap_or(Value::String(expected.clone()));
    assert_eq!(found, want, "{pointer} of {schedule_id}");
}

// HCL-001 HCL-010 SCN-087 SCN-088 SCN-089 SCN-090 SCN-091
#[then("no DALI frames should have been sent")]
async fn then_no_dali_frames(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert!(frames.is_empty(), "unexpected DALI traffic: {frames:?}");
}

// HCL-043
#[then(regex = r#"^HCL schedule "([^"]+)" should be gone$"#)]
async fn then_schedule_is_gone(world: &mut DaliWorld, schedule_id: String) {
    let path = format!("{HCL_SCHEDULES_PATH}/{schedule_id}");
    world.send_http_request("GET", &path, None, "");
    assert_eq!(
        world.last_response().expect("detail response").status,
        404,
        "deleted schedule {schedule_id} must not be readable"
    );
}

const SCENARIO_MIDNIGHT_UNIX_MS: u64 = 1_782_259_200_000;
const MS_PER_MINUTE: u64 = 60_000;

fn set_clock(world: &mut DaliWorld, minutes_since_midnight: u64) {
    let body = serde_json::to_vec(&json!({
        "unix_ms": SCENARIO_MIDNIGHT_UNIX_MS + minutes_since_midnight * MS_PER_MINUTE,
        "timezone": "UTC0",
    }))
    .expect("time body");
    world.send_http_request("PUT", "/api/v1/time", Some(&body), "application/json");
    assert_eq!(
        world.last_response().expect("time response").status,
        200,
        "anchoring the clock should succeed"
    );
}

fn scheduler_ticks(port: u16) -> u64 {
    crate::steps::physical_devices_steps::fetch_json(port, "/api/v1/diagnostics")
        .map(|json| json["hcl"]["ticks"].as_u64().unwrap_or(0))
        .unwrap_or(0)
}

fn dapc_frame(group_id: Option<u8>, level: u8) -> u16 {
    let address = match group_id {
        Some(group) => dali2rust_domain::dali::types::DaliAddress::group(group).expect("group"),
        None => dali2rust_domain::dali::types::DaliAddress::Broadcast,
    };
    dali2rust_domain::dali::pres::command::DaliCommand::Standard {
        address,
        command: dali2rust_domain::dali::pres::standard::StandardCommand::DirectArcPower { level },
    }
    .to_forward_frame()
    .raw()
}

fn last_active_frame(group_id: Option<u8>) -> u16 {
    let address = match group_id {
        Some(group) => dali2rust_domain::dali::types::DaliAddress::group(group).expect("group"),
        None => dali2rust_domain::dali::types::DaliAddress::Broadcast,
    };
    dali2rust_domain::dali::pres::command::DaliCommand::Standard {
        address,
        command: dali2rust_domain::dali::pres::standard::StandardCommand::GoToLastActiveLevel,
    }
    .to_forward_frame()
    .raw()
}

fn sent_frames(world: &DaliWorld) -> Vec<u16> {
    world.dali_mock().lock().expect("mock lock").sent_frames()
}

fn wait_for_frame(world: &mut DaliWorld, frame: u16) {
    let mock = world.dali_mock().clone();
    dali2rust_test_support::wait_until(
        || {
            mock.lock()
                .expect("mock lock")
                .sent_frames()
                .contains(&frame)
        },
        std::time::Duration::from_secs(5),
    );
    assert!(
        sent_frames(world).contains(&frame),
        "frame {frame:#06x} never reached the bus; saw {:#06x?}",
        sent_frames(world)
    );
}

// HCL-050 HCL-057 HCL-063 HCL-051 HCL-053 HCL-054 HCL-056 HCL-060 HCL-061 HCL-062 HCL-064 HCL-077 HCL-078 SYS-237
#[given(regex = r"^the controller clock reads (\d+) minutes past midnight$")]
async fn given_clock_reads(world: &mut DaliWorld, minutes: u64) {
    set_clock(world, minutes);
}

// HCL-051 HCL-053 HCL-061 HCL-064 HCL-060 HCL-077
#[when(regex = r"^the scheduler has run (\d+) more ticks$")]
async fn when_scheduler_ticks(world: &mut DaliWorld, count: u64) {
    let port = world.server_port();
    let target = scheduler_ticks(port) + count;
    dali2rust_test_support::wait_until(
        || scheduler_ticks(port) >= target,
        std::time::Duration::from_secs(10),
    );
    assert!(
        scheduler_ticks(port) >= target,
        "the scheduler did not reach {target} ticks"
    );
}

// HCL-050 HCL-063 HCL-053 HCL-054 HCL-056 HCL-057 SYS-237
#[then(regex = r"^the scheduler should drive group (\d+) to level (\d+)$")]
async fn then_group_driven(world: &mut DaliWorld, group_id: u8, level: u8) {
    wait_for_frame(world, dapc_frame(Some(group_id), level));
}

// HCL-057 HCL-078
#[then(regex = r"^the scheduler should drive broadcast to level (\d+)$")]
async fn then_broadcast_driven(world: &mut DaliWorld, level: u8) {
    wait_for_frame(world, dapc_frame(None, level));
}

// HCL-062
#[then(regex = r"^the scheduler should recall the last active level on group (\d+)$")]
async fn then_recall_last_active(world: &mut DaliWorld, group_id: u8) {
    wait_for_frame(world, last_active_frame(Some(group_id)));
}

// HCL-051 HCL-064 HCL-061
#[then("no DALI frames should have reached the bus")]
async fn then_no_frames(world: &mut DaliWorld) {
    let frames = sent_frames(world);
    assert!(frames.is_empty(), "unexpected DALI traffic: {frames:#06x?}");
}

// HCL-060 HCL-061
#[then("no arc power level should have been driven")]
async fn then_no_dapc(world: &mut DaliWorld) {
    let dapc: Vec<u16> = sent_frames(world)
        .into_iter()
        .filter(|frame| (frame >> 8) & 0x01 == 0)
        .collect();
    assert!(
        dapc.is_empty(),
        "a colour-only point must not touch brightness: {dapc:#06x?}"
    );
}

// HCL-060 HCL-077
#[then("some DALI frames should have reached the bus")]
async fn then_some_frames(world: &mut DaliWorld) {
    assert!(
        !sent_frames(world).is_empty(),
        "the colour half of the point should still be published"
    );
}

// HCL-053
#[then(regex = r"^exactly (\d+) arc power levels? should have been driven$")]
async fn then_dapc_count(world: &mut DaliWorld, expected: usize) {
    let dapc: Vec<u16> = sent_frames(world)
        .into_iter()
        .filter(|frame| (frame >> 8) & 0x01 == 0)
        .collect();
    assert_eq!(dapc.len(), expected, "arc power frames: {dapc:#06x?}");
}

// HCL-072
#[then("the HCL override target list should be empty")]
async fn then_override_targets_empty(world: &mut DaliWorld) {
    let targets = last_json(world)["targets"]
        .as_array()
        .expect("override targets should be an array")
        .clone();
    assert!(
        targets.is_empty(),
        "a running schedule holds no flags: {targets:#?}"
    );
}

// HCL-078
#[then(regex = r#"^HCL schedule "([^"]+)" eventually reports itself suspended$"#)]
async fn then_override_eventually_suspended(world: &mut DaliWorld, schedule_id: String) {
    let port = world.server_port();
    let path = format!("/api/v1/hcl-schedules/{schedule_id}/override");
    dali2rust_test_support::wait_until(
        || {
            crate::steps::physical_devices_steps::fetch_json(port, &path)
                .is_some_and(|json| json["suspended"] == serde_json::Value::Bool(true))
        },
        std::time::Duration::from_secs(10),
    );
    let json = crate::steps::physical_devices_steps::fetch_json(port, &path)
        .expect("override read should answer");
    assert_eq!(
        json["suspended"],
        serde_json::Value::Bool(true),
        "a manual level command must stand down a schedule that drives the level: {json}"
    );
}

// HCL-077
#[then(regex = r"^physical device (\d+) should hold a stored colour$")]
async fn then_device_holds_stored_colour(world: &mut DaliWorld, short: u64) {
    let port = world.server_port();
    let colour = || {
        crate::steps::physical_devices_steps::fetch_json(port, "/api/v1/adapters/0/physical-devices")
            .and_then(|json| {
                json["physical_devices"]
                    .as_array()?
                    .iter()
                    .find(|d| d["short_address"].as_u64() == Some(short))
                    .and_then(|d| d["state"]["color_temperature_kelvin"].as_u64())
            })
    };
    dali2rust_test_support::wait_until(|| colour().is_some(), std::time::Duration::from_secs(5));
    assert!(
        colour().is_some(),
        "the schedule's colour must have reached device {short}'s record, or the \
         level command below has no snapshot colour to be mistaken for its own"
    );
}

