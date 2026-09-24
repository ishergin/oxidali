use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_test_support::wait_until;

use crate::DaliWorld;

const PUBLISH_WAIT: Duration = Duration::from_secs(6);

fn payload_json(world: &DaliWorld, topic: &str) -> serde_json::Value {
    let messages = world.mqtt_mock().published_on(topic);
    let last = messages
        .last()
        .unwrap_or_else(|| panic!("no publish on {topic}"));
    serde_json::from_slice(&last.payload)
        .unwrap_or_else(|e| panic!("payload on {topic} is not JSON: {e}"))
}

// MQTT-003 MQTT-004 MQTT-005 MQTT-006 MQTT-007 MQTT-008 MQTT-009 MQTT-010 MQTT-011 MQTT-012 MQTT-013 MQTT-015 MQTT-016 MQTT-017 MQTT-018 MQTT-019 MQTT-001 MQTT-002 MQTT-014 MQTT-020 SET-HA-020 MQTT-024
#[given(regex = r#"^the Home Assistant bridge is enabled with controller id "([^"]*)"$"#)]
async fn enable_bridge(world: &mut DaliWorld, controller_id: String) {
    let body = format!(
        r#"{{"enabled":true,"broker_host":"broker.test","controller_id":"{controller_id}"}}"#
    );
    world.send_http_request(
        "PATCH",
        "/api/v1/settings/home-assistant",
        Some(body.as_bytes()),
        "application/json",
    );
    let status = world.last_response().expect("no response").status;
    assert_eq!(status, 200, "enabling the bridge");
}

// MQTT-011
#[then(regex = r#"^the MQTT session should carry a retained last will on "([^"]*)"$"#)]
async fn will_registered(world: &mut DaliWorld, topic: String) {
    let mock = world.mqtt_mock().clone();
    wait_until(|| mock.last_will().is_some(), PUBLISH_WAIT);
    let will = mock.last_will().expect("a session without a will lies on reset");
    assert_eq!(will.topic, topic);
    assert_eq!(will.payload, b"offline");
    assert!(will.retain, "a non-retained will is invisible to a later client");
}

// MQTT-010 MQTT-011 MQTT-014 MQTT-019 MQTT-001
#[then(regex = r#"^MQTT should have a retained "([^"]*)" on "([^"]*)"$"#)]
async fn retained_text(world: &mut DaliWorld, expected: String, topic: String) {
    let mock = world.mqtt_mock().clone();
    let want = expected.clone();
    let t = topic.clone();
    wait_until(
        move || {
            mock.published_on(&t)
                .last()
                .is_some_and(|m| m.payload == want.as_bytes())
        },
        PUBLISH_WAIT,
    );
    let last = world
        .mqtt_mock()
        .published_on(&topic)
        .pop()
        .unwrap_or_else(|| panic!("no publish on {topic}"));
    assert_eq!(String::from_utf8_lossy(&last.payload), expected);
    assert!(last.retain, "availability must survive a client connecting later");
}

// MQTT-011
#[then(regex = r#"^the birth on "([^"]*)" should precede every discovery config$"#)]
async fn birth_precedes_discovery(world: &mut DaliWorld, topic: String) {
    let published = world.mqtt_mock().published();
    let birth = published
        .iter()
        .position(|m| m.topic == topic && m.payload == b"online")
        .expect("no birth message at all");
    let first_config = published.iter().position(|m| m.topic.ends_with("/config"));
    if let Some(config) = first_config {
        assert!(
            birth < config,
            "a config published before the birth is marked unavailable on arrival"
        );
    }
}

// MQTT-002 MQTT-005 MQTT-007 MQTT-008 MQTT-010 MQTT-013 MQTT-018
#[then(regex = r#"^MQTT should have exactly (\d+) publish(?:es)? on "([^"]*)"$"#)]
async fn publish_count(world: &mut DaliWorld, count: usize, topic: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || mock.published_on(&t).len() >= count, PUBLISH_WAIT);
    assert_eq!(
        world.mqtt_mock().published_on(&topic).len(),
        count,
        "one publish per registry commit — no more, no fewer"
    );
}

// MQTT-001 MQTT-002 MQTT-005 MQTT-007 MQTT-009 MQTT-013 MQTT-017 MQTT-018 MQTT-024
#[then(regex = r#"^the MQTT payload on "([^"]*)" should have string field "([^"]*)" = "([^"]*)"$"#)]
async fn payload_string(world: &mut DaliWorld, topic: String, field: String, expected: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    let at = field.split('.').fold(&v, |node, key| &node[key]);
    assert_eq!(at.as_str(), Some(expected.as_str()), "field {field}");
}

// MQTT-005 MQTT-007 MQTT-013
#[then(regex = r#"^the MQTT payload on "([^"]*)" should have numeric field "([^"]*)" = (\d+)$"#)]
async fn payload_number(world: &mut DaliWorld, topic: String, field: String, expected: u64) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    assert_eq!(v[&field].as_u64(), Some(expected), "field {field}");
}

// MQTT-003 MQTT-004 MQTT-006 MQTT-012 MQTT-016 MQTT-017 MQTT-018 MQTT-019 MQTT-020
#[when(regex = r#"^Home Assistant publishes (.+) on "([^"]*)"$"#)]
async fn ha_publishes(world: &mut DaliWorld, payload: String, topic: String) {
    world.mqtt_mock().deliver(&topic, payload.as_bytes());
}

// MQTT-003 MQTT-017
#[then(regex = r#"^the virtual lamp (\d+) level on adapter (\d+) should eventually be (\d+)$"#)]
async fn lamp_level_eventually(world: &mut DaliWorld, lamp: u8, adapter: u8, level: u64) {
    let path = format!("/api/v1/adapters/{adapter}/virtual-lamps/{lamp}");
    dali2rust_test_support::wait_until(
        || crate::steps::get_json(world, &path)["state"]["level"].as_u64() == Some(level),
        PUBLISH_WAIT,
    );
}

// MQTT-004 MQTT-006
#[then(regex = r#"^the MQTT unroutable-command count should eventually be at least (\d+)$"#)]
async fn unroutable_at_least(world: &mut DaliWorld, want: u64) {
    dali2rust_test_support::wait_until(
        || {
            crate::steps::get_json(world, "/api/v1/diagnostics")["mqtt"]
                ["commands_unroutable_total"]
                .as_u64()
                .unwrap_or(0)
                >= want
        },
        PUBLISH_WAIT,
    );
}

// MQTT-015
#[then(regex = r#"^the MQTT payload on "([^"]*)" should have boolean field "([^"]*)" = (true|false)$"#)]
async fn payload_boolean(world: &mut DaliWorld, topic: String, field: String, expected: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    assert_eq!(v[&field].as_bool(), Some(expected == "true"), "field {field}");
}

// MQTT-008 MQTT-015
#[then(regex = r#"^the MQTT payload on "([^"]*)" array field "([^"]*)" should contain "([^"]*)"$"#)]
async fn payload_array_contains(world: &mut DaliWorld, topic: String, field: String, want: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    let array = v[&field]
        .as_array()
        .unwrap_or_else(|| panic!("field {field} is not an array: {v:?}"));
    assert!(
        array.iter().any(|item| item.as_str() == Some(want.as_str())),
        "field {field} does not contain {want:?}: {array:?}"
    );
}

// MQTT-007
#[then(regex = r#"^the MQTT payload on "([^"]*)" should list availability topic "([^"]*)"$"#)]
async fn payload_lists_availability(world: &mut DaliWorld, topic: String, want: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    let sources = v["availability"]
        .as_array()
        .unwrap_or_else(|| panic!("availability is not an array: {v:?}"));
    assert!(
        sources.iter().any(|s| s["topic"].as_str() == Some(want.as_str())),
        "availability does not list {want:?}: {sources:?}"
    );
}

// MQTT-013
#[then(regex = r#"^the MQTT payload on "([^"]*)" should not have field "([^"]*)"$"#)]
async fn payload_field_absent(world: &mut DaliWorld, topic: String, field: String) {
    let mock = world.mqtt_mock().clone();
    let t = topic.clone();
    wait_until(move || !mock.published_on(&t).is_empty(), PUBLISH_WAIT);
    let v = payload_json(world, &topic);
    assert!(
        v.get(&field).is_none(),
        "field {field} must be absent from the payload: {v:?}"
    );
}

// MQTT-010
#[then(regex = r#"^the MQTT broker should eventually observe (\d+) connects$"#)]
async fn broker_connect_count(world: &mut DaliWorld, want: u32) {
    let mock = world.mqtt_mock().clone();
    wait_until(move || mock.connect_calls() >= want, PUBLISH_WAIT);
    assert_eq!(world.mqtt_mock().connect_calls(), want, "broker connect count");
}

// MQTT-016 MQTT-019 MQTT-020 MQTT-017 RULE-025
#[then("the scripted DALI exchanges should eventually be consumed")]
async fn scripted_exchanges_eventually_consumed(world: &mut DaliWorld) {
    let mock = world.dali_mock().clone();
    wait_until(
        move || mock.lock().expect("mock lock").scripted_exchanges_remaining() == 0,
        PUBLISH_WAIT,
    );
    let mock = world.dali_mock().lock().expect("mock lock");
    assert_eq!(
        mock.scripted_exchanges_remaining(),
        0,
        "scripted exchanges left unconsumed"
    );
    assert_eq!(mock.script_error(), None, "script error");
}

// MQTT-009
#[then(regex = r#"^the last operation result should report at least (\d+) published entit(?:y|ies)$"#)]
async fn operation_result_published_entities(world: &mut DaliWorld, want: u64) {
    let json = crate::steps::last_json(world);
    let published = json
        .pointer("/result/entities_published")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| panic!("no entities_published in operation view: {json:?}"));
    assert!(
        published >= want,
        "expected at least {want} published entities, got {published}: {json:?}"
    );
}
