use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusHost};
use dali2rust_test_support::{
    command_frame, confirmation_frame, event_frame, publish_until_refused, wait_until,
};

const WANT_REFUSALS: u32 = 5;

#[test]
fn commands_burst_overflow_matches_attempted_bus005() {
    let config = BusConfig {
        commands_ingress: 1,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    let tally = publish_until_refused(
        &publisher,
        BusChannel::Commands,
        command_frame,
        WANT_REFUSALS,
    );
    let c = host.counters_snapshot();
    assert!(c.commands.ingress_overflow >= WANT_REFUSALS);
    assert_eq!(c.commands.ingress_overflow, tally.refused);
    assert_eq!(c.commands.publish_attempted, tally.attempted);
    assert_eq!(
        c.commands.publish_queued + c.commands.ingress_overflow,
        c.commands.publish_attempted
    );
}

#[test]
fn events_burst_overflow_matches_attempted_bus006() {
    let config = BusConfig {
        events_ingress: 2,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(config, |reg| reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES));
    let tally = publish_until_refused(&publisher, BusChannel::Events, event_frame, WANT_REFUSALS);
    let c = host.counters_snapshot();
    assert!(c.events.ingress_overflow >= WANT_REFUSALS);
    assert_eq!(c.events.ingress_overflow, tally.refused);
    assert_eq!(c.events.publish_attempted, tally.attempted);
    assert_eq!(
        c.events.publish_queued + c.events.ingress_overflow,
        c.events.publish_attempted
    );
}

#[test]
fn mixed_channel_small_ingress_bus007() {
    let config = BusConfig {
        commands_ingress: 1,
        confirmations_ingress: 1,
        events_ingress: 1,
        ..BusConfig::default()
    };
    let (host, publisher, _subs) = BusHost::spawn(
        config,
        |reg| {
            (
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        },
    );
    let overflow_channel = |channel: BusChannel, make: &dyn Fn() -> dali2rust_bus::BusFrame| {
        wait_until(
            || {
                for _ in 0..3 {
                    let _ = publisher.try_publish(channel, make());
                }
                let c = host.counters_snapshot();
                match channel {
                    BusChannel::Commands => c.commands.ingress_overflow >= 1,
                    BusChannel::Confirmations => c.confirmations.ingress_overflow >= 1,
                    BusChannel::Events => c.events.ingress_overflow >= 1,
                }
            },
            Duration::from_secs(2),
        );
    };
    overflow_channel(BusChannel::Commands, &command_frame);
    overflow_channel(BusChannel::Confirmations, &confirmation_frame);
    overflow_channel(BusChannel::Events, &event_frame);
    let c = host.counters_snapshot();
    assert!(c.commands.ingress_overflow >= 1);
    assert!(c.confirmations.ingress_overflow >= 1);
    assert!(c.events.ingress_overflow >= 1);
}

#[test]
fn publish_attempted_equals_calls_bus008() {
    let config = BusConfig {
        commands_ingress: 2,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    for _ in 0..10 {
        let _ = publisher.try_publish(BusChannel::Commands, command_frame());
    }
    wait_until(
        || host.counters_snapshot().commands.publish_attempted == 10,
        Duration::from_millis(500),
    );
    let c = host.counters_snapshot();
    assert_eq!(c.commands.publish_attempted, 10);
    assert_eq!(
        c.commands.publish_queued + c.commands.ingress_overflow,
        c.commands.publish_attempted
    );
}
