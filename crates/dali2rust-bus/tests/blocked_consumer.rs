use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusHost, PublishResult};
use dali2rust_test_support::{command_frame, confirmation_frame, event_frame, wait_until};

#[test]
fn full_first_subscriber_does_not_block_second_bus001() {
    let config = BusConfig {
        commands_ingress: 8,
        ..BusConfig::default()
    };
    let (host, publisher, (s1, s2)) = BusHost::spawn(
        config,
        |reg| {
            (
                reg.subscribe_commands(1, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            )
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .iter()
                .map(|s| u64::from(s.receiver_overflow))
                .sum::<u64>()
                >= 1
        },
        Duration::from_millis(500),
    );
    let _ = s2.recv_timeout(Duration::from_millis(500)).unwrap();
    let _ = s1.try_recv();
}

#[test]
fn slow_event_subscriber_does_not_block_commands_bus003() {
    let (host, publisher, (_ev, cmd_rx)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_events(1, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
                reg.subscribe_commands(8, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            )
        },
    );
    for _ in 0..3 {
        assert_eq!(
            publisher.try_publish(BusChannel::Events, event_frame()),
            PublishResult::Queued
        );
    }
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    wait_until(
        || {
            host.counters_snapshot()
                .event_subscribers
                .iter()
                .map(|s| u64::from(s.receiver_overflow))
                .sum::<u64>()
                >= 1
        },
        Duration::from_millis(500),
    );
    let _ = cmd_rx.recv_timeout(Duration::from_millis(500)).unwrap();
}

#[test]
fn slow_confirmation_subscriber_does_not_block_commands_bus004() {
    let (_host, publisher, (_conf, cmd_rx)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_confirmations(1),
                reg.subscribe_commands(8, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            )
        },
    );
    for _ in 0..3 {
        assert_eq!(
            publisher.try_publish(BusChannel::Confirmations, confirmation_frame()),
            PublishResult::Queued
        );
    }
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    let _ = cmd_rx.recv_timeout(Duration::from_millis(500)).unwrap();
}
