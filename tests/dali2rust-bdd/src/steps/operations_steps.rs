use std::time::Duration;

use cucumber::then;
use serde_json::Value;

use crate::steps::last_json;
use crate::steps::polling::{wait_for_operation_status, wait_for_operation_status_within};
use crate::DaliWorld;

// OP-100 COMM-001 COMM-030
#[then(regex = r#"^the operations list should contain a key starting with "([^"]+)"$"#)]
async fn then_operations_list_has_key_with_prefix(world: &mut DaliWorld, prefix: String) {
    let response = world.last_response().expect("last response");
    let json: Value = serde_json::from_slice(&response.body).expect("response should be JSON");
    let keys = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    assert!(
        keys.iter()
            .filter_map(Value::as_str)
            .any(|key| key.starts_with(&prefix)),
        "no operation key starting with {prefix:?} in {json:?}"
    );
}

// POLICY-009
#[then(regex = r#"^the operations list should contain no key starting with "([^"]+)"$"#)]
async fn then_operations_list_lacks_key_with_prefix(world: &mut DaliWorld, prefix: String) {
    let response = world.last_response().expect("last response");
    let json: Value = serde_json::from_slice(&response.body).expect("response should be JSON");
    let keys = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    let found: Vec<&str> = keys
        .iter()
        .filter_map(Value::as_str)
        .filter(|key| key.starts_with(&prefix))
        .collect();
    assert!(found.is_empty(), "unexpected operations {found:?}");
}

// OP-100 OP-130 OP-131 OP-132 PD-041 COMM-080
#[then(regex = r"^the operations list should contain exactly (\d+) operations?$")]
async fn then_operations_list_count(world: &mut DaliWorld, expected: usize) {
    world.send_http_request("GET", "/api/v1/operations", None, "");
    let json = last_json(world);
    let operations = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    assert_eq!(
        operations.len(),
        expected,
        "unexpected operations list: {json:?}"
    );
}

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 GRP-030 GRP-063 HCL-020 HCL-022 HCL-027 HCL-028 HCL-030 HCL-051 HCL-054 HCL-056 HCL-057 HCL-060 HCL-061 HCL-062 HCL-063 HCL-064 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-009 MQTT-012 MQTT-013 MQTT-014 MQTT-015 MQTT-017 MQTT-018 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-167 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-178 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-189 PD-190 PD-191 PD-192 PD-193 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-241 PD-242 PD-243 PD-250 PD-251 PD-252 PD-253 PD-254 PD-255 PERS-005 REG-030 REG-031 RULE-003 RULE-004 RULE-005 RULE-006 RULE-020 RULE-021 RULE-022 RULE-023 RULE-024 SCN-040 SCN-041 SCN-046 SCN-050 SCN-060 SCN-062 SCN-063 SCN-083 SCN-084 SCN-085 SCN-086 SCN-092 SCN-093 SET-HA-020 STATS-005 SYS-210 SYS-211 SYS-213 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 SYS-241 WS-003 WS-004 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024 PD-267 PD-268 POLICY-010 POLICY-011 ADP-027 COMM-097 COMM-099 SYS-251 SYS-252 SYS-253 PD-271 RULE-027 WS-059 COMM-100 PD-272 PD-273 RULE-030 RULE-031 RULE-032 RULE-033 RULE-034 RULE-035 RULE-036 RULE-037 RULE-038 RULE-039 RULE-060 RULE-061 RULE-062 RULE-063 RULE-064 RULE-065 RULE-066 RULE-067 RULE-068 RULE-069 RULE-070 RULE-080 RULE-081 RULE-082 RULE-083 RULE-084 RULE-085 PD-285
#[then("the last operation eventually succeeds")]
async fn then_last_operation_eventually_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status(world, "succeeded");
}

// COMM-097
#[then("the last operation eventually runs")]
async fn then_last_operation_eventually_runs(world: &mut DaliWorld) {
    wait_for_operation_status(world, "running");
}

const FULL_SEGMENT_OPERATION_TIMEOUT: Duration = Duration::from_secs(90);

// PD-240 COMM-098
#[then("the last operation eventually succeeds within the full-segment budget")]
async fn then_full_segment_operation_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status_within(world, "succeeded", FULL_SEGMENT_OPERATION_TIMEOUT);
}

const GROUP_APPLY_PACING_TIMEOUT: Duration = Duration::from_secs(14);

// GRP-066
#[then("the last operation eventually succeeds within the group-apply pacing budget")]
async fn then_group_apply_operation_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status_within(world, "succeeded", GROUP_APPLY_PACING_TIMEOUT);
}

// ADP-021 ADP-022 ADP-023 INP-077 INP-080 INP-081 PD-103 PD-156 PD-164 PD-169 PD-183 PD-194 RULE-022 SCN-062 SYS-231 SYS-233 ADP-025 ADP-026 PD-267 PD-268 POLICY-010 POLICY-011 COMM-099 COMM-101 HCL-079 INP-096 PD-285
#[then("the last operation eventually fails")]
async fn then_last_operation_eventually_fails(world: &mut DaliWorld) {
    wait_for_operation_status(world, "failed");
}

// ADP-021 ADP-022 ADP-023 PD-164 PD-169 PD-183 SYS-231 SYS-233 ADP-025 ADP-026 PD-267 PD-268 POLICY-010 POLICY-011 COMM-099 COMM-101 HCL-079 PD-285
#[then(regex = r#"^the operation error code should be "(\w+)"$"#)]
async fn then_operation_error_code(world: &mut DaliWorld, code: String) {
    let json = last_json(world);
    assert_eq!(
        json.pointer("/error/code").and_then(Value::as_str),
        Some(code.as_str()),
        "operation view: {json:?}"
    );
}

// ADP-021 ADP-022 ADP-023 INP-077 INP-080 INP-081 PD-183 RULE-022 ADP-025 ADP-026 POLICY-010 POLICY-011 HCL-079 INP-096
#[then(regex = r#"^the operation error message should be "(\w+)"$"#)]
async fn then_operation_error_message(world: &mut DaliWorld, message: String) {
    let json = last_json(world);
    assert_eq!(
        json.pointer("/error/message").and_then(Value::as_str),
        Some(message.as_str()),
        "operation view: {json:?}"
    );
}
