use std::time::Duration;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::steps::physical_devices_steps::fetch_json;
use crate::DaliWorld;

const SETTINGS_PATH: &str = "/api/v1/settings/poller";
const FAST_INTERVAL_MS: u32 = 200;
const WAIT: Duration = Duration::from_secs(10);

fn poller_counter(port: u16, name: &str) -> u64 {
    fetch_json(port, "/api/v1/diagnostics")
        .and_then(|json| json["poller"][name].as_u64())
        .unwrap_or(0)
}

fn probe_within(budget: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    let tripped = std::cell::Cell::new(false);
    dali2rust_test_support::wait_until(
        || {
            if probe() {
                tripped.set(true);
                return true;
            }
            start.elapsed() >= budget
        },
        budget * 2,
    );
    tripped.get()
}

fn poller_block(port: u16) -> Value {
    fetch_json(port, "/api/v1/diagnostics")
        .map(|json| json["poller"].clone())
        .unwrap_or(Value::Null)
}

fn wait_for_counter(port: u16, name: &str, target: u64) {
    let reached = probe_within(WAIT, || poller_counter(port, name) >= target);
    assert!(
        reached,
        "poller counter {name} never reached {target}; whole block was {:#}",
        poller_block(port)
    );
}

// POL-002
#[given("adapter 0 has a discovered but unbound physical device 0")]
async fn given_discovered_unbound_device(world: &mut DaliWorld) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        crate::steps::physical_devices_steps::script_discovery(&mock);
    }
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(br#"{"mode":"scan_known_short_addresses"}"#),
        "application/json",
    );
    crate::steps::physical_devices_steps::wait_for_operation_status(world, "succeeded");
    world.dali_mock().lock().expect("mock lock").clear();
}

// POL-006 POL-008 POL-030 POL-031 SYS-230 SYS-231 SYS-232 SYS-233 SYS-236 SYS-238 SYS-239
#[given(regex = r"^the DALI mock answers every query with (\d+)$")]
async fn given_mock_answers_every_query(world: &mut DaliWorld, value: u8) {
    world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .set_persistent_response(value);
}

// POL-002 POL-006 POL-008 POL-030 POL-031 SYS-238
#[when(regex = r"^the poller is enabled with a (\d+) ms interval$")]
async fn when_poller_enabled(world: &mut DaliWorld, interval_ms: u32) {
    let body = format!(r#"{{"enabled":true,"interval_ms":{interval_ms}}}"#);
    world.send_http_request("PATCH", SETTINGS_PATH, Some(body.as_bytes()), "application/json");
    assert_eq!(
        world.last_response().expect("patch response").status,
        200,
        "enabling the poller should succeed"
    );
}

// POL-002 POL-030
#[when(regex = r"^the poller has run (\d+) more cycles$")]
async fn when_poller_cycles(world: &mut DaliWorld, count: u64) {
    let port = world.server_port();
    let target = poller_counter(port, "cycles_total") + count;
    wait_for_counter(port, "cycles_total", target);
}

// POL-006 POL-008 POL-031 SYS-238
#[when("the poller has completed a read")]
async fn when_poller_completed_read(world: &mut DaliWorld) {
    let port = world.server_port();
    let target = poller_counter(port, "reads_completed") + 1;
    wait_for_counter(port, "reads_completed", target);
}

// POL-002
#[then(regex = r#"^the poller counter "([a-z_]+)" should have increased$"#)]
async fn then_counter_increased(world: &mut DaliWorld, name: String) {
    let port = world.server_port();
    wait_for_counter(port, &name, 1);
}

// POL-002
#[then(regex = r#"^the poller counter "([a-z_]+)" should be (\d+)$"#)]
async fn then_counter_is(world: &mut DaliWorld, name: String, expected: u64) {
    let port = world.server_port();
    assert_eq!(
        poller_counter(port, &name),
        expected,
        "poller counter {name}"
    );
}

// POL-006
#[then("the operations list should contain no poller-created operation")]
async fn then_no_poller_operations(world: &mut DaliWorld) {
    world.send_http_request("GET", "/api/v1/operations", None, "");
    let response = world.last_response().expect("operations response");
    assert_eq!(response.status, 200, "operations list should be readable");
    let json: Value = serde_json::from_slice(&response.body).expect("operations JSON");
    let rows = json["operations"].as_array().cloned().unwrap_or_default();
    let reads: Vec<&Value> = rows
        .iter()
        .filter(|row| row["kind"].as_str() == Some("attribute_read"))
        .collect();
    assert!(
        reads.is_empty(),
        "the poller must not create operation rows; found {reads:#?}"
    );
}

// POL-008
#[then(regex = r#"^virtual lamp (\d+) on adapter (\d+) should eventually report level (\d+) from value_source "([a-z_]+)"$"#)]
async fn then_vl_reports_level_from_source(
    world: &mut DaliWorld,
    lamp_id: u8,
    adapter_id: u8,
    level: u64,
    source: String,
) {
    let port = world.server_port();
    let path = format!("/api/v1/adapters/{adapter_id}/virtual-lamps/{lamp_id}");
    let matches = |json: &Value| {
        json["state"]["level"].as_u64() == Some(level)
            && json["state"]["value_source"].as_str() == Some(source.as_str())
    };
    let seen = probe_within(WAIT, || fetch_json(port, &path).as_ref().is_some_and(matches));
    assert!(
        seen,
        "virtual lamp {lamp_id} should read level {level} from {source}; state was {:#}",
        fetch_json(port, &path)
            .map(|json| json["state"].clone())
            .unwrap_or(Value::Null)
    );
}

// POL-030
#[then("the poller cycle counter should not have gone backwards")]
async fn then_cycles_not_reset(world: &mut DaliWorld) {
    let port = world.server_port();
    assert!(
        poller_counter(port, "cycles_total") >= 2,
        "a restarted worker would have reset the free-running cycle counter"
    );
}

// POL-030
#[then("the poller should run no further cycle within its former interval")]
async fn then_no_cycle_within_former_interval(world: &mut DaliWorld) {
    let port = world.server_port();
    let before = poller_counter(port, "cycles_total");
    let budget = Duration::from_millis(u64::from(FAST_INTERVAL_MS) * 5);
    let cycled = probe_within(budget, || poller_counter(port, "cycles_total") > before);
    assert!(
        !cycled,
        "the stretched interval should have suppressed the next cycle; block was {:#}",
        poller_block(port)
    );
}

// POL-031
#[then("the poller should keep cycling without publishing further reads")]
async fn then_cycles_without_reads(world: &mut DaliWorld) {
    let port = world.server_port();
    let published = poller_counter(port, "reads_published");
    let cycles = poller_counter(port, "cycles_total");
    wait_for_counter(port, "cycles_total", cycles + 3);
    assert_eq!(
        poller_counter(port, "reads_published"),
        published,
        "a disabled poller must publish no further reads"
    );
}

// POL-002
#[then("no addressed DALI frames should have reached the bus")]
async fn then_no_addressed_frames(world: &mut DaliWorld) {
    let addressed: Vec<u16> = world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .sent_frames()
        .into_iter()
        .filter(|frame| (frame >> 8) & 0xFE != 0xFE)
        .collect();
    assert!(
        addressed.is_empty(),
        "unexpected addressed DALI traffic: {addressed:#06x?}"
    );
}
