use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusHost, PublishResult};
use dali2rust_test_support::{command_frame, confirmation_frame, event_frame};

#[test]
fn three_command_subscribers_each_receive_one_frame() {
    let config = BusConfig {
        commands_ingress: 8,
        ..BusConfig::default()
    };
    let (_host, publisher, (a, b, c)) = BusHost::spawn(
        config,
        |reg| {
            (
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            )
        },
    );
    let f = command_frame();
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, f.clone()),
        PublishResult::Queued
    );
    let fa = a
        .recv_timeout(Duration::from_millis(500))
        .expect("subscriber A");
    let fb = b
        .recv_timeout(Duration::from_millis(500))
        .expect("subscriber B");
    let fc = c
        .recv_timeout(Duration::from_millis(500))
        .expect("subscriber C");
    assert_eq!(fa.postcard_wire_len(), fb.postcard_wire_len());
    assert_eq!(fa.postcard_wire_len(), fc.postcard_wire_len());
}

#[test]
fn confirmations_and_events_subscribers_each_receive_one_frame() {
    let config = BusConfig {
        confirmations_ingress: 8,
        events_ingress: 8,
        ..BusConfig::default()
    };
    let (_host, publisher, (c1, c2, e1, e2)) = BusHost::spawn(
        config,
        |reg| {
            (
                reg.subscribe_confirmations(16),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Confirmations, confirmation_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, event_frame()),
        PublishResult::Queued
    );
    let _ = c1.recv_timeout(Duration::from_millis(500)).unwrap();
    let _ = c2.recv_timeout(Duration::from_millis(500)).unwrap();
    let _ = e1.recv_timeout(Duration::from_millis(500)).unwrap();
    let _ = e2.recv_timeout(Duration::from_millis(500)).unwrap();
}
