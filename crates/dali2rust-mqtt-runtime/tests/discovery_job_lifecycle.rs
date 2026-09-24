mod support;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusId, PublishResult};
use dali2rust_contracts::msg::{BusEventPayload, OperationWorkerSignal};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_test_support::{recv_event_matching, wait_until};
use support::{enabled_settings, spawn_bridge, StubHaReadPort, StubSettings};

const WAIT_S: Duration = Duration::from_secs(8);
const STREAK_WAIT: Duration = Duration::from_secs(12);

fn begin_republish(h: &support::Harness, workflow: u64) {
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::HomeAssistantDiscoveryPublishCommand {},
    );
    assert_eq!(
        h.publisher.try_publish(
            BusChannel::Commands,
            dali2rust_bus::BusFrame::command(envelope)
        ),
        PublishResult::Queued
    );
    let counters = Arc::clone(&h.counters);
    wait_until(
        move || counters.discovery_published_total.load(Ordering::Relaxed) >= 1,
        WAIT_S,
    );
}

fn is_bridge_unavailable_failure(payload: &BusEventPayload, workflow: u64) -> bool {
    let BusEventPayload::OperationWorkerSignalEvent(signal) = payload else {
        return false;
    };
    signal.workflow_correlation_id == workflow
        && signal.signal == OperationWorkerSignal::WorkerFailed
        && signal
            .error
            .as_ref()
            .is_some_and(|e| e.message.as_str() == "ha_bridge_unavailable")
}

#[test]
fn disabling_the_bridge_fails_an_in_flight_republish_instead_of_stranding_its_operation() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(Arc::clone(&settings), read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    begin_republish(&h, 91);

    settings.set(dali2rust_domain::registry::HomeAssistantSettingsView {
        enabled: false,
        ..enabled_settings()
    });

    let refusal = recv_event_matching(&h.ev_obs_rx, WAIT_S, |payload| {
        is_bridge_unavailable_failure(payload, 91)
    });
    assert_eq!(refusal.meta.correlation_id, 91);
    assert!(
        h.counters.discovery_published_total.load(Ordering::Relaxed) < 81,
        "the run must have been cut short, not completed"
    );
}

#[test]
fn a_second_consecutive_refused_dial_fails_a_stalled_republish_job() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(Arc::clone(&settings), read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), WAIT_S);

    begin_republish(&h, 92);

    h.mock.fail_next_connects(8);
    h.mock.set_connected(false);

    let refusal = recv_event_matching(&h.ev_obs_rx, STREAK_WAIT, |payload| {
        is_bridge_unavailable_failure(payload, 92)
    });
    assert_eq!(refusal.meta.correlation_id, 92);
    assert!(
        h.counters.discovery_published_total.load(Ordering::Relaxed) < 81,
        "the run must have been cut short, not completed"
    );
    assert!(
        h.mock.connect_calls() >= 3,
        "the refusal must wait for the second refused dial: {} dials",
        h.mock.connect_calls()
    );
}

#[test]
fn a_bulk_discovery_run_publishes_availability_beside_every_lamp_config_mqtt031() {
    let settings = StubSettings::new(enabled_settings());
    let read_port = StubHaReadPort::with_lamp(1);
    let h = spawn_bridge(settings, read_port);
    let counters = Arc::clone(&h.counters);
    wait_until(move || counters.is_connected(), Duration::from_secs(5));

    begin_republish(&h, 7);

    let mock = Arc::clone(&h.mock);
    wait_until(
        move || !mock.published_on("homeassistant/light/ctl1/a0_vl_1/config").is_empty(),
        Duration::from_secs(5),
    );
    let avail = h.mock.published_on("dali/ctl1/a0/vl/1/availability");
    assert!(
        !avail.is_empty(),
        "the bulk run announced a config that lists an availability topic and never published on it"
    );
    assert!(avail[0].retain, "availability must be retained");
}
