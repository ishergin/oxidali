use dali2rust_contracts::msg::AdapterSettingsUpdateCommand;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusId, PublishResult};
use dali2rust_contracts::bus::{



    runtime_observation_api_timestamped,
};
use dali2rust_contracts::msg::{
    BusEventPayload, ColorMode, ColorValue, DeliveryStatus, DeviceType, ErrorCode, LastDapcSource,
    LightSetpoint, PowerState, RuntimeRegistryUpdateEntry, RuntimeSource,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{VirtualLampReadPort};
use dali2rust_registry_runtime::{RegistryStore, RegistryWorkerCounters};
use dali2rust_test_support::wait_until;

mod support;
use support::{publish_cmd, recv_confirm_for};

use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as PdPatch;

fn spawn_cmd_stack(
    adapter_count: u8,
) -> (
    dali2rust_bus::BusPublisher,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<RegistryStore>,
    Arc<RegistryWorkerCounters>,
    dali2rust_bus::BusHost,
) {
    let (publisher, conf_rx, _ev_rx, store, counters, host) =
        spawn_cmd_stack_with_events(adapter_count);
    (publisher, conf_rx, store, counters, host)
}

fn spawn_cmd_stack_with_events(
    adapter_count: u8,
) -> (
    dali2rust_bus::BusPublisher,
    std::sync::mpsc::Receiver<BusFrame>,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<RegistryStore>,
    Arc<RegistryWorkerCounters>,
    dali2rust_bus::BusHost,
) {
    let s = support::spawn_registry_stack(adapter_count, 32);
    (s.publisher, s.conf_rx, s.ev_rx, s.store, s.counters, s._host)
}

fn assert_exec_failed_msg(c: &dali2rust_contracts::msg::ConfirmationEnvelope, needle: &str) {
    assert_eq!(c.status, DeliveryStatus::ExecutionFailed);
    let err = c
        .confirmation
        .error
        .as_ref()
        .expect("product error");
    assert!(
        err.message.as_str().contains(needle),
        "got {:?}",
        err.message.as_str()
    );
}

fn assert_ok(c: &dali2rust_contracts::msg::ConfirmationEnvelope) {
    assert_eq!(c.status, DeliveryStatus::Ok);
    assert!(c.confirmation.error.is_none());
}

fn recv_event_for(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
    correlation_id: u64,
) -> std::sync::Arc<dali2rust_contracts::msg::EventEnvelope> {
    for _ in 0..40 {
        let frame = rx.recv_timeout(Duration::from_millis(100)).expect("event");
        if let BusFrame::Event(ev) = frame {
            if ev.meta.correlation_id == correlation_id {
                return ev;
            }
        }
    }
    panic!("no event for correlation {correlation_id}");
}

fn seed_physical(
    publisher: &dali2rust_bus::BusPublisher,
    store: &RegistryStore,
    short: u8,
) {
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, u64::from(short), BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliDiscoveryProgressEvent { registry_adapter_id: 0, short_address: short, random_address: None, device_type: DeviceType::Dt6Led, color_mode: ColorMode::Unknown, dt8_xy_capable: false, dt8_tc_capable: false, dt8_rgb_capable: false, dt8_rgbwaf_capable: false, supported_device_types: None });
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
    wait_until(
        || store.physical_device_view(0, short).is_some(),
        Duration::from_millis(500),
    );
    assert!(store.physical_device_view(0, short).is_some());
}

#[test]
fn virtual_lamp_bind_wrong_target_adapter() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    let corr = 501u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, 2, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 3, physical_short_address: 10 }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "wrong_target_adapter");
    assert_eq!(
        c.confirmation.error.as_ref().unwrap().code,
        ErrorCode::InvalidResourceId
    );
}

#[test]
fn virtual_lamp_bind_adapter_id_out_of_range() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    let corr = 502u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 5, virtual_lamp_id: 3, physical_short_address: 10 }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "adapter_id_out_of_range");
}

#[test]
fn virtual_lamp_bind_rejected_missing_physical() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    let _ = store;
    let corr = 503u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 1, physical_short_address: 20 }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "virtual_lamp_bind_rejected");
    assert_eq!(c.confirmation.error.as_ref().unwrap().code, ErrorCode::Conflict);
}

#[test]
fn virtual_lamp_rebind_conflict_returns_virtual_lamp_rebind_rejected() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 580, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 7, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 580));

    let corr = 581u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampRebindCommand { adapter_id: 0, virtual_lamp_id: 7, physical_short_address: 44 }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "virtual_lamp_rebind_rejected");
    assert_eq!(c.confirmation.error.as_ref().unwrap().code, ErrorCode::Conflict);
}

#[test]
fn a_second_lamp_cannot_take_a_short_address_another_lamp_holds() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 620, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 3, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 620));

    let corr = 621u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 5, physical_short_address: 10 }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "virtual_lamp_bind_rejected");
    assert_eq!(c.confirmation.error.as_ref().unwrap().code, ErrorCode::Conflict);
    assert_eq!(store.virtual_lamp_view(0, 5).binding_short, None);
    assert_eq!(
        store.virtual_lamp_view(0, 3).binding_short,
        Some(10),
        "the lamp that already holds the address keeps it"
    );
}

#[test]
fn an_address_is_free_again_once_the_lamp_holding_it_moves_away() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    seed_physical(&publisher, &store, 11);
    for (corr, lamp, short) in [(630u64, 3u8, 10u8), (631, 3, 10), (632, 3, 11), (633, 6, 10)] {
        publish_cmd(
            &publisher,
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: lamp, physical_short_address: short }),
        );
        assert_ok(&recv_confirm_for(&conf_rx, corr));
    }
    assert_eq!(store.virtual_lamp_view(0, 3).binding_short, Some(11));
    assert_eq!(store.virtual_lamp_view(0, 6).binding_short, Some(10));
}

#[test]
fn virtual_lamp_bind_rebind_unbind_happy_path() {
    let (publisher, conf_rx, store, counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    seed_physical(&publisher, &store, 11);

    let c1 = 601u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c1, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 4, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c1));
    assert_eq!(store.virtual_lamp_view(0, 4).binding_short, Some(10));

    let c2 = 602u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c2, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampRebindCommand { adapter_id: 0, virtual_lamp_id: 4, physical_short_address: 11 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c2));
    assert_eq!(store.virtual_lamp_view(0, 4).binding_short, Some(11));

    let c3 = 603u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c3, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampUnbindCommand { adapter_id: 0, virtual_lamp_id: 4 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c3));
    assert_eq!(store.virtual_lamp_view(0, 4).binding_short, None);
    assert!(counters.command.virtual_lamp_bindings_applied.load(Ordering::Relaxed) >= 2);
}

#[test]
fn virtual_lamp_bind_success_emits_virtual_lamp_changed_event() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 15);
    let corr = 604u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 9, physical_short_address: 15 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    assert!(matches!(ev.payload, BusEventPayload::VirtualLampChangedEvent(_)));
}

#[test]
fn physical_device_override_success_and_missing_record() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 12);
    let ok_corr = 701u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, ok_corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceOverrideCommand { adapter_id: 0, short_address: 12, patch_mask: PdPatch::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64("Bench"), clear_device_type_override: false, device_type_override: DeviceType::Unknown, clear_color_mode_override: false, color_mode_override: ColorMode::Unknown , dt8_auto_activation_repair: true, dt8_rgbwaf_control_assert: true}),
    );
    assert_ok(&recv_confirm_for(&conf_rx, ok_corr));
    assert_eq!(
        store.physical_device_view(0, 12).expect("pd").name,
        "Bench"
    );

    let bad_corr = 702u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, bad_corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceOverrideCommand { adapter_id: 0, short_address: 63, patch_mask: PdPatch::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64("X"), clear_device_type_override: false, device_type_override: DeviceType::Unknown, clear_color_mode_override: false, color_mode_override: ColorMode::Unknown , dt8_auto_activation_repair: true, dt8_rgbwaf_control_assert: true}),
    );
    let c = recv_confirm_for(&conf_rx, bad_corr);
    assert_exec_failed_msg(&c, "physical_device_override_not_applied");
    assert_eq!(c.confirmation.error.as_ref().unwrap().code, ErrorCode::InvalidValue);
}

#[test]
fn physical_device_override_success_emits_physical_device_changed_event() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 13);
    let corr = 703u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceOverrideCommand { adapter_id: 0, short_address: 13, patch_mask: PdPatch::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64("Desk"), clear_device_type_override: false, device_type_override: DeviceType::Unknown, clear_color_mode_override: false, color_mode_override: ColorMode::Unknown , dt8_auto_activation_repair: true, dt8_rgbwaf_control_assert: true}),
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    assert!(matches!(
        ev.payload,
        BusEventPayload::PhysicalDeviceChangedEvent(_)
    ));
}

#[test]
fn registry_runtime_update_not_applied_for_unknown_physical_short() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    let _ = store;
    let sp = LightSetpoint {
        power: PowerState::On,
        level: 40,
        color: None,
    };
    let corr = 801u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(99, &sp, 9_000))) },
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "registry_runtime_not_applied");
}

#[test]
fn registry_runtime_update_applies_physical_short() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 14);
    let sp = LightSetpoint {
        power: PowerState::On,
        level: 88,
        color: None,
    };
    let corr = 802u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(14, &sp, 11_000))) },
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let v = store.physical_device_view(0, 14).expect("pd");
    assert_eq!(v.state.level, Some(88));
    assert_eq!(v.state.last_seen_ms, Some(11_000));
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert_eq!(body.short_address, Some(14));
    assert_eq!(body.virtual_lamp_id, None);
    assert_eq!(body.state_setpoint.level, 88);
}

#[test]
fn a_level_commit_on_a_coloured_gear_publishes_the_commits_dimensions_not_the_records() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 21);

    let colour = LightSetpoint {
        power: PowerState::Unknown,
        level: 0,
        color: Some(dali2rust_contracts::msg::ColorValue {
            mode: dali2rust_contracts::msg::ColorMode::Cct,
            color_temperature_kelvin: 2700,
            ..Default::default()
        }),
    };
    let corr = 861u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(
                    21, &colour, 12_000,
                ),
            ),
        ),
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert_eq!(
        body.commit_dimensions,
        dali2rust_contracts::msg::SetpointDimensions {
            level: false,
            color: true
        },
        "a colour-only commit states the colour and nothing else"
    );

    let level = LightSetpoint::from_level(200, None);
    let corr = 862u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(
                0,
                dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_short_physical(
                    21, &level, 13_000,
                ),
            ),
        ),
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert!(
        body.state_setpoint.color.is_some(),
        "the premise: the record still holds the colour, so reading the snapshot \
         would answer 'this commit stated a colour'"
    );
    assert_eq!(
        body.commit_dimensions,
        dali2rust_contracts::msg::SetpointDimensions {
            level: true,
            color: false
        },
        "the commit stated a level and nothing else"
    );
}

#[test]
fn registry_runtime_colour_only_update_keeps_stored_level() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 14);

    let level_sp = LightSetpoint {
        power: PowerState::On,
        level: 120,
        color: None,
    };
    let c1 = 811u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c1, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(14, &level_sp, 11_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c1));

    let colour_only = LightSetpoint {
        power: PowerState::On,
        level: 0,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 3000,
            ..ColorValue::default()
        }),
    };
    let c2 = 812u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c2, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(14, &colour_only, 12_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c2));
    let v = store.physical_device_view(0, 14).expect("pd");
    assert_eq!(v.state.level, Some(120), "colour-only must keep the level");
    assert_eq!(v.state.last_seen_ms, Some(12_000), "metadata still applies");

    let off_sp = LightSetpoint {
        power: PowerState::Off,
        level: 0,
        color: None,
    };
    let c3 = 813u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c3, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(14, &off_sp, 13_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c3));
    let v = store.physical_device_view(0, 14).expect("pd");
    assert_eq!(v.state.level, Some(0), "off writes the honest zero");
}

#[test]
fn a_switch_on_with_colour_on_a_dark_lamp_recalls_the_shadowed_level() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 15);

    for (corr, setpoint) in [
        (821u64, LightSetpoint { power: PowerState::On, level: 120, color: None }),
        (822u64, LightSetpoint { power: PowerState::Off, level: 0, color: None }),
    ] {
        publish_cmd(
            &publisher,
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(15, &setpoint, 20_000 + corr))),
        );
        assert_ok(&recv_confirm_for(&conf_rx, corr));
    }
    assert_eq!(
        store.physical_device_view(0, 15).expect("pd").state.level,
        Some(0),
        "the off is stored as the honest zero"
    );

    let switch_on = LightSetpoint {
        power: PowerState::On,
        level: 0,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 3000,
            ..ColorValue::default()
        }),
    };
    let c = 823u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(15, &switch_on, 23_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c));
    let v = store.physical_device_view(0, 15).expect("pd");
    assert_eq!(
        v.state.level,
        Some(120),
        "the recall lands on the level the lamp was last lit at, not on the off's zero"
    );
    assert_eq!(v.state.power, "on", "and the lamp is on");
    assert_eq!(
        v.state.color_temperature_kelvin,
        Some(3000),
        "the colour it carried still applies"
    );
}

#[test]
fn a_recall_on_a_lamp_never_seen_lit_leaves_the_level_unknown() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 17);

    let switch_on = LightSetpoint { power: PowerState::On, level: 0, color: None };
    let c = 841u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(17, &switch_on, 40_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c));
    let v = store.physical_device_view(0, 17).expect("pd");
    assert_eq!(v.state.level, None, "no shadow, so no prediction");
    assert_eq!(v.state.power, "on", "the lamp is still on");
}

#[test]
fn a_colour_only_setpoint_states_neither_power_nor_level() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 16);

    let lit = LightSetpoint { power: PowerState::On, level: 90, color: None };
    let c1 = 831u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c1, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(16, &lit, 30_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c1));

    let colour_only = LightSetpoint {
        power: PowerState::Unknown,
        level: 0,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 4000,
            ..ColorValue::default()
        }),
    };
    let c2 = 832u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c2, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(16, &colour_only, 31_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c2));
    let v = store.physical_device_view(0, 16).expect("pd");
    assert_eq!(v.state.level, Some(90), "a colour states no level");
    assert_eq!(v.state.power, "on", "and no power either");
    assert_eq!(v.state.color_temperature_kelvin, Some(4000));
}

#[test]
fn registry_runtime_vl_only_rejected_when_unbound() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    let corr = 803u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::sniffer_level(12, 180, 12_345))) },
    );
    assert_exec_failed_msg(&recv_confirm_for(&conf_rx, corr), "vl_unbound");
    let dto = store.virtual_lamp_view(0, 12);
    assert_eq!(dto.state.level, None);
    assert_eq!(dto.state.last_seen_ms, None);
    assert!(
        ev_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "unbound vl-only update must not emit RuntimeStateChangedEvent"
    );
}

#[test]
fn registry_runtime_missing_setpoint_and_missing_observation() {
    let (publisher, conf_rx, _store, _counters, _host) = spawn_cmd_stack(1);
    let mut entry = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: Some(1),
        short_address: None,
        setpoint: None,
        observation: Some(runtime_observation_api_timestamped(1)),
        last_dapc_source: None,
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    let c1 = 901u64;
    publish_cmd(
        &publisher,
        { let __corr = c1; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, entry.clone())) },
    );
    let r1 = recv_confirm_for(&conf_rx, c1);
    assert_exec_failed_msg(&r1, "registry_runtime_missing_setpoint");

    entry.setpoint = Some(LightSetpoint {
        power: PowerState::On,
        level: 1,
        color: None,
    });
    entry.observation = None;
    let c2 = 902u64;
    publish_cmd(
        &publisher,
        { let __corr = c2; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, entry)) },
    );
    let r2 = recv_confirm_for(&conf_rx, c2);
    assert_exec_failed_msg(&r2, "registry_runtime_missing_observation");
}

#[test]
fn adapter_settings_update_success() {
    let (publisher, conf_rx, store, counters, _host) = spawn_cmd_stack(1);
    let _ = store;
    let corr = 1001u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, 0, None, dali2rust_contracts::msg::AdapterSettingsUpdateCommand { patch_mask: AdapterSettingsUpdateCommand::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64((Some("Adapter-A")).unwrap_or("")), enabled: false }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    assert_eq!(counters.command.adapter_settings_applied.load(Ordering::Relaxed), 1);
}

#[test]
fn adapter_settings_out_of_range_returns_adapter_id_out_of_range() {
    let (publisher, conf_rx, _store, _counters, _host) = spawn_cmd_stack(1);
    let corr = 1002u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, 1, None, dali2rust_contracts::msg::AdapterSettingsUpdateCommand { patch_mask: AdapterSettingsUpdateCommand::PATCH_NAME, name: dali2rust_contracts::msg::fixed_text_64((Some("Adapter-B")).unwrap_or("")), enabled: true }),
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "adapter_id_out_of_range");
    assert_eq!(
        c.confirmation.error.as_ref().expect("product error").code,
        ErrorCode::InvalidResourceId
    );
}

#[test]
fn non_registry_command_is_ignored_without_confirmation() {
    let (publisher, conf_rx, store, counters, _host) = spawn_cmd_stack(1);
    let _ = store;
    let before = counters.command.ignored_commands.load(Ordering::Relaxed);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 4242, BusId::default().0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 1, command: 254, repeat_count: 0, raw_mode: false, raw_expects_backward: false }),
    );
    wait_until(
        || counters.command.ignored_commands.load(Ordering::Relaxed) == before + 1,
        Duration::from_millis(500),
    );
    assert_eq!(counters.command.ignored_commands.load(Ordering::Relaxed), before + 1);
    assert!(conf_rx.recv_timeout(Duration::from_millis(80)).is_err());
}

#[test]
fn registry_runtime_combined_vl_and_short_single_commit_and_event() {
    let (publisher, conf_rx, ev_rx, store, counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 10);
    let bind_corr = 2100u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, bind_corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 9, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, bind_corr));

    let sp = LightSetpoint {
        power: PowerState::On,
        level: 66,
        color: None,
    };
    let corr = 2101u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, dali2rust_contracts::msg::RuntimeRegistryUpdateEntry::api_virtual_lamp_and_short(9, 10, &sp, 5000))) },
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    assert_eq!(
        store.physical_device_view(0, 10).expect("pd").state.level,
        Some(66)
    );
    assert_eq!(
        counters.command.physical_device_runtime_applied.load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        counters.command.virtual_lamp_runtime_applied.load(Ordering::Relaxed),
        1
    );
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert_eq!(body.virtual_lamp_id, Some(9));
    assert_eq!(body.short_address, Some(10));
    assert_eq!(body.state_setpoint.level, 66);
}

#[test]
fn registry_runtime_binding_mismatch_fails() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    seed_physical(&publisher, &store, 11);
    let bind_corr = 2200u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, bind_corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 8, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, bind_corr));

    let sp = LightSetpoint {
        power: PowerState::On,
        level: 1,
        color: None,
    };
    let mut obs = runtime_observation_api_timestamped(100);
    obs.last_dapc_source = LastDapcSource::Sniffer;
    let entry = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: Some(8),
        short_address: Some(11),
        setpoint: Some(sp),
        observation: Some(obs),
        last_dapc_source: None,
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    let corr = 2201u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, entry)) },
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "registry_runtime_binding_mismatch");
    assert_eq!(
        c.confirmation.error.as_ref().expect("err").code,
        ErrorCode::Conflict
    );
}

#[test]
fn registry_runtime_entry_last_dapc_source_overrides_observation() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 14);
    let sp = LightSetpoint {
        power: PowerState::On,
        level: 10,
        color: None,
    };
    let mut obs = runtime_observation_api_timestamped(11_000);
    obs.last_dapc_source = LastDapcSource::Sniffer;
    let entry = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(14),
        setpoint: Some(sp),
        observation: Some(obs),
        last_dapc_source: Some(LastDapcSource::Scene),
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    let corr = 2301u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, entry)) },
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert_eq!(body.state_observation.last_dapc_source, LastDapcSource::Scene);
}

#[test]
fn registry_runtime_entry_last_dapc_source_none_preserves_existing_value() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 15);

    let initial = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(15),
        setpoint: Some(LightSetpoint {
            power: PowerState::On,
            level: 20,
            color: None,
        }),
        observation: Some(runtime_observation_api_timestamped(12_000)),
        last_dapc_source: Some(LastDapcSource::Scene),
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    publish_cmd(
        &publisher,
        { let __corr = 2350; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, initial)) },
    );
    assert_ok(&recv_confirm_for(&conf_rx, 2350));
    let _ = recv_event_for(&ev_rx, 2350);

    let mut obs = runtime_observation_api_timestamped(12_500);
    obs.last_dapc_source = LastDapcSource::Sniffer;
    let follow_up = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: Some(15),
        setpoint: Some(LightSetpoint {
            power: PowerState::On,
            level: 21,
            color: None,
        }),
        observation: Some(obs),
        last_dapc_source: None,
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    let corr = 2351u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, follow_up)) },
    );
    assert_ok(&recv_confirm_for(&conf_rx, corr));
    let ev = recv_event_for(&ev_rx, corr);
    let BusEventPayload::RuntimeStateChangedEvent(body) = &ev.payload else {
        panic!("expected runtime event");
    };
    assert_eq!(body.state_observation.last_dapc_source, LastDapcSource::Scene);
}

#[test]
fn registry_runtime_missing_virtual_lamp_and_short_fails() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 20);
    let entry = RuntimeRegistryUpdateEntry {
        virtual_lamp_id: None,
        short_address: None,
        setpoint: Some(LightSetpoint {
            power: PowerState::On,
            level: 1,
            color: None,
        }),
        observation: Some(runtime_observation_api_timestamped(1)),
        last_dapc_source: None,
        source: RuntimeSource::Api,
        observed_at_mono_ms: None,
    };
    let corr = 2401u64;
    publish_cmd(
        &publisher,
        { let __corr = corr; dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, __corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, entry)) },
    );
    let c = recv_confirm_for(&conf_rx, corr);
    assert_exec_failed_msg(&c, "registry_runtime_missing_target");
}

#[test]
fn forgetting_a_device_unbinds_the_lamp_and_leaves_the_lamp() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 10);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 700, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::VirtualLampBindCommand { adapter_id: 0, virtual_lamp_id: 4, physical_short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 700));

    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 701, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceDeleteCommand { adapter_id: 0, short_address: 10 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 701));

    assert!(
        store.physical_device_view(0, 10).is_none(),
        "the device record must be gone"
    );
    let lamp = store.virtual_lamp_view(0, 4);
    assert_eq!(
        lamp.binding_short, None,
        "the lamp survives — it is a product object the operator named — but its \
         binding must not survive the device"
    );
}

#[test]
fn forgetting_a_device_twice_reports_not_found() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 11);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 710, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceDeleteCommand { adapter_id: 0, short_address: 11 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 710));

    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 711, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceDeleteCommand { adapter_id: 0, short_address: 11 }),
    );
    let c = recv_confirm_for(&conf_rx, 711);
    assert_exec_failed_msg(&c, "physical_device_not_found");
    assert_eq!(c.confirmation.error.as_ref().unwrap().code, ErrorCode::NotFound);
}

#[test]
fn a_forgotten_device_still_on_the_wire_returns_blank_on_the_next_scan() {
    let (publisher, conf_rx, store, _counters, _host) = spawn_cmd_stack(1);
    seed_physical(&publisher, &store, 12);
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 720, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceNotesUpdateCommand { adapter_id: 0, short_address: 12, notes: dali2rust_contracts::msg::fixed_text_48("third from the window") }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 720));

    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 721, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::PhysicalDeviceDeleteCommand { adapter_id: 0, short_address: 12 }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 721));

    seed_physical(&publisher, &store, 12);
    let view = store
        .physical_device_view(0, 12)
        .expect("a scan re-creates the record");
    assert!(
        view.notes.as_deref().unwrap_or("").is_empty(),
        "the record comes back blank: notes, name and overrides do not survive"
    );
}

#[test]
fn a_foreign_recall_verb_is_resolved_against_the_shadowed_level() {
    use dali2rust_contracts::msg::{LevelTransition, RegistryLevelTransitionCommand};

    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 21);

    for (corr, setpoint) in [
        (901u64, LightSetpoint { power: PowerState::On, level: 137, color: None }),
        (902u64, LightSetpoint { power: PowerState::Off, level: 0, color: None }),
    ] {
        publish_cmd(
            &publisher,
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, corr, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(21, &setpoint, 30_000 + corr))),
        );
        assert_ok(&recv_confirm_for(&conf_rx, corr));
    }
    assert_eq!(
        store.physical_device_view(0, 21).expect("pd").state.level,
        Some(0)
    );

    let c = 903u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), RegistryLevelTransitionCommand {
            adapter_id: 0,
            virtual_lamp_id: None,
            short_address: Some(21),
            transition: LevelTransition::GoToLastActiveLevel,
            observation: Some(dali2rust_contracts::msg::RuntimeObservation::sniffer_timestamped(33_000)),
            source: dali2rust_contracts::msg::RuntimeSource::Sniffer,
            observed_at_mono_ms: Some(33_000),
        }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c));
    assert_eq!(
        store.physical_device_view(0, 21).expect("pd").state.level,
        Some(137),
        "the verb resolves against the shadow the OFF left standing"
    );
}

#[test]
fn an_unresolvable_verb_leaves_the_stored_level_untouched() {
    use dali2rust_contracts::msg::{LevelTransition, RegistryLevelTransitionCommand};

    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_cmd_stack_with_events(1);
    seed_physical(&publisher, &store, 22);
    let lit = LightSetpoint { power: PowerState::On, level: 90, color: None };
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 911, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::RegistryRuntimeUpdateCommand::internal(0, RuntimeRegistryUpdateEntry::api_short_physical(22, &lit, 40_000))),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 911));

    let c = 912u64;
    publish_cmd(
        &publisher,
        dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, c, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), RegistryLevelTransitionCommand {
            adapter_id: 0,
            virtual_lamp_id: None,
            short_address: Some(22),
            transition: LevelTransition::StepUp,
            observation: Some(dali2rust_contracts::msg::RuntimeObservation::sniffer_timestamped(41_000)),
            source: dali2rust_contracts::msg::RuntimeSource::Sniffer,
            observed_at_mono_ms: Some(41_000),
        }),
    );
    assert_ok(&recv_confirm_for(&conf_rx, c));
    assert_eq!(
        store.physical_device_view(0, 22).expect("pd").state.level,
        Some(90),
        "no bounds means no step this controller can name"
    );
}
