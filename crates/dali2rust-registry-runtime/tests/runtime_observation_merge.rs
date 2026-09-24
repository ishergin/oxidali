mod support;

use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    CompactErrorPayload, DeviceType, ErrorCode, LightSetpoint, PowerState,
    RegistryRuntimeUpdateCommand, RuntimeObservation, RuntimeRegistryUpdateEntry, RuntimeSource,
    StatusFlags,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{PhysicalDeviceStateView};
use dali2rust_test_support::wait_until;

use support::{publish_cmd, recv_confirm_for, seed_physical_via_discovery, spawn_registry_stack};

const SHORT: u8 = 7;

fn lamp_failure_flags() -> StatusFlags {
    StatusFlags {
        raw: 0x02,
        gear_failure: false,
        lamp_failure: true,
        lamp_on: false,
        limit_error: false,
        fade_running: false,
        reset_state: false,
        missing_short_address: false,
        power_cycle_seen: false,
    }
}

fn all_clear_flags() -> StatusFlags {
    StatusFlags {
        raw: 0x00,
        gear_failure: false,
        lamp_failure: false,
        lamp_on: false,
        limit_error: false,
        fade_running: false,
        reset_state: false,
        missing_short_address: false,
        power_cycle_seen: false,
    }
}

fn level_setpoint(level: u8) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level,
        color: None,
    }
}

fn runtime_entry(
    level: u8,
    source: RuntimeSource,
    observation: RuntimeObservation,
) -> RuntimeRegistryUpdateEntry {
    RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(SHORT),
        setpoint: Some(level_setpoint(level)),
        observation: Some(observation),
        last_dapc_source: None,
        source,
        observed_at_mono_ms: None,
    }
}

fn publish_runtime(
    stack: &support::RegistryTestStack,
    corr: u64,
    level: u8,
    source: RuntimeSource,
    observation: RuntimeObservation,
) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            RegistryRuntimeUpdateCommand::internal(0, runtime_entry(level, source, observation)),
        ),
    );
}

fn status_observation(flags: StatusFlags) -> RuntimeObservation {
    RuntimeObservation {
        status_flags: Some(flags),
        failure_status: None,
        value_source: Some(RuntimeSource::Poller),
        last_seen_ms: Some(1_000),
        last_dapc_source: dali2rust_contracts::msg::LastDapcSource::Unknown,
        error: None,
    }
}

fn assert_ok(c: &dali2rust_contracts::msg::ConfirmationEnvelope) {
    assert_eq!(
        c.status,
        dali2rust_contracts::msg::DeliveryStatus::Ok,
        "got {:?}",
        c.confirmation.error
    );
}

fn state(stack: &support::RegistryTestStack) -> PhysicalDeviceStateView {
    stack
        .store
        .physical_device_view(0, SHORT)
        .expect("seeded device")
        .state
}

fn stack_with_read_status() -> support::RegistryTestStack {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    publish_runtime(
        &stack,
        201,
        120,
        RuntimeSource::Poller,
        status_observation(lamp_failure_flags()),
    );
    assert_ok(&recv_confirm_for(&stack.conf_rx, 201));
    wait_until(
        || state(&stack).status.is_some(),
        Duration::from_millis(500),
    );
    assert_eq!(
        state(&stack).status.map(|s| s.raw),
        Some(0x02),
        "the read must have committed the flags the rest of this test depends on"
    );
    stack
}

#[test]
fn a_fact_that_observed_no_status_keeps_the_stored_flags() {
    let stack = stack_with_read_status();

    publish_runtime(
        &stack,
        202,
        180,
        RuntimeSource::Sniffer,
        RuntimeObservation::sniffer_timestamped(2_000),
    );
    assert_ok(&recv_confirm_for(&stack.conf_rx, 202));
    wait_until(
        || state(&stack).level == Some(180),
        Duration::from_millis(500),
    );

    let after = state(&stack);
    assert_eq!(
        after.status.as_ref().map(|s| s.raw),
        Some(0x02),
        "a DAPC frame carries no status byte and must not clear one"
    );
    assert!(
        after.status.as_ref().is_some_and(|s| s.lamp_failure),
        "and the decoded flags must survive with it, not just the raw byte"
    );
    assert_eq!(
        after.last_seen_ms,
        Some(2_000),
        "what the fact DID observe still applies"
    );
    assert_eq!(
        after.value_source.map(|s| format!("{s:?}")),
        Some("Sniffer".to_string()),
        "and so does its provenance"
    );
}

#[test]
fn an_observed_all_clear_status_overwrites_a_stored_failure() {
    let stack = stack_with_read_status();

    publish_runtime(
        &stack,
        203,
        120,
        RuntimeSource::Poller,
        status_observation(all_clear_flags()),
    );
    assert_ok(&recv_confirm_for(&stack.conf_rx, 203));
    wait_until(
        || {
            state(&stack)
                .status
                .as_ref()
                .is_some_and(|s| !s.lamp_failure)
        },
        Duration::from_millis(500),
    );

    let after = state(&stack);
    assert_eq!(
        after.status.as_ref().map(|s| s.raw),
        Some(0x00),
        "an observed zero is an observation, and must replace the stored failure"
    );
    assert!(
        after.status.is_some(),
        "clearing the failure must not clear the fact that we looked"
    );
}

#[test]
fn an_inbound_observation_error_is_ignored() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    let mut observation = RuntimeObservation::api_timestamped(1_000);
    observation.error = Some(CompactErrorPayload::new(
        ErrorCode::ExecutionFailed,
        "execution_failed",
    ));

    publish_runtime(&stack, 204, 90, RuntimeSource::Api, observation);
    assert_ok(&recv_confirm_for(&stack.conf_rx, 204));
    wait_until(
        || state(&stack).level == Some(90),
        Duration::from_millis(500),
    );

    assert!(
        state(&stack).error.is_none(),
        "an observation cannot report an error; wiring one needs its own set/clear path"
    );
}

fn absence_entry() -> RuntimeRegistryUpdateEntry {
    RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(SHORT),
        setpoint: Some(LightSetpoint::default()),
        observation: Some(RuntimeObservation::device_absent(RuntimeSource::Poller)),
        last_dapc_source: None,
        source: RuntimeSource::Poller,
        observed_at_mono_ms: None,
    }
}

fn publish_absence(stack: &support::RegistryTestStack, corr: u64) {
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            RegistryRuntimeUpdateCommand::internal(0, absence_entry()),
        ),
    );
}

#[test]
fn an_absence_verdict_does_not_relabel_the_level_it_leaves_alone() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    publish_runtime(
        &stack,
        210,
        200,
        RuntimeSource::Api,
        RuntimeObservation::api_timestamped(1_000),
    );
    assert_ok(&recv_confirm_for(&stack.conf_rx, 210));
    wait_until(|| state(&stack).level == Some(200), Duration::from_millis(500));

    publish_absence(&stack, 211);
    assert_ok(&recv_confirm_for(&stack.conf_rx, 211));
    wait_until(
        || state(&stack).error.is_some(),
        Duration::from_millis(500),
    );

    let after = state(&stack);
    assert_eq!(after.level, Some(200), "the level is the operator's own evidence");
    assert_eq!(
        after.value_source,
        Some(dali2rust_domain::registry::ObservationSource::Api),
        "the poller read nothing; it must not be named as the source of this level"
    );
    assert_eq!(
        after.error.as_ref().map(|e| e.code),
        Some(ErrorCode::DeviceAbsent)
    );
}

#[test]
fn a_repeated_absence_verdict_says_nothing_twice() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, SHORT, DeviceType::Dt6Led, &stack.store);

    publish_absence(&stack, 220);
    assert_ok(&recv_confirm_for(&stack.conf_rx, 220));
    let first = support::recv_runtime_state_changed(&stack.ev_rx, 220);
    assert_eq!(
        first.commit_source,
        RuntimeSource::Poller,
        "the COMMIT came from the background sweep, whatever the record says about the level"
    );

    let published_after_first = stack
        .counters
        .command
        .runtime_events_published
        .load(std::sync::atomic::Ordering::Relaxed);

    publish_absence(&stack, 221);
    assert_ok(&recv_confirm_for(&stack.conf_rx, 221));

    assert_eq!(
        stack
            .counters
            .command
            .runtime_events_published
            .load(std::sync::atomic::Ordering::Relaxed),
        published_after_first,
        "the second verdict changed nothing, so nothing was announced"
    );
    assert_eq!(
        state(&stack).error.as_ref().map(|e| e.code),
        Some(ErrorCode::DeviceAbsent),
        "and the record still says the gear is silent"
    );
}
