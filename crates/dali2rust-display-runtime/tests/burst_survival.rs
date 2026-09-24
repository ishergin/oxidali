mod support;

use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, PublishResult};
use dali2rust_contracts::msg::IpAddressAssignedEvent;
use dali2rust_test_support::wait_until;

use support::{healthy_sample, spawn_display_worker_on_bus};

#[test]
fn display_worker_survives_a_foreign_event_burst() {
    let (_host, publisher, view, _worker) = spawn_display_worker_on_bus(
        BusConfig {
            events_ingress: 64,
            ..BusConfig::default()
        },
        healthy_sample(),
    );
    for i in 0..32u64 {
        let ev = dali2rust_contracts::bus::event_envelope(
            0,
            i,
            0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliEventPayload {
                wire_address: 2,
                command: 254,
                repeat_count: 1,
            },
        );
        assert_eq!(
            publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
            PublishResult::Queued
        );
    }
    let ev = dali2rust_contracts::bus::event_envelope(
        0,
        99,
        0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        IpAddressAssignedEvent::from_ip_text("10.0.0.9"),
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
    wait_until(
        || view.lines()[0].text.contains("10.0.0.9"),
        Duration::from_secs(3),
    );
}

#[test]
fn display_worker_survives_a_declared_event_burst() {
    let (_host, publisher, view, _worker) = spawn_display_worker_on_bus(
        BusConfig {
            events_ingress: 128,
            ..BusConfig::default()
        },
        healthy_sample(),
    );
    for i in 0..64u16 {
        let ev = dali2rust_contracts::bus::event_envelope(
            0,
            u64::from(i),
            0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliInputEventObservedEvent {
                registry_adapter_id: 0,
                scheme: 2,
                short_address: Some(5),
                device_group: None,
                instance_group: None,
                instance_number: Some(u8::try_from(i % 10).expect("instance")),
                instance_type: Some(1),
                event_info: 0x002,
                typed: dali2rust_contracts::msg::InputEventKind::Button,
                typed_value: 0x002,
                observed_at_ms: 0,
                observed_at_mono_ms: 0,
            },
        );
        assert_eq!(
            publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
            PublishResult::Queued
        );
    }
    wait_until(
        || view.lines()[5].text.contains("A05."),
        Duration::from_secs(3),
    );
    let ev = dali2rust_contracts::bus::event_envelope(
        0,
        999,
        0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        IpAddressAssignedEvent::from_ip_text("10.0.0.9"),
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
    wait_until(
        || view.lines()[0].text.contains("10.0.0.9"),
        Duration::from_secs(3),
    );
}
