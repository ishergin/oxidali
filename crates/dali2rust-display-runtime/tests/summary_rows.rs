mod support;

use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusPublisher, PublishResult};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    BusHealthVerdict, ColorMode, ColorValue, DaliBusHealthProbedEvent, DaliDiscoveryProgressEvent,
    DaliInputEventObservedEvent, DaliObservedFrameEvent, DaliSceneRecalledEvent, DaliTargetScope,
    DaliTargetStateAppliedEvent, DeviceType, ObservedKind,
    InputEventKind, IpAddressAssignedEvent, LightSetpoint, Origin, PowerState, RuntimeSource,
};
use dali2rust_display_runtime::{DisplaySample, DisplayView};
use dali2rust_test_support::wait_until;

use support::{healthy_sample, spawn_display_worker_on_bus};

fn publish<P: Into<dali2rust_contracts::msg::BusEventPayload>>(publisher: &BusPublisher, payload: P) {
    let ev = event_envelope(0, 1, 0, Some(Origin::Internal), payload);
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
}

fn wait_row(view: &DisplayView, row: usize, needle: &str) {
    wait_until(
        || view.lines()[row].text.contains(needle),
        Duration::from_secs(3),
    );
}

#[test]
fn the_address_row_follows_the_assigned_ip() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, IpAddressAssignedEvent::from_ip_text("10.0.0.9"));
    wait_row(&view, 0, "10.0.0.9");
}

#[test]
fn a_single_controller_keeps_its_uptime_and_shows_no_role() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, IpAddressAssignedEvent::from_ip_text("10.0.0.9"));
    wait_row(&view, 0, "10.0.0.9");
    let row = view.lines()[0].text.clone();
    assert!(row.contains("00H00M"), "the uptime is the whole right side: {row:?}");
}

#[test]
fn the_active_unit_of_a_pair_is_named_on_the_address_row() {
    let sample = DisplaySample {
        redundancy_enabled: true,
        controller_active: true,
        ..healthy_sample()
    };
    let (_host, publisher, view, _w) = spawn_display_worker_on_bus(BusConfig::default(), sample);
    publish(&publisher, IpAddressAssignedEvent::from_ip_text("10.0.0.9"));
    wait_row(&view, 0, "A00H00M");
    assert!(view.lines()[0].text.contains("10.0.0.9"), "the address stays");
}

#[test]
fn a_standby_says_so_and_takes_the_inverted_treatment() {
    let sample = DisplaySample {
        redundancy_enabled: true,
        controller_active: false,
        ..healthy_sample()
    };
    let (_host, publisher, view, _w) = spawn_display_worker_on_bus(BusConfig::default(), sample);
    publish(&publisher, IpAddressAssignedEvent::from_ip_text("10.0.0.9"));
    wait_row(&view, 0, "S00H00M");
    assert!(
        view.lines()[0].invert,
        "a standby's address row is inverted"
    );
}

#[test]
fn the_bus_row_takes_the_health_verdict_and_inverts_on_failure() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    wait_row(&view, 2, "BUS  ?");
    publish(
        &publisher,
        DaliBusHealthProbedEvent {
            registry_adapter_id: 0,
            control_answered: true,
            lamp_failure: BusHealthVerdict::One,
        },
    );
    wait_row(&view, 2, "LAMP FAIL");
    assert!(view.lines()[2].invert, "a failure takes the alarm treatment");
}

#[test]
fn a_button_press_reaches_the_input_row() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(
        &publisher,
        DaliInputEventObservedEvent {
            registry_adapter_id: 0,
            scheme: 2,
            short_address: Some(5),
            device_group: None,
            instance_group: None,
            instance_number: Some(2),
            instance_type: Some(1),
            event_info: 0x002,
            typed: InputEventKind::Button,
            typed_value: 0x002,
            observed_at_ms: 0,
            observed_at_mono_ms: 0,
        },
    );
    wait_row(&view, 5, "A05.2 SHORT");
}

#[test]
fn the_poller_row_reads_the_settings_without_any_event() {
    let sample = DisplaySample {
        poller_enabled: true,
        poller_interval_ms: 5_000,
        ..healthy_sample()
    };
    let (_host, _publisher, view, _w) = spawn_display_worker_on_bus(BusConfig::default(), sample);
    wait_row(&view, 4, "POLL 5S OK");
}

#[test]
fn an_applied_change_lands_on_the_live_row_with_its_source() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, applied(RuntimeSource::Mqtt, 180, None));
    wait_row(&view, 6, "A03 ON 180 HA");
}

#[test]
fn an_hcl_apply_fills_the_hcl_row() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, applied(RuntimeSource::Hcl, 200, Some(3200)));
    wait_row(&view, 3, "3200K");
}

#[test]
fn a_discovery_walk_takes_the_live_row_with_a_count() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(
        &publisher,
        DaliDiscoveryProgressEvent {
            registry_adapter_id: 0,
            short_address: 36,
            random_address: None,
            device_type: DeviceType::Dt6Led,
            color_mode: ColorMode::Unknown,
            dt8_xy_capable: false,
            dt8_tc_capable: false,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            supported_device_types: None,
        },
    );
    wait_row(&view, 6, "SCAN 1 FOUND");
    assert!(view.lines()[6].bar.is_none(), "an address is not progress; no bar over 64");
}

#[test]
fn switching_off_says_off_rather_than_a_level() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    let mut ev = applied(RuntimeSource::Api, 0, None);
    ev.setpoint.power = PowerState::Off;
    publish(&publisher, ev);
    wait_row(&view, 6, "A03 OFF");
}

#[test]
fn a_group_apply_is_named_as_a_group() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    let mut ev = applied(RuntimeSource::Api, 200, None);
    ev.scope = DaliTargetScope::Group;
    ev.short_address = Some(0);
    ev.group_id = Some(2);
    publish(&publisher, ev);
    wait_row(&view, 6, "G02 ON 200");
}

#[test]
fn a_scene_recall_names_the_scene() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(
        &publisher,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Group,
            short_address: 0,
            group_id: 2,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: 0,
        },
    );
    wait_row(&view, 6, "G02 SC3");
}

#[test]
fn a_foreign_group_command_is_shown_as_a_group() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, observed_group_level(200));
    wait_row(&view, 6, "G02 ON 200 BUS");
}

#[test]
fn a_foreign_scene_recall_names_the_scene() {
    let mut ev = observed_group_level(0);
    ev.observed_kind = ObservedKind::SceneRecallObserved;
    ev.scene_id = Some(4);
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    publish(&publisher, ev);
    wait_row(&view, 6, "G02 SC4 BUS");
}

fn observed_group_level(level: u8) -> DaliObservedFrameEvent {
    DaliObservedFrameEvent {
        registry_adapter_id: 0,
        observed_kind: ObservedKind::TargetStateObserved,
        scope: DaliTargetScope::Group,
        short_address: None,
        group_id: Some(2),
        scene_id: None,
        setpoint: Some(LightSetpoint {
            power: PowerState::On,
            level,
            color: None,
        }),
        dapc_observed: true,
        level_transition: None,
        raw_frame: [0x85, level, 0],
        raw_width: dali2rust_contracts::msg::ObservedFrameWidth::Forward16,
        decode_status: dali2rust_contracts::msg::DecodeStatus::Decoded,
        observed_at_ms: 0,
        observed_at_mono_ms: 0,
    }
}

fn applied(source: RuntimeSource, level: u8, kelvin: Option<u16>) -> DaliTargetStateAppliedEvent {
    DaliTargetStateAppliedEvent {
        registry_adapter_id: 0,
        scope: DaliTargetScope::Short,
        virtual_lamp_id: None,
        short_address: Some(3),
        group_id: None,
        setpoint: LightSetpoint {
            power: PowerState::On,
            level,
            color: kelvin.map(|k| ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: k,
                x: 0,
                y: 0,
                r: 0,
                g: 0,
                b: 0,
                w: 0,
                a: 0,
                f: 0,
            }),
        },
        dapc_applied: true,
        source,
        applied_at_mono_ms: 0,
    }
}

#[test]
fn an_event_for_another_adapter_does_not_reach_the_rows() {
    let (_host, publisher, view, _w) =
        spawn_display_worker_on_bus(BusConfig::default(), healthy_sample());
    let mut foreign = applied(RuntimeSource::Api, 180, None);
    foreign.registry_adapter_id = 1;
    publish(&publisher, foreign);
    publish(&publisher, IpAddressAssignedEvent::from_ip_text("10.0.0.9"));
    wait_row(&view, 0, "10.0.0.9");
    assert_eq!(
        view.lines()[6].text, "OK   --",
        "adapter 1's fact must not take adapter 0's live row"
    );
}
