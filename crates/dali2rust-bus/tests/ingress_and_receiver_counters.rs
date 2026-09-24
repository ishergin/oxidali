use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusHost, PublishResult};
use dali2rust_test_support::{
    command_frame, confirmation_frame, event_frame, publish_until_refused, wait_until,
};

#[test]
fn ingress_overflow_matches_publish_attempts_bus019() {
    let config = BusConfig {
        commands_ingress: 1,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    let tally = publish_until_refused(&publisher, BusChannel::Commands, command_frame, 1);
    let c = host.counters_snapshot();
    assert!(c.commands.ingress_overflow >= 1);
    assert_eq!(c.commands.ingress_overflow, tally.refused);
    assert_eq!(c.commands.publish_attempted, tally.attempted);
}

#[test]
fn receiver_overflow_after_inbox_full_bus020() {
    let config = BusConfig {
        commands_ingress: 8,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(1, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    for _ in 0..3 {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, command_frame()),
            PublishResult::Queued
        );
    }
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
}

#[test]
fn independent_channel_counters_bus021() {
    let (host, publisher, ()) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            let _ = reg.subscribe_commands(8, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            let _ = reg.subscribe_confirmations(8);
            let _ = reg.subscribe_events(8, dali2rust_contracts::msg::EVENT_VARIANT_NAMES);
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Confirmations, confirmation_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, event_frame()),
        PublishResult::Queued
    );
    wait_until(
        || {
            let counters = host.counters_snapshot();
            counters.commands.publish_attempted == 1
                && counters.confirmations.publish_attempted == 1
                && counters.events.publish_attempted == 1
        },
        Duration::from_millis(500),
    );
    let c = host.counters_snapshot();
    assert_eq!(c.commands.publish_attempted, 1);
    assert_eq!(c.confirmations.publish_attempted, 1);
    assert_eq!(c.events.publish_attempted, 1);
    assert_eq!(c.commands.publish_queued, 1);
    assert_eq!(c.confirmations.publish_queued, 1);
    assert_eq!(c.events.publish_queued, 1);
}
