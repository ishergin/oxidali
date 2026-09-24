use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult};
use dali2rust_test_support::{command_frame, event_frame, projected_event_frame, wait_until};

const RECV: Duration = Duration::from_millis(500);

const DALI_EVENT: &[&str] = &["DaliEventPayload"];
const RUNTIME_EVENT: &[&str] = &["RuntimeStateChangedEvent"];

fn publish_ok(publisher: &dali2rust_bus::BusPublisher, frame: BusFrame) {
    assert_eq!(
        publisher.try_publish(BusChannel::Events, frame),
        PublishResult::Queued
    );
}

#[test]
fn declared_subscriber_receives_routed_event_and_undeclared_does_not() {
    let (host, publisher, (declared, undeclared)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(4, DALI_EVENT),
            reg.subscribe_events(4, RUNTIME_EVENT),
        )
    });

    publish_ok(&publisher, event_frame());
    let frame = declared
        .recv_timeout(RECV)
        .expect("declared subscriber receives the routed event");
    assert!(matches!(frame, BusFrame::Event(_)));
    assert!(
        undeclared.try_recv().is_err(),
        "an undeclared kind must not reach this inbox"
    );
    wait_until(
        || host.counters_snapshot().event_subscribers[0].delivered == 1,
        RECV,
    );
    let c = host.counters_snapshot();
    assert_eq!(c.event_subscribers[0].delivered, 1);
    assert_eq!(c.event_subscribers[1].delivered, 0);
}

#[test]
fn zero_target_event_is_silent_and_injects_no_rejection() {
    let (host, publisher, (other, conf)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(4, RUNTIME_EVENT),
            reg.subscribe_confirmations(8),
        )
    });

    publish_ok(&publisher, event_frame());
    publish_ok(&publisher, projected_event_frame(2));
    other
        .recv_timeout(RECV)
        .expect("the declared kind still arrives after the observed-only one");
    assert!(conf.try_recv().is_err(), "no synthetic rejection for events");
    wait_until(
        || host.counters_snapshot().event_subscribers[0].delivered == 1,
        RECV,
    );
    let c = host.counters_snapshot();
    assert_eq!(c.event_subscribers[0].delivered, 1);
    assert_eq!(c.delivery_rejected_dropped, 0);
}

#[test]
fn every_declaring_subscriber_receives_a_shared_kind() {
    let (host, publisher, (a, b)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(4, DALI_EVENT),
            reg.subscribe_events(4, DALI_EVENT),
        )
    });

    publish_ok(&publisher, event_frame());
    a.recv_timeout(RECV).expect("first declaring subscriber");
    b.recv_timeout(RECV).expect("second declaring subscriber");
    wait_until(
        || host.counters_snapshot().event_subscribers[1].delivered == 1,
        RECV,
    );
    let c = host.counters_snapshot();
    assert_eq!(c.event_subscribers[0].delivered, 1);
    assert_eq!(c.event_subscribers[1].delivered, 1);
}

#[test]
fn funneled_inbox_routes_commands_and_events_by_their_own_sets() {
    let (_host, publisher, funnel) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_commands_and_events(8, &["DaliCommandPayload"], RUNTIME_EVENT)
    });

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
    publish_ok(&publisher, event_frame());
    publish_ok(&publisher, projected_event_frame(2));

    let first = funnel.recv_timeout(RECV).expect("routed command");
    assert!(matches!(first, BusFrame::Command(_)));
    let second = funnel.recv_timeout(RECV).expect("declared event");
    assert!(matches!(second, BusFrame::Event(_)));
    assert!(
        funnel.try_recv().is_err(),
        "the undeclared kind must not have been funneled"
    );
}

#[test]
fn overflowed_declared_target_counts_receiver_overflow_and_peers_still_deliver() {
    let (host, publisher, (tiny, healthy)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(1, DALI_EVENT),
            reg.subscribe_events(8, DALI_EVENT),
        )
    });

    for _ in 0..4 {
        publish_ok(&publisher, event_frame());
    }
    for _ in 0..4 {
        healthy.recv_timeout(RECV).expect("healthy peer delivery");
    }
    wait_until(
        || host.counters_snapshot().event_subscribers[1].delivered == 4,
        RECV,
    );
    let c = host.counters_snapshot();
    assert_eq!(c.event_subscribers[1].delivered, 4);
    assert!(
        c.event_subscribers[0].receiver_overflow > 0,
        "the full declared inbox must count its own loss"
    );
    assert_eq!(
        c.event_subscribers[0].delivered + c.event_subscribers[0].receiver_overflow,
        4,
        "every routed frame is either delivered or counted as overflow"
    );
    drop(tiny);
}

#[test]
#[should_panic(expected = "subscribe_events: NotAnEvent is not a payload variant")]
fn unknown_event_name_panics_at_spawn() {
    let _ = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events(4, &["NotAnEvent"])
    });
}

#[test]
fn probe_counts_only_routed_drops() {
    let (_host, publisher, (rx, index)) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events_indexed(1, RUNTIME_EVENT)
    });
    let probe = publisher.event_inbox_probe(index);

    for _ in 0..4 {
        publish_ok(&publisher, event_frame());
    }
    for corr in 0..4 {
        publish_ok(&publisher, projected_event_frame(corr));
    }
    dali2rust_test_support::wait_until(
        || probe.dropped_total() >= 3,
        Duration::from_millis(500),
    );
    assert_eq!(
        probe.dropped_total(),
        3,
        "probe counts routed drops only, never observed-only kinds"
    );
    rx.recv_timeout(RECV).expect("first declared frame fits");
}
