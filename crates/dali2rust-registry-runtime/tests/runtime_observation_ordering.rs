mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    DeviceType, LightSetpoint, PowerState, RegistryRuntimeUpdateCommand, RuntimeObservation,
    RuntimeRegistryUpdateEntry, RuntimeSource, OBSERVATION_ORDER_WINDOW_MS,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_test_support::wait_until;

use support::{publish_cmd, recv_confirm_for, seed_physical_via_discovery, spawn_registry_stack};

const SHORT: u8 = 7;

fn level_setpoint(level: u8) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level,
        color: None,
    }
}

fn runtime_entry(level: u8, source: RuntimeSource, stamp: Option<u32>) -> RuntimeRegistryUpdateEntry {
    RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(SHORT),
        setpoint: Some(level_setpoint(level)),
        observation: Some(RuntimeObservation::timestamped(source, 1_000)),
        last_dapc_source: None,
        source,
        observed_at_mono_ms: stamp,
    }
}

fn publish_runtime(
    stack: &support::RegistryTestStack,
    corr: u64,
    level: u8,
    source: RuntimeSource,
    stamp: Option<u32>,
) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            RegistryRuntimeUpdateCommand::internal(0, runtime_entry(level, source, stamp)),
        ),
    );
}

fn assert_ok(c: &dali2rust_contracts::msg::ConfirmationEnvelope) {
    assert_eq!(
        c.status,
        dali2rust_contracts::msg::DeliveryStatus::Ok,
        "got {:?}",
        c.confirmation.error
    );
}

fn stored_level(stack: &support::RegistryTestStack) -> Option<u8> {
    stack.store.physical_device_view(0, SHORT).and_then(|v| v.state.level)
}

#[test]
fn a_read_that_finished_late_does_not_overwrite_a_fresher_observation() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    publish_runtime(&stack, 101, 200, RuntimeSource::Sniffer, Some(3_000));
    assert_ok(&recv_confirm_for(&stack.conf_rx, 101));
    wait_until(|| stored_level(&stack) == Some(200), Duration::from_millis(500));
    while stack.ev_rx.recv_timeout(Duration::from_millis(100)).is_ok() {}

    publish_runtime(&stack, 102, 90, RuntimeSource::Poller, Some(2_000));
    let confirmation = recv_confirm_for(&stack.conf_rx, 102);

    assert_eq!(
        stored_level(&stack),
        Some(200),
        "a stale observation must not roll the level back"
    );
    assert_ok(&confirmation);
    assert!(
        stack.ev_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "a superseded fact is not a commit, so it publishes no state-changed event"
    );
    assert_eq!(
        stack
            .counters
            .command
            .runtime_updates_superseded
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "and it must be visible — a guard silent at zero cannot be told from a dead one"
    );
}

#[test]
fn unstamped_facts_keep_arrival_order_and_do_not_erase_the_stored_key() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    publish_runtime(&stack, 111, 120, RuntimeSource::Poller, Some(5_000));
    assert_ok(&recv_confirm_for(&stack.conf_rx, 111));
    wait_until(|| stored_level(&stack) == Some(120), Duration::from_millis(500));

    publish_runtime(&stack, 112, 60, RuntimeSource::Api, None);
    assert_ok(&recv_confirm_for(&stack.conf_rx, 112));
    wait_until(|| stored_level(&stack) == Some(60), Duration::from_millis(500));

    publish_runtime(&stack, 113, 30, RuntimeSource::Poller, Some(4_000));
    let _ = recv_confirm_for(&stack.conf_rx, 113);
    assert_eq!(
        stored_level(&stack),
        Some(60),
        "the ordering context must survive an unstamped winner"
    );
}

#[test]
fn a_long_quiet_record_still_accepts_a_fresh_command() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    let long_ago = OBSERVATION_ORDER_WINDOW_MS + 10_000;
    publish_runtime(&stack, 121, 200, RuntimeSource::Poller, Some(long_ago));
    assert_ok(&recv_confirm_for(&stack.conf_rx, 121));
    wait_until(|| stored_level(&stack) == Some(200), Duration::from_millis(500));

    publish_runtime(&stack, 122, 45, RuntimeSource::Api, Some(0));
    assert_ok(&recv_confirm_for(&stack.conf_rx, 122));
    wait_until(|| stored_level(&stack) == Some(45), Duration::from_millis(500));
}
