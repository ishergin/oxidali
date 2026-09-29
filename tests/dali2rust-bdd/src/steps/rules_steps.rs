use cucumber::when;
use dali2rust_contracts::msg::MAX_RULES_SOURCE_BYTES;

use crate::DaliWorld;

// RULE-009
#[when("I PUT an oversized rules document")]
async fn when_put_oversized_document(world: &mut DaliWorld) {
    let mut source = String::from("# ");
    source.push_str(&"x".repeat(MAX_RULES_SOURCE_BYTES - 1));
    let body = serde_json::json!({ "base_revision": 0, "source": source }).to_string();
    world.send_http_request("PUT", "/api/v1/rules", Some(body.as_bytes()), "application/json");
}

use std::time::Duration;

use cucumber::then;
use dali2rust_test_support::sync::wait_until;

// RULE-020 RULE-021 RULE-023 RULE-024 RULE-027 RULE-032
#[then(regex = r#"^within (\d+) seconds the stats pointer "([^"]+)" reaches (\d+)$"#)]
async fn then_stats_pointer_reaches(world: &mut DaliWorld, secs: u64, pointer: String, expected: u64) {
    let port = world.server_port();
    let seen = std::cell::RefCell::new(0u64);
    wait_until(
        || {
            let value = crate::steps::physical_devices_steps::fetch_json(port, "/api/v1/stats")
                .and_then(|json| json.pointer(&pointer).and_then(serde_json::Value::as_u64))
                .unwrap_or(0);
            *seen.borrow_mut() = value;
            value >= expected
        },
        Duration::from_secs(secs),
    );
    assert!(
        *seen.borrow() >= expected,
        "stats{pointer}: expected >= {expected} within {secs}s, last saw {}",
        seen.borrow()
    );
}

// RULE-020
#[then("the mock transport should have sent a broadcast off frame")]
async fn then_broadcast_off_sent(world: &mut DaliWorld) {
    let mock = world.dali_mock();
    let guard = mock.lock().expect("mock lock");
    let frames = guard.sent_frames();
    assert!(
        frames.iter().any(|f| *f == 0xFE00),
        "expected broadcast off (DAPC 0, 0xFE00) among {frames:04X?}"
    );
}

const RULE_FIRE_TIMEOUT: Duration = Duration::from_secs(5);

fn rule_fire_count(port: u16, name: &str) -> Option<u64> {
    let json = crate::steps::physical_devices_steps::fetch_json(port, "/api/v1/rules?format=json")?;
    json.pointer("/rules/rules")?
        .as_array()?
        .iter()
        .find(|rule| rule["name"].as_str() == Some(name))?
        .pointer("/runtime/fire_count")?
        .as_u64()
}

// RULE-030 RULE-034 RULE-035 RULE-037 RULE-038
#[then(regex = r#"^the rule "([^"]+)" eventually has fired (\d+) times?$"#)]
async fn then_rule_eventually_fired(world: &mut DaliWorld, name: String, expected: u64) {
    let port = world.server_port();
    wait_until(
        || rule_fire_count(port, &name).is_some_and(|count| count >= expected),
        RULE_FIRE_TIMEOUT,
    );
    assert_eq!(rule_fire_count(port, &name), Some(expected), "fire count of rule {name}");
}

// RULE-030 RULE-034 RULE-035 RULE-037 RULE-038
#[then(regex = r#"^the rule "([^"]+)" should have fired (\d+) times?$"#)]
async fn then_rule_has_fired(world: &mut DaliWorld, name: String, expected: u64) {
    let port = world.server_port();
    assert_eq!(rule_fire_count(port, &name), Some(expected), "fire count of rule {name}");
}

// RULE-037 RULE-038
#[then(regex = r#"^the rule "([^"]+)" eventually has fired at least (\d+) times?$"#)]
async fn then_rule_eventually_fired_at_least(world: &mut DaliWorld, name: String, expected: u64) {
    let port = world.server_port();
    wait_until(
        || rule_fire_count(port, &name).is_some_and(|count| count >= expected),
        RULE_FIRE_TIMEOUT,
    );
    let count = rule_fire_count(port, &name);
    assert!(count.is_some_and(|count| count >= expected), "fire count of rule {name}: {count:?}");
}
