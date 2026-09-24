mod support;

use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{RegistryReadPort, VirtualLampReadPort};
use dali2rust_test_support::wait_until;

use support::{
    assert_exec_failed_msg, publish_cmd, recv_confirm_for, recv_runtime_state_changed,
    seed_physical_via_discovery, spawn_registry_stack,
};
use dali2rust_bus::BusId;
use dali2rust_contracts::msg::DeviceType;

#[test]
fn registry_runtime_vl_only_rejected_when_unbound() {
    let stack = spawn_registry_stack(1, 16);
    let corr = 7u64;
    publish_cmd(
        &stack.publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::sniffer_level(12, 180, 12_345))) },
    );

    assert_eq!(stack.store.virtual_lamp_snapshot(0, 12).runtime_level, 0);
    assert_exec_failed_msg(&recv_confirm_for(&stack.conf_rx, corr), "vl_unbound");
    assert!(
        stack.ev_rx.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
        "unbound vl-only update must not publish RuntimeStateChangedEvent"
    );
}

#[test]
fn runtime_state_changed_event_reflects_post_commit_physical_when_vl_bound() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, 7, DeviceType::Dt6Led, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 3, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 12, physical_short_address: 7 }),
    );
    wait_until(
        || stack.store.virtual_lamp_view(0, 12).binding_short == Some(7),
        std::time::Duration::from_millis(500),
    );

    let corr = 8u64;
    publish_cmd(
        &stack.publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::sniffer_level(12, 55, 99_001))) },
    );
    wait_until(
        || {
            stack
                .store
                .physical_device_view(0, 7)
                .is_some_and(|pd| pd.state.level == Some(55))
        },
        std::time::Duration::from_millis(500),
    );

    assert_eq!(
        stack
            .store
            .physical_device_view(0, 7)
            .expect("pd")
            .state
            .level,
        Some(55)
    );
    let body = recv_runtime_state_changed(&stack.ev_rx, corr);
    assert_eq!(body.virtual_lamp_id, Some(12u8));
    assert_eq!(body.short_address, Some(7));
    assert_eq!(body.state_setpoint.level, 55);
    assert_eq!(body.state_observation.last_seen_ms, Some(99_001));
}

#[test]
fn a_short_only_commit_names_the_lamp_bound_to_that_short_address() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, 7, DeviceType::Dt6Led, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            3,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::VirtualLampBindCommand {
                adapter_id: 0,
                virtual_lamp_id: 12,
                physical_short_address: 7,
            },
        ),
    );
    wait_until(
        || stack.store.virtual_lamp_view(0, 12).binding_short == Some(7),
        std::time::Duration::from_millis(500),
    );

    let corr = 21u64;
    let setpoint = dali2rust_contracts::msg::LightSetpoint {
        power: dali2rust_contracts::msg::PowerState::On,
        level: 77,
        color: None,
    };
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(
                    7, &setpoint, 12_345,
                ),
            ),
        ),
    );

    let event = recv_runtime_state_changed(&stack.ev_rx, corr);
    assert_eq!(event.short_address, Some(7));
    assert_eq!(
        event.virtual_lamp_id,
        Some(12),
        "the commit landed on lamp 12; an event without it hides that from every consumer"
    );
    assert_eq!(event.state_setpoint.level, 77);
}

#[test]
fn a_short_only_commit_on_an_unbound_device_names_no_lamp() {
    let stack = spawn_registry_stack(1, 16);
    seed_physical_via_discovery(&stack.publisher, 2, 9, DeviceType::Dt6Led, &stack.store);

    let corr = 22u64;
    let setpoint = dali2rust_contracts::msg::LightSetpoint {
        power: dali2rust_contracts::msg::PowerState::On,
        level: 41,
        color: None,
    };
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(
                    9, &setpoint, 12_346,
                ),
            ),
        ),
    );

    let event = recv_runtime_state_changed(&stack.ev_rx, corr);
    assert_eq!(event.short_address, Some(9));
    assert_eq!(event.virtual_lamp_id, None);
}
