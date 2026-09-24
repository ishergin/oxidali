use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, CommandArrivalObserver, PublishResult,
};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{BusCommandPayload, BusEnvelope, DaliCommandPayload, Origin};
use dali2rust_test_support::{command_frame, event_frame};

#[derive(Default)]
struct RecordingObserver {
    seen: AtomicU32,
    origins: Mutex<Vec<Origin>>,
    variants: Mutex<Vec<&'static str>>,
}

impl CommandArrivalObserver for RecordingObserver {
    fn observe_command(&self, meta: &BusEnvelope, payload: &BusCommandPayload) {
        self.seen.fetch_add(1, Ordering::Relaxed);
        self.origins.lock().expect("origins lock").push(meta.origin);
        self.variants
            .lock()
            .expect("variants lock")
            .push(payload.variant_name());
    }
}

fn spawn_with(observer: &Arc<RecordingObserver>) -> (BusHost, dali2rust_bus::BusPublisher) {
    let (host, publisher, ()) = BusHost::spawn_with_command_observer(
        BusConfig::default(),
        Some(Arc::clone(observer) as Arc<dyn CommandArrivalObserver>),
        |_reg| (),
    );
    (host, publisher)
}

fn command_with_origin(origin: Origin) -> BusFrame {
    BusFrame::command(command_envelope(
        0,
        7,
        0,
        Some(origin),
        DaliCommandPayload {
            wire_address: 2,
            command: 5,
            repeat_count: 1,
            raw_mode: false,
            raw_expects_backward: false,
        },
    ))
}

#[test]
fn every_published_command_is_observed_with_its_origin() {
    let observer = Arc::new(RecordingObserver::default());
    let (_host, publisher) = spawn_with(&observer);

    for origin in [Origin::Api, Origin::Poller] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, command_with_origin(origin)),
            PublishResult::Queued
        );
    }

    assert_eq!(observer.seen.load(Ordering::Relaxed), 2);
    assert_eq!(
        *observer.origins.lock().expect("origins lock"),
        vec![Origin::Api, Origin::Poller]
    );
}

#[test]
fn a_frame_rejected_before_ingress_is_never_announced_as_arrived() {
    let observer = Arc::new(RecordingObserver::default());
    let (_host, publisher) = spawn_with(&observer);

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, event_frame()),
        PublishResult::RejectedKindMismatch
    );
    assert_eq!(observer.seen.load(Ordering::Relaxed), 0);
}

#[test]
fn confirmations_and_events_are_not_command_arrivals() {
    let observer = Arc::new(RecordingObserver::default());
    let (_host, publisher) = spawn_with(&observer);

    assert_eq!(
        publisher.try_publish(BusChannel::Events, event_frame()),
        PublishResult::Queued
    );
    assert_eq!(observer.seen.load(Ordering::Relaxed), 0);
}

#[test]
fn a_bus_without_an_observer_publishes_exactly_as_before() {
    let (_host, publisher, ()) = BusHost::spawn(BusConfig::default(), |_reg| ());
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_frame()),
        PublishResult::Queued
    );
}
