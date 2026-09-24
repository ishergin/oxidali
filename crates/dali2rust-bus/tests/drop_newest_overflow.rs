use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusPublisher, PublishResult};
use dali2rust_test_support::{command_frame, confirmation_frame, event_frame};

const MAX_PUBLISH_ATTEMPTS: usize = 256;

fn publish_until_drop(
    publisher: &BusPublisher,
    channel: BusChannel,
    make_frame: impl Fn() -> BusFrame,
) -> Vec<PublishResult> {
    let mut results = Vec::new();
    for _ in 0..MAX_PUBLISH_ATTEMPTS {
        let result = publisher.try_publish(channel, make_frame());
        let dropped = result == PublishResult::DroppedIngressFull;
        results.push(result);
        if dropped {
            break;
        }
    }
    results
}

fn assert_drop_newest(results: &[PublishResult]) {
    assert_eq!(
        results.last(),
        Some(&PublishResult::DroppedIngressFull),
        "no drop within {MAX_PUBLISH_ATTEMPTS} publishes: {results:?}"
    );
    assert!(
        results[..results.len() - 1]
            .iter()
            .all(|r| *r == PublishResult::Queued),
        "drop-newest: earlier publishes must stay queued: {results:?}"
    );
}

#[test]
fn commands_ingress_drop_newest_bus009() {
    let config = BusConfig {
        commands_ingress: 2,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    let results = publish_until_drop(&publisher, BusChannel::Commands, command_frame);
    assert_drop_newest(&results);
    assert!(host.counters_snapshot().commands.ingress_overflow >= 1);
}

#[test]
fn commands_ingress_queues_after_drain_bus010() {
    let config = BusConfig {
        commands_ingress: 1,
        ..BusConfig::default()
    };
    let (_host, publisher, rx) = BusHost::spawn(
        config,
        |reg| reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
    );
    let mut results = Vec::new();
    results.push(
        publisher.try_publish(BusChannel::Commands, command_frame()),
    );
    let _ = rx.recv_timeout(Duration::from_millis(500)).unwrap();
    results.push(
        publisher.try_publish(BusChannel::Commands, command_frame()),
    );
    assert!(results.len() >= 2);
    assert_eq!(results[1], PublishResult::Queued);
}

#[test]
fn confirmations_ingress_overflow_bus011() {
    let config = BusConfig {
        confirmations_ingress: 1,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(config, |reg| reg.subscribe_confirmations(4));
    let results = publish_until_drop(&publisher, BusChannel::Confirmations, confirmation_frame);
    assert_drop_newest(&results);
    assert!(host.counters_snapshot().confirmations.ingress_overflow >= 1);
}

#[test]
fn events_ingress_overflow_bus012() {
    let config = BusConfig {
        events_ingress: 1,
        ..BusConfig::default()
    };
    let (host, publisher, _rx) = BusHost::spawn(config, |reg| reg.subscribe_events(4, dali2rust_contracts::msg::EVENT_VARIANT_NAMES));
    let results = publish_until_drop(&publisher, BusChannel::Events, event_frame);
    assert_drop_newest(&results);
    assert!(host.counters_snapshot().events.ingress_overflow >= 1);
}
