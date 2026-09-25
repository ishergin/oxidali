use cucumber::{given, then, when};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::{json, Value};

use crate::steps::physical_devices_steps::{
    script_discovery, script_discovery_with_features, wait_for_operation_status,
};
use crate::{DaliWorld};
use crate::steps::{assert_result_skips_lamp, last_json};

fn patch_group_name(world: &mut DaliWorld, group_id: u8, name: &str) {
    let path = format!("/api/v1/adapters/0/groups/{group_id}");
    let body = serde_json::to_vec(&json!({ "name": name })).expect("group patch body");
    world.send_http_request("PATCH", &path, Some(&body), "application/json");
    assert_eq!(
        world.last_response().expect("group patch response").status,
        200,
        "group patch for {group_id} should succeed"
    );
}

fn patch_membership_row(world: &mut DaliWorld, virtual_lamp_id: u8, desired_groups_mask: u16) {
    let desired: [bool; 16] = std::array::from_fn(|idx| desired_groups_mask & (1u16 << idx) != 0);
    let body = serde_json::to_vec(&json!({
        "rows": [{
            "virtual_lamp_id": virtual_lamp_id,
            "desired": desired
        }]
    }))
    .expect("matrix patch body");
    world.send_http_request(
        "PATCH",
        "/api/v1/adapters/0/group-membership-matrix",
        Some(&body),
        "application/json",
    );
    crate::steps::await_config_write(world, &format!("matrix patch for vl{virtual_lamp_id}"));
}

fn group_direct_arc_frame(group_id: u8, level: u8) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::group(group_id).expect("valid group address"),
        command: StandardCommand::DirectArcPower { level },
    }
    .to_forward_frame()
    .raw()
}

pub(crate) fn script_group_membership_add(mock: &MockDaliTransport, short: u8, group_id: u8) {
    let address = DaliAddress::short(short).expect("short address");
    let add = DaliCommand::Standard {
        address,
        command: StandardCommand::AddToGroup { group: group_id },
    };
    let frame = add.to_forward_frame().raw();
    mock.expect_forward_frame(frame);
    mock.expect_forward_frame(frame);
    let mask = 1u16 << group_id;
    let q0 = DaliCommand::Standard {
        address,
        command: StandardCommand::QueryGroups0To7,
    };
    let q1 = DaliCommand::Standard {
        address,
        command: StandardCommand::QueryGroups8To15,
    };
    mock.expect_forward_frame_with_backward(q0.to_forward_frame().raw(), Some((mask & 0xFF) as u8));
    mock.expect_forward_frame_with_backward(q1.to_forward_frame().raw(), Some((mask >> 8) as u8));
}

// GRP-030 GRP-063 REG-030 SYS-210 MQTT-017 SYS-241
#[given(regex = r"^adapter 0 group add for short (\d+) group (\d+) is scripted$")]
async fn given_group_add_scripted(world: &mut DaliWorld, short: u8, group_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_group_membership_add(&mock, short, group_id);
}

fn script_group_add_with_mask(mock: &MockDaliTransport, short: u8, group_id: u8, mask: u16) {
    let address = DaliAddress::short(short).expect("short address");
    let add = DaliCommand::Standard {
        address,
        command: StandardCommand::AddToGroup { group: group_id },
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(add);
    mock.expect_forward_frame(add);
    let q0 = DaliCommand::Standard {
        address,
        command: StandardCommand::QueryGroups0To7,
    };
    let q1 = DaliCommand::Standard {
        address,
        command: StandardCommand::QueryGroups8To15,
    };
    mock.expect_forward_frame_with_backward(q0.to_forward_frame().raw(), Some((mask & 0xFF) as u8));
    mock.expect_forward_frame_with_backward(q1.to_forward_frame().raw(), Some((mask >> 8) as u8));
}

// MQTT-018
#[given(regex = r"^adapter 0 group adds for short (\d+) groups (\d+) and (\d+) are scripted$")]
async fn given_two_group_adds_scripted(world: &mut DaliWorld, short: u8, first: u8, second: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let first_mask = 1u16 << first;
    script_group_add_with_mask(&mock, short, first, first_mask);
    script_group_add_with_mask(&mock, short, second, first_mask | (1u16 << second));
}

// MQTT-018
#[given(regex = r"^adapter 0 desired membership includes virtual lamp (\d+) in groups (\d+) and (\d+)$")]
async fn given_desired_membership_two_groups(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    first: u8,
    second: u8,
) {
    patch_membership_row(world, virtual_lamp_id, (1u16 << first) | (1u16 << second));
}

// MQTT-017 MQTT-018
#[given(regex = r"^a group direct-arc script for level (\d+) on group (\d+)$")]
async fn given_group_direct_arc_script(world: &mut DaliWorld, level: u8, group_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(group_direct_arc_frame(group_id, level));
}

// RULE-025
#[given(regex = r"^a group last-active-level script for group (\d+)$")]
async fn given_group_last_active_script(world: &mut DaliWorld, group_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: DaliAddress::group(group_id).expect("valid group address"),
            command: StandardCommand::GoToLastActiveLevel,
        }
        .to_forward_frame()
        .raw(),
    );
}

// MQTT-018
#[given(regex = r"^a group off script for group (\d+)$")]
async fn given_group_off_script(world: &mut DaliWorld, group_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(group_direct_arc_frame(group_id, 0));
}

const CCT_3000K_MIREK: u16 = 333;

fn expect_group_cct_staging_and_activate(
    mock: &dali2rust_adapters::dali::transport::mock::MockDaliTransport,
    group_id: u8,
    mirek: u16,
) -> DaliAddress {
    use dali2rust_domain::dali::pres::special::SpecialCommand;
    const DT8_SET_TEMPERATURE_TC_OPCODE: u8 = 231;
    const DT8_ACTIVATE_OPCODE: u8 = 226;

    let enable_dt8 = DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
        .to_forward_frame()
        .raw();
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0((mirek & 0x00FF) as u8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr1((mirek >> 8) as u8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(enable_dt8);
    let group = DaliAddress::group(group_id).expect("valid group address");
    let group_command_byte = u16::from(group.encode_address_byte() | 0x01) << 8;
    mock.expect_forward_frame(group_command_byte | u16::from(DT8_SET_TEMPERATURE_TC_OPCODE));
    mock.expect_forward_frame(enable_dt8);
    mock.expect_forward_frame(group_command_byte | u16::from(DT8_ACTIVATE_OPCODE));
    group
}

// MQTT-016 MQTT-020
#[given(regex = r"^a group cct 3000K target-state script for group (\d+)$")]
async fn given_group_cct_script(world: &mut DaliWorld, group_id: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    expect_group_cct_staging_and_activate(&mock, group_id, CCT_3000K_MIREK);
}

// MQTT-020
#[given(regex = r"^a group cct 3000K power-on target-state script for group (\d+)$")]
async fn given_group_cct_power_on_script(world: &mut DaliWorld, group_id: u8) {
    use dali2rust_domain::dali::pres::standard::StandardCommand;
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    let group = expect_group_cct_staging_and_activate(&mock, group_id, CCT_3000K_MIREK);
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: group,
            command: StandardCommand::GoToLastActiveLevel,
        }
        .to_forward_frame()
        .raw(),
    );
}

// GRP-001
#[given("adapter 0 has groups 1 and 7 configured")]
async fn given_groups_configured(world: &mut DaliWorld) {
    patch_group_name(world, 1, "Group 1");
    patch_group_name(world, 7, "Group 7");
}

// GRP-020
#[given(regex = r#"^group (\d+) exists with name "([^"]+)"$"#)]
async fn given_group_exists_with_name(world: &mut DaliWorld, group_id: u8, name: String) {
    patch_group_name(world, group_id, &name);
}

// GRP-030 REG-030 GRP-063 GRP-066 GRP-070 GRP-072 OP-130 OP-131 SYS-210 SYS-211 SYS-212 SYS-213 SYS-214 SYS-215 VL-010 VL-020 VL-025 VL-054 SCN-030 SCN-040 SCN-043 SCN-060 SCN-062 SCN-063 SCN-065 SCN-080 REG-031 VL-034 VL-035 MQTT-002 MQTT-008 MQTT-009 MQTT-016 MQTT-017 MQTT-018 SCN-083 SCN-084 SCN-085 SYS-241 MQTT-020 POL-006 POL-008 POL-030 POL-031 SYS-238 VL-100 ADP-026
#[given("adapter 0 has a discovered and bound virtual lamp 1 on physical device 0")]
async fn given_discovered_and_bound_vl1(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_discovery(&mock);
    }
    bind_discovered_vl1(world);
}

const DT8_FEATURES_NO_COLOUR: u8 = 0x80;

// GRP-073
#[given("adapter 0 has a discovered and bound virtual lamp 1 with no confirmed colour")]
async fn given_discovered_and_bound_vl1_without_colour(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_discovery_with_features(&mock, DT8_FEATURES_NO_COLOUR);
    }
    bind_discovered_vl1(world);
}

pub(crate) fn bind_discovered_vl1(world: &mut DaliWorld) {
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(br#"{"mode":"scan_known_short_addresses"}"#),
        "application/json",
    );
    wait_for_operation_status(world, "succeeded");

    let body =
        serde_json::to_vec(&json!({ "physical_short_address": 0 })).expect("binding request body");
    world.send_http_request(
        "PUT",
        "/api/v1/adapters/0/virtual-lamps/1/binding",
        Some(&body),
        "application/json",
    );
    assert_eq!(
        world.last_response().expect("binding response").status,
        200,
        "binding virtual lamp 1 should succeed"
    );
    world.dali_mock().lock().expect("mock lock").clear();
}

// GRP-030 GRP-063 GRP-070 GRP-072 OP-131 REG-030 SYS-210 MQTT-002 MQTT-016 MQTT-017 SYS-241 GRP-073 MQTT-020
#[given(regex = r"^adapter 0 desired membership includes virtual lamp (\d+) in group (\d+)$")]
#[when(regex = r"^adapter 0 desired membership includes virtual lamp (\d+) in group (\d+)$")]
async fn given_desired_membership_includes(world: &mut DaliWorld, virtual_lamp_id: u8, group_id: u8) {
    patch_membership_row(world, virtual_lamp_id, 1u16 << group_id);
}

// GRP-066
#[given("adapter 0 desired membership includes virtual lamp 1 in all 16 groups")]
async fn given_desired_membership_vl1_all_groups(world: &mut DaliWorld) {
    patch_membership_row(world, 1, 0xFFFF);
}

// GRP-066
#[given("adapter 0 desired membership includes virtual lamps 2 through 63 in all 16 groups")]
async fn given_desired_membership_bulk_all_groups(world: &mut DaliWorld) {
    let desired = [true; 16];
    let rows: Vec<Value> = (2u8..=63)
        .map(|vl| json!({ "virtual_lamp_id": vl, "desired": desired }))
        .collect();
    let body = serde_json::to_vec(&json!({ "rows": rows })).expect("bulk matrix patch body");
    world.send_http_request(
        "PATCH",
        "/api/v1/adapters/0/group-membership-matrix",
        Some(&body),
        "application/json",
    );
    crate::steps::await_config_write(world, "bulk matrix patch");
}

// GRP-066
#[given("cumulative group adds for short 0 across all 16 groups are scripted")]
async fn given_cumulative_group_adds_scripted(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let address = DaliAddress::short(0).expect("short address");
    let mut mask = 0u16;
    for group_id in 0..16u8 {
        mask |= 1u16 << group_id;
        let add = DaliCommand::Standard {
            address,
            command: StandardCommand::AddToGroup { group: group_id },
        }
        .to_forward_frame()
        .raw();
        mock.expect_forward_frame(add);
        mock.expect_forward_frame(add);
        let q0 = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryGroups0To7,
        }
        .to_forward_frame()
        .raw();
        let q1 = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryGroups8To15,
        }
        .to_forward_frame()
        .raw();
        mock.expect_forward_frame_with_backward(q0, Some((mask & 0xFF) as u8));
        mock.expect_forward_frame_with_backward(q1, Some((mask >> 8) as u8));
    }
}

// GRP-066
#[then("the group-apply operation result should count 16 programmed and 992 skipped outcomes")]
async fn then_group_apply_counts_paced_outcomes(world: &mut DaliWorld) {
    let json = last_json(world);
    let result = json
        .get("result")
        .unwrap_or_else(|| panic!("result in operation view: {json:?}"));
    assert_eq!(
        result.get("programmed_total").and_then(Value::as_u64),
        Some(16),
        "result: {result:?}"
    );
    assert_eq!(
        result.get("skipped_total").and_then(Value::as_u64),
        Some(992),
        "result: {result:?}"
    );
    assert_eq!(
        result.get("failed_total").and_then(Value::as_u64),
        Some(0),
        "result: {result:?}"
    );
    let skipped_rows = result
        .get("skipped")
        .and_then(Value::as_array)
        .expect("skipped rows");
    assert_eq!(skipped_rows.len(), 128, "detail rows are capped");
}

// REG-030
#[when(regex = r"^adapter 0 desired membership excludes virtual lamp (\d+) from group (\d+)$")]
async fn when_desired_membership_excludes(world: &mut DaliWorld, virtual_lamp_id: u8, _group_id: u8) {
    patch_membership_row(world, virtual_lamp_id, 0);
}

// GRP-001
#[then("the groups list should contain groups 1 and 7")]
async fn then_groups_list_contains(world: &mut DaliWorld) {
    let json = last_json(world);
    let groups = json
        .get("groups")
        .and_then(Value::as_array)
        .expect("groups array");
    let ids: Vec<u64> = groups
        .iter()
        .filter_map(|group| group.get("group_id").and_then(Value::as_u64))
        .collect();
    assert!(
        ids.contains(&1) && ids.contains(&7),
        "expected groups 1 and 7, got {json:?}"
    );
}

// GRP-030
#[then("the group membership matrix should expose 16 group columns")]
async fn then_matrix_exposes_group_columns(world: &mut DaliWorld) {
    let json = last_json(world);
    let groups = json
        .get("groups")
        .and_then(Value::as_array)
        .expect("groups array");
    assert_eq!(groups.len(), 16, "expected 16 group columns, got {json:?}");
    let rows = json.get("rows").and_then(Value::as_array).expect("rows array");
    let first_row = rows.first().expect("at least one row");
    let desired = first_row
        .get("desired")
        .and_then(Value::as_array)
        .expect("desired array");
    let applied = first_row
        .get("applied")
        .and_then(Value::as_array)
        .expect("applied array");
    assert_eq!(desired.len(), 16, "expected desired[16], got {json:?}");
    assert_eq!(applied.len(), 16, "expected applied[16], got {json:?}");
}

// GRP-030 REG-030 GRP-063 VL-036 VL-102
#[then(
    regex = r"^the group membership matrix should show virtual lamp (\d+) desired group (\d+) as (true|false) and applied group \d+ as (true|false)$"
)]
async fn then_matrix_row_membership(
    world: &mut DaliWorld,
    virtual_lamp_id: u8,
    group_id: usize,
    desired: String,
    applied: String,
) {
    let json = last_json(world);
    let rows = json.get("rows").and_then(Value::as_array).expect("rows array");
    let row = rows
        .iter()
        .find(|row| row.get("virtual_lamp_id").and_then(Value::as_u64) == Some(u64::from(virtual_lamp_id)))
        .unwrap_or_else(|| panic!("virtual lamp {virtual_lamp_id} row missing: {json:?}"));
    let desired_values = row
        .get("desired")
        .and_then(Value::as_array)
        .expect("desired array");
    let applied_values = row
        .get("applied")
        .and_then(Value::as_array)
        .expect("applied array");
    assert_eq!(
        desired_values.get(group_id).and_then(Value::as_bool),
        Some(desired == "true"),
        "unexpected desired[{group_id}] for vl{virtual_lamp_id}: {json:?}"
    );
    assert_eq!(
        applied_values.get(group_id).and_then(Value::as_bool),
        Some(applied == "true"),
        "unexpected applied[{group_id}] for vl{virtual_lamp_id}: {json:?}"
    );
}

fn assert_group_dirty(world: &DaliWorld, group_id: u8, expected: bool) {
    let json = last_json(world);
    assert_eq!(
        json.get("group_id").and_then(Value::as_u64),
        Some(u64::from(group_id)),
        "unexpected group payload: {json:?}"
    );
    assert_eq!(
        json.get("dirty").and_then(Value::as_bool),
        Some(expected),
        "group {group_id} dirty should be {expected}: {json:?}"
    );
}

// REG-030
#[then(regex = r"^group (\d+) should be marked dirty$")]
async fn then_group_dirty(world: &mut DaliWorld, group_id: u8) {
    assert_group_dirty(world, group_id, true);
}

// VL-036
#[then(regex = r"^group (\d+) should not be marked dirty$")]
async fn then_group_not_dirty(world: &mut DaliWorld, group_id: u8) {
    assert_group_dirty(world, group_id, false);
}

// GRP-063
#[then(regex = r#"^the group-apply operation result should list skipped virtual lamp (\d+) with reason "([^"]+)"$"#)]
async fn then_group_apply_result_lists_skipped(world: &mut DaliWorld, virtual_lamp_id: u8, reason: String) {
    assert_result_skips_lamp(world, virtual_lamp_id, &reason);
}

// GRP-070
#[then(regex = r"^the DALI mock transport should have sent group (\d+) direct arc level (\d+)$")]
async fn then_group_direct_arc_level(world: &mut DaliWorld, group_id: u8, level: u8) {
    let expected = group_direct_arc_frame(group_id, level);
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_eq!(frames, vec![expected], "unexpected forward frames: {frames:?}");
}
