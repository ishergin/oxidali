use std::time::Duration;

use cucumber::{given, then};
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::last_json;
use crate::DaliWorld;

const STATS_TIMEOUT: Duration = Duration::from_secs(2);

fn stats_snapshot(world: &mut DaliWorld) -> Value {
    super::get_json(world, "/api/v1/stats")
}

fn pointer_u64(json: &Value, pointer: &str) -> u64 {
    json.pointer(pointer)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{pointer} missing or not a number in {json}"))
}

fn bus_publish_totals(world: &mut DaliWorld) -> (u64, u64) {
    let json = super::get_json(world, "/api/v1/diagnostics");
    (
        pointer_u64(&json, "/bus/commands/publish_attempted"),
        pointer_u64(&json, "/bus/events/publish_attempted"),
    )
}

// STATS-001
#[then("the JSON response should have the stats blocks")]
async fn stats_blocks_present(world: &mut DaliWorld) {
    let json = last_json(world);
    for block in ["sample_ms", "controller", "bus", "dali", "operations"] {
        assert!(json.get(block).is_some(), "missing stats block {block}");
    }
    for field in [
        "/bus/commands_published_total",
        "/bus/events_published_total",
        "/bus/commands_ingress_overflow_total",
        "/bus/confirmation_timeouts_total",
        "/dali/commands_executed_total",
        "/dali/errors_total",
        "/dali/target_state_superseded_total",
        "/dali/backward_early_rejected_total",
        "/dali/backward_multi_answer_total",
        "/dali/console_log_dropped_total",
        "/dali/console_log_busy_total",
        "/dali/console_log_truncated_total",
        "/dali/console_log_unavailable_total",
        "/dali/console_uart_errors_total",
        "/operations/running",
        "/operations/succeeded_total",
        "/operations/failed_total",
        "/operations/timed_out_total",
        "/operations/cancelled_total",
    ] {
        assert!(
            json.pointer(field).map(Value::is_u64).unwrap_or(false),
            "stats field {field} must be a number: {json}"
        );
    }
}

// STATS-001
#[then("the stats heap fields should be null on the host build")]
async fn stats_heap_null_on_host(world: &mut DaliWorld) {
    let json = last_json(world);
    for field in [
        "/controller/free_heap_bytes",
        "/controller/internal_free_bytes",
        "/controller/internal_largest_block_bytes",
        "/controller/internal_min_free_bytes",
        "/controller/internal_total_bytes",
        "/controller/internal_allocated_blocks",
        "/controller/rust_internal_live_bytes",
        "/controller/rust_internal_peak_bytes",
        "/controller/rust_psram_live_bytes",
        "/controller/rust_psram_peak_bytes",
    ] {
        assert_eq!(
            json.pointer(field),
            Some(&Value::Null),
            "heap field {field} must be null without a platform port: {json}"
        );
    }
}

// STATS-001
#[then("the stats network block should be null on the host build")]
async fn stats_network_null_on_host(world: &mut DaliWorld) {
    let json = last_json(world);
    assert_eq!(
        json.get("network"),
        Some(&Value::Null),
        "the network block must be present and null without a link: {json}"
    );
}

// STATS-003
#[given("I remember the stats DALI commands executed total")]
async fn remember_dali_executed(world: &mut DaliWorld) {
    let json = stats_snapshot(world);
    world.remembered_u64 = Some(pointer_u64(&json, "/dali/commands_executed_total"));
}

// STATS-003
#[then("the stats DALI commands executed total should have increased")]
async fn dali_executed_increased(world: &mut DaliWorld) {
    let before = world.remembered_u64.expect("remembered executed total");
    let json = stats_snapshot(world);
    let now = pointer_u64(&json, "/dali/commands_executed_total");
    assert!(
        now > before,
        "expected dali.commands_executed_total to grow: before={before} now={now}"
    );
}

// STATS-006
#[then("the stats DALI block should carry the wire load figures")]
async fn stats_dali_wire_load_present(world: &mut DaliWorld) {
    let json = last_json(world);
    let load = pointer_u64(&json, "/dali/wire_load_permille");
    let own = pointer_u64(&json, "/dali/wire_load_own_permille");
    let _foreign = pointer_u64(&json, "/dali/foreign_frames_total");
    assert!(load <= 1000, "wire_load_permille out of range: {load}");
    assert!(own <= load, "own share {own} exceeds total load {load}");
}

// STATS-004
#[then("the stats uptime should eventually be above zero")]
async fn stats_uptime_above_zero(world: &mut DaliWorld) {
    wait_until(
        || {
            let json = stats_snapshot(world);
            pointer_u64(&json, "/controller/uptime_ms") > 0
                && pointer_u64(&json, "/sample_ms") > 0
        },
        STATS_TIMEOUT,
    );
}

// STATS-004
#[then("the controller summary uptime should be above zero")]
async fn controller_uptime_above_zero(world: &mut DaliWorld) {
    world.send_http_request("GET", "/api/v1/controller", None, "");
    let resp = world.last_response().expect("controller response");
    assert_eq!(resp.status, 200, "GET /api/v1/controller");
    let json: Value = serde_json::from_slice(&resp.body).expect("controller body should be JSON");
    assert!(
        pointer_u64(&json, "/uptime_ms") > 0,
        "controller uptime_ms must come from the Clock port: {json}"
    );
}

// STATS-005
#[then(regex = r"^the stats operations succeeded total should be at least (\d+)$")]
async fn stats_operations_succeeded_at_least(world: &mut DaliWorld, expected: u64) {
    let json = last_json(world);
    let succeeded = pointer_u64(&json, "/operations/succeeded_total");
    assert!(
        succeeded >= expected,
        "operations.succeeded_total: expected >= {expected}, got {succeeded}"
    );
}

// STATS-005
#[then("the stats operations running gauge should be 0")]
async fn stats_operations_running_zero(world: &mut DaliWorld) {
    let json = last_json(world);
    let running = pointer_u64(&json, "/operations/running");
    assert_eq!(
        running, 0,
        "every accepted operation reached a terminal status, so nothing is in \
         flight — a non-zero gauge means acceptances and outcomes disagree: {json}"
    );
}

// STATS-010
#[given("I remember the diagnostics bus publish totals")]
async fn remember_bus_publish_totals(world: &mut DaliWorld) {
    world.remembered_pair = Some(bus_publish_totals(world));
}

// STATS-010
#[then("the diagnostics bus publish totals should be unchanged")]
async fn bus_publish_totals_unchanged(world: &mut DaliWorld) {
    let before = world.remembered_pair.expect("remembered publish totals");
    let now = bus_publish_totals(world);
    assert_eq!(
        now, before,
        "reading /api/v1/stats published on the bus: commands/events attempted \
         went from {before:?} to {now:?}"
    );
}

// SYS-251 SYS-252 SYS-253
#[then(regex = r"^the stats dali ([a-z_]+) should eventually be (\d+)$")]
async fn then_stats_dali_counter(world: &mut DaliWorld, field: String, expected: u64) {
    let pointer = format!("/dali/{field}");
    let port = world.server_port();
    wait_until(
        || {
            crate::steps::physical_devices_steps::fetch_json(port, "/api/v1/stats")
                .and_then(|json| json.pointer(&pointer).and_then(Value::as_u64))
                == Some(expected)
        },
        STATS_TIMEOUT,
    );
    let json = stats_snapshot(world);
    assert_eq!(pointer_u64(&json, &pointer), expected, "{json}");
}
