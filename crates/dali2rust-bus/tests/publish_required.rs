use std::time::{Duration, Instant};

use dali2rust_bus::{
    publish_required, BusChannel, BusConfig, BusHost, PublishResult,
    HANDLER_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_UNCAPPED,
};
use dali2rust_test_support::event_frame;

fn fill_events_ingress(publisher: &dali2rust_bus::BusPublisher) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if publisher.try_publish(BusChannel::Events, event_frame()) != PublishResult::Queued {
            return;
        }
    }
    panic!("events ingress never refused a publish — cannot test required delivery");
}

#[test]
fn a_required_publish_survives_a_full_ingress() {
    let (host, publisher, _rx) = BusHost::spawn(
        BusConfig {
            events_ingress: 4,
            ..BusConfig::default()
        },
        |reg| reg.subscribe_events(16, &[]),
    );
    fill_events_ingress(&publisher);

    let outcome = publish_required(
        &publisher,
        BusChannel::Events,
        event_frame(),
        &REQUIRED_PUBLISH_BACKOFF_MS,
        REQUIRED_PUBLISH_UNCAPPED,
        "test",
    );

    assert!(
        outcome.queued,
        "a required frame was dropped although the bus task was draining normally"
    );
    assert!(
        outcome.retries > 0,
        "the ingress was not actually full — this run proved nothing"
    );
    let counters = host.counters_snapshot();
    assert_eq!(
        counters.events.publish_queued + counters.events.ingress_overflow,
        counters.events.publish_attempted,
        "every attempt is either queued or counted as overflow"
    );
}

#[test]
fn an_uncontended_required_publish_costs_no_retries() {
    let (_host, publisher, _rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_events(16, &[]));

    let started = Instant::now();
    let outcome = publish_required(
        &publisher,
        BusChannel::Events,
        event_frame(),
        &REQUIRED_PUBLISH_BACKOFF_MS,
        REQUIRED_PUBLISH_UNCAPPED,
        "test",
    );

    assert!(outcome.queued);
    assert_eq!(outcome.retries, 0);
    assert!(
        started.elapsed() < Duration::from_millis(20),
        "the uncontended path slept: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_budget_caps_the_total_sleep_a_required_publish_may_add() {
    let (_host, publisher, _rx) = BusHost::spawn(
        BusConfig {
            events_ingress: 4,
            ..BusConfig::default()
        },
        |reg| reg.subscribe_events(16, &[]),
    );
    fill_events_ingress(&publisher);

    const BUDGET_MS: u32 = 80;
    let started = Instant::now();
    let outcome = publish_required(
        &publisher,
        BusChannel::Events,
        event_frame(),
        &REQUIRED_PUBLISH_BACKOFF_MS,
        BUDGET_MS,
        "test",
    );

    assert!(
        outcome.slept_ms <= BUDGET_MS,
        "slept {} ms against a {BUDGET_MS} ms budget",
        outcome.slept_ms
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "the budget did not bound the wall clock: {:?}",
        started.elapsed()
    );
}

#[test]
fn an_exhausted_budget_still_attempts_the_publish() {
    let (_host, publisher, _rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_events(16, &[]));

    let started = Instant::now();
    let outcome = publish_required(
        &publisher,
        BusChannel::Events,
        event_frame(),
        &REQUIRED_PUBLISH_BACKOFF_MS,
        0,
        "test",
    );

    assert!(outcome.queued, "an empty budget must not suppress the attempt");
    assert_eq!(outcome.retries, 0);
    assert_eq!(outcome.slept_ms, 0);
    assert!(started.elapsed() < Duration::from_millis(20));
}

#[test]
fn the_handler_schedule_stays_within_one_bus_drain_interval() {
    const BUS_DRAIN_INTERVAL_MS: u64 = 10;
    let total: u64 = HANDLER_PUBLISH_BACKOFF_MS.iter().sum();
    assert_eq!(HANDLER_PUBLISH_BACKOFF_MS[0], 0, "the first attempt never sleeps");
    assert!(
        total >= BUS_DRAIN_INTERVAL_MS,
        "a handler retry that cannot outlast one drain interval buys nothing"
    );
    assert!(
        total <= 2 * BUS_DRAIN_INTERVAL_MS,
        "{total} ms on the httpd task is downtime for all ten sockets"
    );
}
