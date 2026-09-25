mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeGroup, DeliveryStatus, PollerSettingsChangedEvent,
    PollerSettingsUpdateCommand,
};
use dali2rust_domain::registry::PollerSettingsReadPort;
use dali2rust_test_support::bus::{recv_confirmation_for, recv_event_matching};

use support::{publish_cmd, spawn_registry_stack};

const RECV_TIMEOUT: Duration = Duration::from_millis(500);

fn command(target_adapter_id: u16, payload: PollerSettingsUpdateCommand) -> dali2rust_contracts::msg::CommandEnvelope {
    dali2rust_contracts::bus::command_envelope(0, 1, target_adapter_id, None, payload)
}

fn only_interval_patch(interval_ms: u32) -> PollerSettingsUpdateCommand {
    PollerSettingsUpdateCommand {
        patch_mask: PollerSettingsUpdateCommand::PATCH_INTERVAL_MS,
        enabled: true,
        interval_ms,
        attribute_groups_mask: DaliAttributeGroup::Extended.mask_bit(),
        include_dt8_color: false,
        include_energy: false,
        include_diagnostics: false,
        skip_unbound_virtual_lamps: false,
    }
}

#[test]
fn defaults_before_any_command_match_the_documented_dto() {
    let stack = spawn_registry_stack(1, 8);
    let view = stack.store.poller_settings_view();
    assert!(!view.enabled);
    assert_eq!(view.interval_ms, 5_000);
    assert_eq!(view.attribute_groups_mask, DaliAttributeGroup::RuntimeStatus.mask_bit());
    assert!(view.include_dt8_color);
    assert!(view.skip_unbound_virtual_lamps);
}

#[test]
fn patch_by_mask_leaves_unmasked_fields_untouched() {
    let stack = spawn_registry_stack(1, 8);

    publish_cmd(&stack.publisher, command(BusId::default().0, only_interval_patch(60_000)));
    let conf = recv_confirmation_for(&stack.conf_rx, 1, RECV_TIMEOUT);
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let ev = recv_event_matching(&stack.ev_rx, RECV_TIMEOUT, |p| {
        matches!(p, BusEventPayload::PollerSettingsChangedEvent(_))
    });
    let BusEventPayload::PollerSettingsChangedEvent(changed) = &ev.payload else {
        unreachable!("filtered above")
    };
    assert_changed_applied_only_interval(changed);

    let view = stack.store.poller_settings_view();
    assert_eq!(view.interval_ms, 60_000);
    assert!(!view.enabled, "PATCH_ENABLED was not set in patch_mask");
    assert_eq!(
        stack
            .counters
            .command
            .poller_settings_applied
            .load(Ordering::Relaxed),
        1
    );
}

fn assert_changed_applied_only_interval(changed: &PollerSettingsChangedEvent) {
    assert_eq!(changed.interval_ms, 60_000, "the patched field is applied");
    assert!(!changed.enabled, "unmasked fields report the stored, pre-patch value");
    assert_eq!(changed.attribute_groups_mask, DaliAttributeGroup::RuntimeStatus.mask_bit());
    assert!(changed.include_dt8_color);
    assert!(changed.skip_unbound_virtual_lamps);
}

#[test]
fn wrong_target_adapter_is_rejected_and_settings_stay_default() {
    let stack = spawn_registry_stack(1, 8);

    publish_cmd(&stack.publisher, command(BusId::default().0 + 1, only_interval_patch(10_000)));
    let conf = recv_confirmation_for(&stack.conf_rx, 1, RECV_TIMEOUT);
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    assert_eq!(
        conf.confirmation
            .error
            .as_ref()
            .expect("product error")
            .message
            .as_str(),
        "wrong_target_adapter"
    );
    assert_eq!(stack.store.poller_settings_view().interval_ms, 5_000);
    assert_eq!(
        stack
            .counters
            .command
            .poller_settings_applied
            .load(Ordering::Relaxed),
        0
    );
}

#[test]
fn poll_targets_carry_the_effective_type_flags() {
    use dali2rust_contracts::msg::DeviceType;
    use dali2rust_domain::registry::PollTargetReadPort;

    let stack = spawn_registry_stack(1, 8);
    support::seed_physical_via_discovery(&stack.publisher, 60, 5, DeviceType::Dt8Color, &stack.store);
    support::seed_physical_via_discovery(&stack.publisher, 61, 6, DeviceType::Dt6Led, &stack.store);
    support::seed_physical_via_discovery(&stack.publisher, 62, 7, DeviceType::Unknown, &stack.store);
    publish_cmd(
        &stack.publisher,
        command(
            BusId::default().0,
            PollerSettingsUpdateCommand {
                patch_mask: PollerSettingsUpdateCommand::PATCH_SKIP_UNBOUND_VIRTUAL_LAMPS,
                enabled: false,
                interval_ms: 0,
                attribute_groups_mask: 0,
                include_dt8_color: false,
                include_energy: false,
                include_diagnostics: false,
                skip_unbound_virtual_lamps: false,
            },
        ),
    );
    let _ = recv_confirmation_for(&stack.conf_rx, 1, RECV_TIMEOUT);

    let targets = stack.store.list_poll_targets(0).targets;
    let flag = |short: u8| {
        targets
            .iter()
            .find(|t| t.short_address == short)
            .map(|t| (t.is_dt8, t.is_dt6))
            .expect("listed")
    };
    assert_eq!(flag(5), (true, false), "dt8 gear: colour top-up, no dt6 sweep");
    assert_eq!(flag(6), (false, true), "dt6 gear keeps its dt6_led group");
    assert_eq!(
        flag(7),
        (false, false),
        "unknown counts as neither — mirrored gates"
    );
}

#[test]
fn unbound_devices_are_filtered_by_the_port_when_the_setting_asks() {
    use dali2rust_contracts::msg::DeviceType;
    use dali2rust_domain::registry::PollTargetReadPort;

    let stack = spawn_registry_stack(1, 8);
    support::seed_physical_via_discovery(&stack.publisher, 70, 9, DeviceType::Dt6Led, &stack.store);
    support::seed_physical_via_discovery(&stack.publisher, 71, 10, DeviceType::Dt6Led, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            0,
            72,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::VirtualLampBindCommand {
                adapter_id: 0,
                virtual_lamp_id: 2,
                physical_short_address: 9,
            },
        ),
    );
    let _ = recv_confirmation_for(&stack.conf_rx, 72, RECV_TIMEOUT);

    let filtered = stack.store.list_poll_targets(0);
    assert_eq!(
        filtered.targets.iter().map(|t| t.short_address).collect::<Vec<_>>(),
        vec![9],
        "only the bound device is listed"
    );
    assert_eq!(filtered.excluded_unbound, 1);

    publish_cmd(
        &stack.publisher,
        command(
            BusId::default().0,
            PollerSettingsUpdateCommand {
                patch_mask: PollerSettingsUpdateCommand::PATCH_SKIP_UNBOUND_VIRTUAL_LAMPS,
                enabled: false,
                interval_ms: 0,
                attribute_groups_mask: 0,
                include_dt8_color: false,
                include_energy: false,
                include_diagnostics: false,
                skip_unbound_virtual_lamps: false,
            },
        ),
    );
    let _ = recv_confirmation_for(&stack.conf_rx, 1, RECV_TIMEOUT);
    let unfiltered = stack.store.list_poll_targets(0);
    assert_eq!(unfiltered.targets.len(), 2, "filter off: both listed");
    assert_eq!(unfiltered.excluded_unbound, 0);
}

#[test]
fn a_disabled_adapter_lists_no_poll_targets_and_says_so() {
    use dali2rust_contracts::msg::{AdapterSettingsUpdateCommand, DeviceType};
    use dali2rust_domain::registry::PollTargetReadPort;

    let stack = spawn_registry_stack(1, 8);
    support::seed_physical_via_discovery(&stack.publisher, 80, 9, DeviceType::Dt6Led, &stack.store);
    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            0,
            82,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::VirtualLampBindCommand {
                adapter_id: 0,
                virtual_lamp_id: 2,
                physical_short_address: 9,
            },
        ),
    );
    let _ = recv_confirmation_for(&stack.conf_rx, 82, RECV_TIMEOUT);
    let enabled = stack.store.list_poll_targets(0);
    assert!(enabled.adapter_enabled);
    assert_eq!(enabled.targets.iter().map(|t| t.short_address).collect::<Vec<_>>(), vec![9]);

    publish_cmd(
        &stack.publisher,
        dali2rust_contracts::bus::command_envelope(
            0,
            81,
            0,
            Some(dali2rust_contracts::msg::Origin::Api),
            AdapterSettingsUpdateCommand {
                patch_mask: AdapterSettingsUpdateCommand::PATCH_ENABLED,
                name: Default::default(),
                enabled: false,
            },
        ),
    );
    let _ = recv_confirmation_for(&stack.conf_rx, 81, RECV_TIMEOUT);

    let disabled = stack.store.list_poll_targets(0);
    assert!(!disabled.adapter_enabled);
    assert!(disabled.targets.is_empty(), "nothing on a disabled adapter is polled");
}
