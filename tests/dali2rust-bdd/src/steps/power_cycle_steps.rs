use std::time::Duration;

use cucumber::{given, then, when};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::{json, Value};

use crate::steps::frames::standard_frame;
use crate::steps::poller_steps::{poller_block, probe_within, wait_for_counter, SETTINGS_PATH};
use crate::steps::polling::fetch_json;
use crate::steps::virtual_lamps_steps::TWO_DEVICE_RANDOM_ADDRESSES;
use crate::DaliWorld;

const WAIT: Duration = Duration::from_secs(10);
const YES: u8 = 0xFF;
const STATUS_POWER_CYCLE_SEEN: u8 = 0x80;
const STATUS_CLEAR: u8 = 0x00;
const LEVEL_OFF: u8 = 0;
const DRIVEN_LEVEL: u8 = 100;
const VERDICT_COUNTERS: [&str; 4] = [
    "health_probes_invalid",
    "health_probes_clear",
    "health_probes_one_failure",
    "health_probes_several_failures",
];
const STATUS_READS_PER_SWEEP: u64 = 2;

fn broadcast_frame(command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command,
    }
    .to_forward_frame()
    .raw()
}

fn probe_frames() -> [u16; 3] {
    [
        broadcast_frame(StandardCommand::QueryControlGearPresent),
        broadcast_frame(StandardCommand::QueryLampFailure),
        broadcast_frame(StandardCommand::QueryPowerFailure),
    ]
}

fn assert_each_probe_is(world: &DaliWorld, probe: &[u16]) {
    let sent = world.dali_mock().lock().expect("mock lock").sent_frames();
    let whole = sent.chunks(probe.len()).filter(|c| c.len() == probe.len()).count();
    assert!(whole >= 2, "at least two whole probes: {sent:#06x?}");
    assert!(
        sent.chunks(probe.len()).all(|chunk| probe.starts_with(chunk)),
        "every probe is {probe:#06x?}, in that order: {sent:#06x?}"
    );
}

fn answer_probe(mock: &MockDaliTransport, control: Option<u8>, power_failure: Option<u8>) {
    let [control_frame, lamp_failure_frame, power_failure_frame] = probe_frames();
    mock.answer_frame(control_frame, control);
    mock.answer_frame(lamp_failure_frame, None);
    mock.answer_frame(power_failure_frame, power_failure);
}

fn answer_status_read(mock: &MockDaliTransport, short: u8, random_address: u32, status: u8) {
    let [high, middle, low, _] = (random_address << 8).to_be_bytes();
    for (command, answer) in [
        (StandardCommand::QueryControlGearPresent, YES),
        (StandardCommand::QueryStatus, status),
        (StandardCommand::QueryActualLevel, LEVEL_OFF),
        (StandardCommand::QueryRandomAddressH, high),
        (StandardCommand::QueryRandomAddressM, middle),
        (StandardCommand::QueryRandomAddressL, low),
    ] {
        mock.answer_frame(standard_frame(short, command), Some(answer));
    }
}

fn answer_with_device_0_status(mock: &MockDaliTransport, device_0_status: u8) {
    for (short, random_address) in TWO_DEVICE_RANDOM_ADDRESSES {
        let status = if short == 0 { device_0_status } else { STATUS_CLEAR };
        answer_status_read(mock, short, random_address, status);
    }
}

fn completed_probes(port: u16) -> u64 {
    let block = poller_block(port);
    VERDICT_COUNTERS
        .iter()
        .map(|name| block[name].as_u64().unwrap_or(0))
        .sum()
}

fn lamp_path(lamp_id: u8) -> String {
    format!("/api/v1/adapters/0/virtual-lamps/{lamp_id}")
}

fn put_target_state(world: &mut DaliWorld, lamp_id: u8, body: &Value) {
    let path = format!("{}/target-state", lamp_path(lamp_id));
    let bytes = serde_json::to_vec(body).expect("target-state body");
    world.send_http_request("PUT", &path, Some(&bytes), "application/json");
    assert_eq!(
        world.last_response().expect("target-state response").status,
        200,
        "virtual lamp {lamp_id} target-state {body}"
    );
}

fn wait_for_lamp_state(world: &DaliWorld, lamp_id: u8, power: &str, level: Option<u8>) {
    let port = world.server_port();
    let path = lamp_path(lamp_id);
    let matches = |json: &Value| {
        json["state"]["power"].as_str() == Some(power)
            && json["state"]["level"].as_u64() == level.map(u64::from)
    };
    let seen = probe_within(WAIT, || fetch_json(port, &path).as_ref().is_some_and(matches));
    assert!(
        seen,
        "virtual lamp {lamp_id} should be {power} at level {level:?}; state was {:#}",
        fetch_json(port, &path)
            .map(|json| json["state"].clone())
            .unwrap_or(Value::Null)
    );
}

fn status_queries_to(world: &DaliWorld, short: u8) -> usize {
    let frame = standard_frame(short, StandardCommand::QueryStatus);
    world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .sent_frames()
        .into_iter()
        .filter(|sent| *sent == frame)
        .count()
}

// PWR-001 PWR-002 PWR-003 PWR-004 PWR-005 PWR-006 PWR-007
#[given(regex = r"^the controller probes the bus every (\d+) ms$")]
async fn given_probe_period(world: &mut DaliWorld, period_ms: u64) {
    world.restart_server_with_probe_period(Duration::from_millis(period_ms));
    let mock = world.dali_mock().lock().expect("mock lock");
    answer_probe(&mock, Some(YES), None);
}

// PWR-001 PWR-002 PWR-003 PWR-004 PWR-005
#[given("the power-cycle watch is switched on")]
async fn given_watch_on(world: &mut DaliWorld) {
    let body = serde_json::to_vec(&json!({ "power_cycle_watch": true })).expect("settings body");
    world.send_http_request("PATCH", SETTINGS_PATH, Some(&body), "application/json");
    assert_eq!(
        world.last_response().expect("settings response").status,
        200,
        "switching the power-cycle watch on"
    );
}

// PWR-002 PWR-003 PWR-004 PWR-005 PWR-006
#[given("virtual lamps 1 and 2 are bound to physical devices 0 and 1")]
async fn given_two_bound_lamps(world: &mut DaliWorld) {
    for (lamp_id, short) in [(1u8, 0u8), (2, 1)] {
        let body = serde_json::to_vec(&json!({ "physical_short_address": short }))
            .expect("binding body");
        let path = format!("{}/binding", lamp_path(lamp_id));
        world.send_http_request("PUT", &path, Some(&body), "application/json");
        assert_eq!(
            world.last_response().expect("binding response").status,
            200,
            "binding virtual lamp {lamp_id} to short address {short}"
        );
    }
    world.dali_mock().lock().expect("mock lock").clear();
}

// PWR-002 PWR-003 PWR-005
#[given("virtual lamps 1 and 2 have been driven to level 100 and switched off")]
#[when("virtual lamps 1 and 2 have been driven to level 100 and switched off")]
async fn drive_and_switch_off(world: &mut DaliWorld) {
    for lamp_id in [1u8, 2] {
        put_target_state(world, lamp_id, &json!({ "power": "on", "level": DRIVEN_LEVEL }));
        wait_for_lamp_state(world, lamp_id, "on", Some(DRIVEN_LEVEL));
        put_target_state(world, lamp_id, &json!({ "power": "off" }));
        wait_for_lamp_state(world, lamp_id, "off", Some(LEVEL_OFF));
    }
}

// PWR-002 PWR-004 PWR-005 PWR-006
#[when("physical device 0 comes back from a power cycle")]
async fn when_device_0_power_cycles(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    answer_with_device_0_status(&mock, STATUS_POWER_CYCLE_SEEN);
    answer_probe(&mock, Some(YES), Some(YES));
}

// PWR-003
#[when("physical device 0 comes back from a power cycle while the probe's control goes unanswered")]
async fn when_device_0_power_cycles_without_control(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    answer_with_device_0_status(&mock, STATUS_POWER_CYCLE_SEEN);
    answer_probe(&mock, None, Some(YES));
}

// PWR-005
#[when("no gear reports a power cycle any more")]
async fn when_no_gear_reports_power_cycle(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    answer_with_device_0_status(&mock, STATUS_CLEAR);
    answer_probe(&mock, Some(YES), None);
}

// PWR-001 PWR-003 PWR-004 PWR-005 PWR-007
#[when(regex = r"^the controller has probed the bus (\d+) more times$")]
async fn when_probed_more_times(world: &mut DaliWorld, count: u64) {
    let port = world.server_port();
    let target = completed_probes(port) + count;
    let reached = probe_within(WAIT, || completed_probes(port) >= target);
    assert!(reached, "the bus was not probed {count} more times");
}

// PWR-002 PWR-004 PWR-005 PWR-006
#[when(regex = r"^the controller has swept the bus for power cycles (\d+) times?$")]
async fn when_swept(world: &mut DaliWorld, sweeps: u64) {
    let port = world.server_port();
    wait_for_counter(port, "power_cycle_sweeps", sweeps);
    wait_for_counter(port, "reads_completed", sweeps * STATUS_READS_PER_SWEEP);
}

// PWR-002 PWR-003 PWR-005
#[when("virtual lamps 1 and 2 are switched on without a level")]
async fn when_switched_on_without_level(world: &mut DaliWorld) {
    for lamp_id in [1u8, 2] {
        put_target_state(world, lamp_id, &json!({ "power": "on" }));
    }
}

// PWR-002 PWR-003 PWR-005
#[then(regex = r"^virtual lamp (\d+) should eventually be on at level (\d+)$")]
async fn then_lamp_on_at_level(world: &mut DaliWorld, lamp_id: u8, level: u8) {
    wait_for_lamp_state(world, lamp_id, "on", Some(level));
}

// PWR-002 PWR-005
#[then(regex = r"^virtual lamp (\d+) should eventually be on at no known level$")]
async fn then_lamp_on_without_level(world: &mut DaliWorld, lamp_id: u8) {
    wait_for_lamp_state(world, lamp_id, "on", None);
}

// PWR-001
#[then("each probe should have broadcast the control, the lamp-failure query and the power-failure query in that order")]
async fn then_probe_frame_order(world: &mut DaliWorld) {
    assert_each_probe_is(world, &probe_frames());
}

// PWR-007
#[then("each probe should have broadcast the control and the lamp-failure query and nothing more")]
async fn then_two_frame_probe(world: &mut DaliWorld) {
    assert_each_probe_is(world, &probe_frames()[..2]);
}

// PWR-006
#[then(regex = r"^no DALI frame should reach the bus within (\d+) ms$")]
async fn then_bus_silent(world: &mut DaliWorld, window_ms: u64) {
    let sent_any = || !world.dali_mock().lock().expect("mock lock").sent_frames().is_empty();
    let spoke = probe_within(Duration::from_millis(window_ms), sent_any);
    assert!(
        !spoke,
        "the bus should stay silent: {:#06x?}",
        world.dali_mock().lock().expect("mock lock").sent_frames()
    );
}

// PWR-003
#[then("no gear should have been asked for its status")]
async fn then_no_status_queries(world: &mut DaliWorld) {
    let asked: Vec<u8> = (0..=63u8).filter(|short| status_queries_to(world, *short) > 0).collect();
    assert!(asked.is_empty(), "short addresses asked for status: {asked:?}");
}

// PWR-004
#[then("every read the poller published should have been a power-cycle sweep read")]
async fn then_only_sweep_reads(world: &mut DaliWorld) {
    let block = poller_block(world.server_port());
    assert_eq!(
        block["reads_published"], block["power_cycle_sweep_reads"],
        "a disabled poller publishes sweep reads only: {block:#}"
    );
}

// PWR-004 PWR-006
#[then("each of physical devices 0 and 1 should have been asked for its status once")]
async fn then_each_device_asked_once(world: &mut DaliWorld) {
    for short in [0u8, 1] {
        assert_eq!(
            status_queries_to(world, short),
            1,
            "physical device {short}: one sweep, one status read"
        );
    }
}
