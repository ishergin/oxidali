use cucumber::{given, then, when};
use serde_json::Value;

use crate::steps::last_json;
use crate::steps::wire::{frame_index, priorities_of, priority_of_settle_us, RELEASE};
use crate::DaliWorld;

// ADP-001 ADP-002 ADP-020 COMM-001 COMM-030 COMM-080 DIAG-030 DIAG-032 GRP-001 GRP-020 GRP-030 GRP-061 GRP-063 GRP-066 HCL-001 HCL-002 HCL-010 HCL-011 HCL-043 HCL-072 HCL-073 HCL-074 HCL-075 HCL-076 INP-010 INP-011 INP-012 INP-016 INP-018 INP-071 INP-073 INP-074 INP-076 INP-079 MQTT-017 MQTT-018 OP-100 OP-101 OP-121 OP-133 PD-035 PD-061 PD-176 PD-200 PD-201 PD-220 PD-221 PD-222 PD-223 PD-230 PD-231 PERS-002 PERS-004 REG-030 REG-031 RULE-001 RULE-003 RULE-004 RULE-006 SCN-001 SCN-010 SCN-011 SCN-012 SCN-020 SCN-030 SCN-040 SCN-041 SCN-046 SCN-050 SCN-060 SCN-061 SCN-062 SCN-063 SCN-065 SCN-080 SCN-081 SCN-082 SCN-083 SCN-084 SCN-085 SCN-092 SCN-093 SET-DALI-001 SET-DALI-002 SET-DALI-005 SET-HA-001 SET-HA-002 SET-HA-010 SET-HA-019 SET-POL-001 SET-POL-002 SET-POL-014 SET-POL-017 SET-POL-019 STATS-001 STATS-002 STATS-004 STATS-005 STATS-010 SYS-001 SYS-002 SYS-004 SYS-007 SYS-008 SYS-009 SYS-010 SYS-011 SYS-012 SYS-013 SYS-014 SYS-015 SYS-016 SYS-050 SYS-210 SYS-211 SYS-213 SYS-216 SYS-241 VL-001 VL-002 VL-010 VL-011 VL-012 VL-013 VL-020 VL-025 VL-034 VL-035 VL-036 WEB-001 WEB-002 WEB-003 WEB-004 VL-100 VL-101 VL-102 VL-103 ADP-026
#[when(regex = r#"^I send a (GET|POST|PUT|DELETE|PATCH) request to "([^"]+)"$"#)]
async fn send_http_request(world: &mut DaliWorld, method: String, path: String) {
    world.send_http_request(&method, &path, None, "");
}

// ADP-021 ADP-022 ADP-023 ADP-024 COMM-001 COMM-004 COMM-007 COMM-008 COMM-010 COMM-030 COMM-032 COMM-033 COMM-034 COMM-035 COMM-036 COMM-038 COMM-052 COMM-055 COMM-056 COMM-057 COMM-080 COMM-081 COMM-082 COMM-083 COMM-084 COMM-087 COMM-088 COMM-089 COMM-090 COMM-091 COMM-092 COMM-093 COMM-094 COMM-095 HCL-020 HCL-022 HCL-051 HCL-054 HCL-056 HCL-057 HCL-060 HCL-061 HCL-062 HCL-063 HCL-064 MQTT-009 MQTT-014 OP-120 PD-030 PD-032 PD-033 PD-034 PD-038 PD-100 PD-101 PD-158 PD-181 PD-182 PD-183 PD-184 PD-185 PD-186 PD-187 PD-188 PD-241 PD-242 PD-243 PD-244 PD-252 PD-253 PD-254 PD-255 RULE-001 RULE-002 RULE-008 RULE-021 RULE-022 SCN-086 SCN-087 SCN-088 SCN-089 SCN-090 SCN-091 SET-HA-020 SYS-233 SYS-236 SYS-239 SYS-241 ADP-025 PD-267 PD-268
#[when(regex = r#"^I POST JSON (.+?) to "([^"]+)"$"#)]
async fn post_json_to_path(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request(
        "POST",
        &path,
        Some(body.as_bytes()),
        "application/json",
    );
}

// COMM-092 GRP-070 GRP-072 GRP-073 MQTT-001 MQTT-002 MQTT-003 MQTT-005 MQTT-007 MQTT-008 MQTT-012 MQTT-013 MQTT-015 MQTT-018 MQTT-019 OP-100 OP-130 OP-131 OP-132 PD-037 PD-040 PD-041 PD-042 PD-043 PD-157 PD-166 PD-168 PD-179 PD-180 PD-250 RULE-003 RULE-004 RULE-005 RULE-006 RULE-020 RULE-021 RULE-022 RULE-023 RULE-024 SCN-051 SYS-210 SYS-235 SYS-240 VL-020 VL-034 VL-035 VL-036 VL-050 VL-051 VL-052 VL-053 VL-054 WS-003 WS-004 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 VL-100 VL-102 MQTT-024
#[when(regex = r#"^I PUT JSON (.+?) to "([^"]+)"$"#)]
async fn put_json_to_path(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request(
        "PUT",
        &path,
        Some(body.as_bytes()),
        "application/json",
    );
}

// ADP-010 ADP-011 ADP-012 ADP-021 ADP-022 ADP-023 ADP-024 GRP-020 GRP-021 HCL-028 HCL-029 HCL-030 HCL-031 HCL-046 HCL-047 HCL-048 HCL-049 INP-013 INP-017 INP-030 INP-031 INP-032 INP-072 INP-075 INP-077 INP-078 INP-080 INP-081 MQTT-008 MQTT-010 PD-027 PD-028 PD-029 PD-035 PD-036 PD-037 PD-060 PD-061 PD-062 PD-063 PD-176 PD-177 PD-192 PD-193 PD-250 PERS-002 PERS-005 POL-030 POL-031 RULE-006 RULE-007 RULE-022 RULE-023 SCN-020 SCN-021 SCN-040 SCN-041 SCN-042 SCN-043 SCN-044 SCN-045 SCN-046 SCN-047 SCN-048 SET-DALI-002 SET-DALI-003 SET-DALI-004 SET-DALI-005 SET-DALI-006 SET-HA-001 SET-HA-010 SET-HA-011 SET-HA-012 SET-HA-013 SET-HA-014 SET-HA-015 SET-HA-016 SET-HA-017 SET-HA-018 SET-HA-019 SET-POL-010 SET-POL-011 SET-POL-012 SET-POL-013 SET-POL-014 SET-POL-015 SET-POL-016 SET-POL-017 SET-POL-018 SET-POL-019 SYS-216 VL-012 VL-020 VL-022 VL-023 VL-024 VL-025 VL-026 ADP-025 ADP-026 POLICY-010 POLICY-011
#[when(regex = r#"^I PATCH JSON (.+?) to "([^"]+)"$"#)]
async fn patch_json_to_path(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request(
        "PATCH",
        &path,
        Some(body.as_bytes()),
        "application/json",
    );
}

// SYS-230 SYS-231 SYS-233 SYS-234
#[when(regex = r#"^I PUT JSON (.+?) to "([^"]+)" in background$"#)]
async fn put_json_to_path_background(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request_background("PUT", &path, Some(body.as_bytes()), "application/json");
}

// COMM-093 COMM-094 COMM-095 SYS-232
#[when(regex = r#"^I POST JSON (.+?) to "([^"]+)" in background$"#)]
async fn post_json_to_path_background(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request_background("POST", &path, Some(body.as_bytes()), "application/json");
}

// PERS-002 SET-POL-017 PERS-004 PERS-005 SET-HA-019 SET-POL-019
#[when("I restart the host stack")]
async fn restart_host_stack(world: &mut DaliWorld) {
    world.restart_server();
}

// ADP-011 ADP-012 GRP-021 GRP-072 OP-121 PD-028 PD-029 PD-032 PD-033 PD-042 PD-043 PD-101 VL-002 VL-013 VL-022 VL-023 VL-024 VL-050 VL-051 VL-052 SET-POL-011 SET-POL-012 SET-POL-013 SET-POL-015 SET-POL-016 VL-034 PD-182 PD-187 SCN-087 SCN-088 SCN-089 SCN-090 SCN-091 COMM-036 HCL-002 HCL-011 HCL-021 HCL-023 HCL-024 HCL-025 HCL-026 HCL-029 HCL-031 HCL-032 HCL-033 HCL-034 HCL-035 HCL-036 HCL-037 HCL-038 HCL-039 HCL-040 HCL-041 HCL-042 HCL-044 HCL-045 HCL-046 HCL-047 HCL-048 HCL-049 HCL-073 HCL-074 PD-061 PD-177 PD-179 PD-180 PD-193 PD-222 PD-223 PD-231 PD-255 SCN-011 SCN-012 SCN-021 SCN-042 SCN-043 SCN-044 SCN-045 SCN-047 SCN-048 SCN-051 SCN-065 SCN-082 SET-DALI-003 SET-DALI-004 SET-DALI-006 SET-HA-011 SET-HA-012 SET-HA-013 SET-HA-014 SET-HA-015 SET-HA-016 SET-HA-017 SYS-216 VL-026 WEB-004 VL-101 VL-103
#[then(regex = r#"^the JSON error should be "([^"]*)"$"#)]
async fn json_error_should_be(world: &mut DaliWorld, code: String) {
    let val = last_json(world);
    assert_eq!(
        val.get("error").and_then(|v| v.as_str()),
        Some(code.as_str()),
        "expected error={code}, got {val:?}"
    );
}

// ADP-002 ADP-010 ADP-024 GRP-020 GRP-063 GRP-066 HCL-010 HCL-027 PD-027 PD-030 PD-038 PD-060 PD-061 PD-062 PD-063 PD-100 PD-181 PD-184 PD-185 PD-186 PD-192 PD-252 PERS-002 PERS-004 SCN-010 SCN-020 SCN-060 SCN-080 SCN-086 SET-HA-001 SET-HA-002 SET-HA-010 SET-HA-018 SET-HA-019 SET-HA-020 VL-010 VL-011 VL-012 VL-020 VL-025
#[then(regex = r#"^the JSON field "([^"]*)" should be "([^"]*)"$"#)]
async fn json_field_string(world: &mut DaliWorld, field: String, expected: String) {
    let val = last_json(world);
    let got = val.get(&field).and_then(|v| v.as_str()).unwrap_or_default();
    assert_eq!(got, expected.as_str(), "field {field}: got {got}");
}

// SET-POL-002 SET-POL-014
#[then(regex = r#"^the JSON field "([^"]*)" should equal (.+)$"#)]
async fn json_field_should_equal(world: &mut DaliWorld, field: String, expected_json: String) {
    let val = last_json(world);
    let expected: Value =
        serde_json::from_str(&expected_json).expect("expected literal must be valid JSON");
    assert_eq!(
        val.get(&field),
        Some(&expected),
        "field {field}: got {val:?}"
    );
}

// OP-120
#[then(regex = r#"^the JSON field "([^"]*)" should be one of "([^"]*)"$"#)]
async fn json_field_one_of(world: &mut DaliWorld, field: String, expected_csv: String) {
    let val = last_json(world);
    let got = val.get(&field).and_then(|v| v.as_str()).unwrap_or_default();
    let expected: Vec<_> = expected_csv.split(',').map(str::trim).collect();
    assert!(
        expected.iter().any(|candidate| *candidate == got),
        "field {field}: got {got}, expected one of {expected:?}"
    );
}

// GRP-063 OP-120 PD-100 SCN-063 SET-HA-020
#[then(regex = r#"^the JSON field "([^"]*)" should be non-empty$"#)]
async fn json_field_nonempty(world: &mut DaliWorld, field: String) {
    let val = last_json(world);
    let s = val
        .get(&field)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(!s.is_empty(), "field {field} should be non-empty, got {val:?}");
}

// GRP-061 VL-011 VL-034 VL-053 HCL-072 HCL-076 PD-063 SCN-061 SET-HA-001
#[then(regex = r#"^the JSON field "([^"]*)" should be absent$"#)]
async fn json_field_absent(world: &mut DaliWorld, field: String) {
    let val = last_json(world);
    assert!(
        val.get(&field).is_none(),
        "expected field {field} absent, got {val:?}"
    );
}

// ADP-002 ADP-010 ADP-021 ADP-024 HCL-010 HCL-072 HCL-075 PD-176 PERS-002 REG-031 SCN-010 SCN-062 SCN-063 SCN-084 SET-DALI-001 SET-DALI-002 SET-DALI-005 SET-HA-001 SET-HA-002 SET-HA-010 SET-POL-001 SET-POL-002 SET-POL-014 SET-POL-018 SET-POL-019
#[then(regex = r#"^the JSON boolean field "([^"]*)" should be (true|false)$"#)]
async fn json_boolean_field(world: &mut DaliWorld, field: String, expected: String) {
    let val = last_json(world);
    assert_eq!(
        val.get(&field).and_then(|v| v.as_bool()),
        Some(expected == "true"),
        "expected field {field} == {expected}, got {val:?}"
    );
}

// ADP-020 SET-POL-002 SET-POL-010 SET-POL-014 SET-POL-017 SCN-010 SET-HA-002
#[then(regex = r#"^the JSON numeric field "([^"]*)" should be (\d+)$"#)]
async fn json_numeric_field(world: &mut DaliWorld, field: String, expected: u64) {
    let val = last_json(world);
    assert_eq!(
        val.get(&field).and_then(|v| v.as_u64()),
        Some(expected),
        "expected field {field} == {expected}, got {val:?}"
    );
}

// INP-010 INP-018 INP-071 INP-076 INP-079 PD-200 PD-201 PD-220 PD-230 RULE-001 RULE-002 RULE-003 RULE-006 STATS-002 VL-010 VL-020 VL-034 VL-035 VL-100
#[then(regex = r#"^the JSON pointer "([^"]*)" should be (\d+)$"#)]
async fn json_pointer_number(world: &mut DaliWorld, pointer: String, expected: u64) {
    let val = last_json(world);
    assert_eq!(
        val.pointer(&pointer).and_then(Value::as_u64),
        Some(expected),
        "expected {pointer} == {expected}, got {val:?}"
    );
}

// INP-011 INP-013 INP-071 INP-073 RULE-001 RULE-002 RULE-004 VL-011
#[then(regex = r#"^the JSON pointer "([^"]*)" should be "([^"]*)"$"#)]
async fn json_pointer_string(world: &mut DaliWorld, pointer: String, expected: String) {
    let val = last_json(world);
    assert_eq!(
        val.pointer(&pointer).and_then(Value::as_str),
        Some(expected.as_str()),
        "expected {pointer} == {expected}, got {val:?}"
    );
}

// INP-010 INP-011 INP-013 INP-071 INP-074 PD-035 PD-036 RULE-001 RULE-004 RULE-006 STATS-002 VL-025
#[then(regex = r#"^the JSON pointer "([^"]*)" should be (true|false)$"#)]
async fn json_pointer_bool(world: &mut DaliWorld, pointer: String, expected: String) {
    let val = last_json(world);
    assert_eq!(
        val.pointer(&pointer).and_then(Value::as_bool),
        Some(expected == "true"),
        "expected {pointer} == {expected}, got {val:?}"
    );
}

// INP-011 INP-079 VL-011
#[then(regex = r#"^the JSON pointer "([^"]*)" should be null$"#)]
async fn json_pointer_null(world: &mut DaliWorld, pointer: String) {
    let val = last_json(world);
    assert_eq!(
        val.pointer(&pointer),
        Some(&Value::Null),
        "expected {pointer} to be null, got {val:?}"
    );
}

// PD-200 PD-201 PD-221
#[then(regex = r#"^the JSON pointer "([^"]*)" should be absent$"#)]
async fn json_pointer_absent(world: &mut DaliWorld, pointer: String) {
    let val = last_json(world);
    assert!(
        val.pointer(&pointer).is_none(),
        "expected {pointer} absent, got {val:?}"
    );
}

// PD-266
#[then(regex = r#"^the JSON pointer "([^"]*)" should be greater than (\d+)$"#)]
async fn json_pointer_greater_than(world: &mut DaliWorld, pointer: String, floor: u64) {
    let val = last_json(world);
    let got = val.pointer(&pointer).and_then(Value::as_u64);
    assert!(
        got.is_some_and(|n| n > floor),
        "expected {pointer} > {floor}, got {got:?} in {val:?}"
    );
}

// PD-200 PD-220 PD-222 PD-230 PD-201 PD-221
#[then(regex = r#"^the JSON pointer "([^"]*)" should be present$"#)]
async fn json_pointer_present(world: &mut DaliWorld, pointer: String) {
    let val = last_json(world);
    assert!(
        val.pointer(&pointer).is_some(),
        "expected {pointer} present, got {val:?}"
    );
}

// DALI-022 DALI-032 DALI-303
#[when(regex = r#"I POST invalid JSON "([^"]*)" to "([^"]+)""#)]
async fn send_post_with_invalid_json(world: &mut DaliWorld, body: String, path: String) {
    world.send_http_request("POST", &path, Some(body.as_bytes()), "application/json");
}

// ADP-001 ADP-002 ADP-010 ADP-011 ADP-012 ADP-020 ADP-021 ADP-022 ADP-023 ADP-024 BUS-013 BUS-014 BUS-016 COMM-001 COMM-004 COMM-007 COMM-008 COMM-010 COMM-030 COMM-032 COMM-033 COMM-034 COMM-035 COMM-036 COMM-038 COMM-052 COMM-055 COMM-056 COMM-057 COMM-080 COMM-081 COMM-082 COMM-083 COMM-084 COMM-087 COMM-088 COMM-089 COMM-090 COMM-091 COMM-092 COMM-093 COMM-094 COMM-095 DALI-001 DALI-002 DALI-003 DALI-004 DALI-008 DALI-009 DALI-012 DALI-013 DALI-014 DALI-015 DALI-016 DALI-017 DALI-020 DALI-021 DALI-022 DALI-023 DALI-030 DALI-031 DALI-032 DALI-033 DALI-040 DALI-041 DALI-050 DALI-051 DALI-052 DALI-053 DALI-110 DALI-111 DALI-112 DALI-113 DALI-200 DALI-300 DALI-302 DALI-303 DALI-304 DALI-305 DALI-306 DALI-307 DIAG-030 DIAG-031 DIAG-032 GRP-001 GRP-020 GRP-021 GRP-030 GRP-061 GRP-063 GRP-066 GRP-070 GRP-072 GRP-073 HCL-001 HCL-002 HCL-010 HCL-011 HCL-020 HCL-021 HCL-022 HCL-023 HCL-024 HCL-025 HCL-026 HCL-027 HCL-028 HCL-029 HCL-030 HCL-031 HCL-032 HCL-033 HCL-034 HCL-035 HCL-036 HCL-037 HCL-038 HCL-039 HCL-040 HCL-041 HCL-042 HCL-043 HCL-044 HCL-045 HCL-046 HCL-047 HCL-048 HCL-049 HCL-051 HCL-054 HCL-056 HCL-057 HCL-060 HCL-061 HCL-062 HCL-063 HCL-064 HCL-072 HCL-073 HCL-074 HCL-075 HCL-076 INP-010 INP-011 INP-012 INP-013 INP-016 INP-017 INP-018 INP-030 INP-031 INP-032 INP-071 INP-072 INP-074 INP-075 INP-077 INP-078 INP-080 INP-081 MQTT-001 MQTT-002 MQTT-003 MQTT-005 MQTT-007 MQTT-008 MQTT-009 MQTT-010 MQTT-012 MQTT-013 MQTT-014 MQTT-015 MQTT-018 MQTT-019 OP-100 OP-101 OP-120 OP-121 OP-130 OP-131 OP-132 OP-133 PD-027 PD-028 PD-029 PD-030 PD-032 PD-033 PD-034 PD-035 PD-036 PD-037 PD-038 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-100 PD-101 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-167 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-178 PD-179 PD-180 PD-181 PD-182 PD-183 PD-184 PD-185 PD-186 PD-187 PD-188 PD-189 PD-191 PD-192 PD-193 PD-194 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-223 PD-230 PD-231 PD-240 PD-241 PD-242 PD-243 PD-244 PD-250 PD-251 PD-252 PD-253 PD-254 PD-255 PERS-002 PERS-004 PERS-005 POL-030 POL-031 REG-030 REG-031 RULE-001 RULE-002 RULE-003 RULE-004 RULE-005 RULE-006 RULE-007 RULE-008 RULE-009 RULE-020 RULE-021 RULE-022 RULE-023 RULE-024 SCN-001 SCN-010 SCN-011 SCN-012 SCN-020 SCN-021 SCN-030 SCN-040 SCN-041 SCN-042 SCN-043 SCN-044 SCN-045 SCN-046 SCN-047 SCN-048 SCN-050 SCN-051 SCN-060 SCN-061 SCN-062 SCN-063 SCN-065 SCN-080 SCN-081 SCN-082 SCN-083 SCN-084 SCN-085 SCN-086 SCN-087 SCN-088 SCN-089 SCN-090 SCN-091 SCN-092 SCN-093 SET-DALI-001 SET-DALI-002 SET-DALI-003 SET-DALI-004 SET-DALI-005 SET-DALI-006 SET-HA-001 SET-HA-002 SET-HA-010 SET-HA-011 SET-HA-012 SET-HA-013 SET-HA-014 SET-HA-015 SET-HA-016 SET-HA-017 SET-HA-018 SET-HA-019 SET-HA-020 SET-POL-001 SET-POL-002 SET-POL-010 SET-POL-011 SET-POL-012 SET-POL-013 SET-POL-014 SET-POL-015 SET-POL-016 SET-POL-017 SET-POL-018 SET-POL-019 STATS-001 STATS-002 STATS-003 STATS-004 STATS-005 STATS-010 SYS-001 SYS-002 SYS-003 SYS-004 SYS-005 SYS-006 SYS-007 SYS-008 SYS-009 SYS-010 SYS-011 SYS-012 SYS-013 SYS-014 SYS-015 SYS-016 SYS-017 SYS-018 SYS-050 SYS-210 SYS-211 SYS-216 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 SYS-241 VL-001 VL-002 VL-010 VL-011 VL-012 VL-013 VL-020 VL-022 VL-023 VL-024 VL-025 VL-026 VL-034 VL-035 VL-036 VL-050 VL-051 VL-052 VL-053 VL-054 WEB-001 WEB-002 WEB-003 WEB-004 WS-003 WS-004 WS-010 WS-013 WS-030 WS-032 WS-040 WS-041 WS-044 WS-045 WS-046 VL-100 VL-101 VL-102 VL-103 MQTT-024 ADP-025 ADP-026 PD-267 PD-268 POLICY-010 POLICY-011
#[then(regex = r#"the response status should be (\d+)"#)]
async fn response_status_should_be(world: &mut DaliWorld, expected: u64) {
    let resp = world.last_response().expect("no response");
    assert_eq!(
        resp.status as u64, expected,
        "expected status {}, got {}",
        expected, resp.status
    );
}

// SYS-001 SYS-004 SYS-007 SYS-010 SYS-012 SYS-013 SYS-002 SYS-050
#[then(regex = r#"the JSON HealthResponse status should be "([^"]+)""#)]
async fn health_response_status(world: &mut DaliWorld, expected_status: String) {
    let resp = world.last_response().expect("no response");
    let health: serde_json::Value =
        serde_json::from_slice(&resp.body).expect("failed to parse JSON HealthResponse");
    let status = health["status"].as_str().expect("status field");
    assert_eq!(
        status, expected_status,
        "expected status '{}', got '{}'",
        expected_status, status
    );
}

// SYS-001 SYS-016
#[then(regex = r"the JSON HealthResponse version should not be empty")]
async fn health_response_version_not_empty(world: &mut DaliWorld) {
    let resp = world.last_response().expect("no response");
    let health: serde_json::Value =
        serde_json::from_slice(&resp.body).expect("failed to parse JSON HealthResponse");
    let version = health["version"].as_str().expect("version field");
    assert!(!version.is_empty(), "version should not be empty");
}

// SYS-050 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234
#[when("I drain the background request")]
async fn when_drain_background(world: &mut DaliWorld) {
    world.drain_background();
}

// GRP-063 OP-120
#[when("I GET the operation from the last JSON response")]
async fn get_operation_from_last_json_response(world: &mut DaliWorld) {
    let json = last_json(world);
    let op_id = json
        .get("operation_id")
        .and_then(|v| v.as_str())
        .expect("operation_id in last JSON response");
    let path = format!("/api/v1/operations/{op_id}");
    world.send_http_request("GET", &path, None, "");
}

// PERS-002 SET-POL-017 PERS-004 PERS-005 SET-HA-019 SET-POL-019
#[given("in-memory slice persistence is enabled for the host stack")]
async fn enable_in_memory_persistence(world: &mut DaliWorld) {
    world.enable_in_memory_persistence();
    world.restart_server();
}


// SYS-235 SYS-236 SYS-237 SYS-238
#[then(regex = r"^the first forward frame should be sent at priority (\d+)$")]
async fn first_forward_frame_priority(world: &mut DaliWorld, expected: u8) {
    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    assert!(
        !settle_us.is_empty(),
        "no forward frames reached the wire, so there is no priority to check"
    );
    let priorities = priorities_of(&settle_us);
    let first = priorities[0];
    assert!(
        first == expected || first == RELEASE,
        "first frame should be priority {expected} (or a §9.2 bus release); \
         whole trace: {priorities:?}"
    );
}

// SYS-236 SYS-238 SYS-237 SYS-240
#[then(regex = r"^no forward frame should be sent at priority ([\d, or]+)$")]
async fn no_forward_frame_at_priorities(world: &mut DaliWorld, spec: String) {
    let forbidden: Vec<u8> = spec
        .split(|c: char| !c.is_ascii_digit())
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.parse().expect("priority digit"))
        .collect();
    assert!(!forbidden.is_empty(), "step matched no priorities to forbid");

    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    assert!(
        !settle_us.is_empty(),
        "no forward frames reached the wire, so this assertion would be vacuous"
    );
    let priorities = priorities_of(&settle_us);
    let offenders: Vec<(usize, u8)> = priorities
        .iter()
        .enumerate()
        .filter(|(_, priority)| forbidden.contains(priority))
        .map(|(index, priority)| (index, *priority))
        .collect();
    assert!(
        offenders.is_empty(),
        "frames {offenders:?} were sent at a forbidden priority ({forbidden:?}); \
         whole trace: {priorities:?}"
    );
}

// SYS-239
#[then("the first forward frame should not be sent at priority 1")]
async fn first_forward_frame_is_not_a_continuation(world: &mut DaliWorld) {
    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let priorities = priorities_of(&settle_us);
    let first = *priorities.first().expect(
        "no forward frame reached the wire, so this assertion would be vacuous",
    );
    assert_ne!(
        first,
        1,
        "the opening frame of a unit carried priority 1, which IEC 62386-103 \
         §9.13.1 forbids with a \"shall\" for a frame that starts a transaction; \
         whole trace: {priorities:?}, frames {frames:02X?}"
    );
}

// SYS-239
#[then("every forward frame after the first should be sent at priority 1")]
async fn later_forward_frames_are_continuations(world: &mut DaliWorld) {
    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let priorities = priorities_of(&settle_us);
    assert!(
        priorities.len() > 1,
        "a unit of one frame proves nothing about continuations; trace: {priorities:?}"
    );
    let offenders: Vec<(usize, u8)> = priorities
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, priority)| **priority != 1)
        .map(|(index, priority)| (index, *priority))
        .collect();
    assert!(
        offenders.is_empty(),
        "frames {offenders:?} continued a transaction at something other than \
         priority 1, so another master was free to precede them; whole trace: \
         {priorities:?}, frames {frames:02X?}"
    );
}

// SYS-240
#[then(regex = r"^the forward frames should be sent at priorities \[([\d, ]+)\]$")]
async fn forward_frames_priority_vector(world: &mut DaliWorld, spec: String) {
    let expected: Vec<u8> = spec
        .split(',')
        .map(|piece| piece.trim().parse().expect("priority digit"))
        .collect();
    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let priorities = priorities_of(&settle_us);
    assert_eq!(
        priorities, expected,
        "priority vector mismatch; frames were {frames:02X?}"
    );
}

// SYS-235
#[then(regex = r"^forward frame 0x([0-9a-fA-F]+) should be sent at priority (\d+)$")]
async fn one_forward_frame_priority(world: &mut DaliWorld, frame_hex: String, expected: u8) {
    let frame = u16::from_str_radix(&frame_hex, 16).expect("hex frame");
    let frames = world.dali_mock().lock().unwrap().sent_frames();
    let settle_us = world.dali_mock().lock().unwrap().sent_frame_settle_us();
    let index = frame_index(&frames, frame);
    let priority = priority_of_settle_us(index, settle_us[index]);
    assert_eq!(
        priority, expected,
        "frame 0x{frame:04X} (index {index}) should be priority {expected}; \
         whole trace: {:?}",
        priorities_of(&settle_us)
    );
}
