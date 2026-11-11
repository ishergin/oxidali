use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, CommandArrivalObserver, PublishResult,
};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{BusCommandPayload, BusEnvelope, DaliCommandPayload, Origin};
use dali2rust_test_support::{command_frame, event_frame, hold_the_router};

#[derive(Default)]
struct RecordingObserver {
    refuses: bool,
    seen: AtomicU32,
    withdrawn: AtomicU32,
    origins: Mutex<Vec<Origin>>,
    variants: Mutex<Vec<&'static str>>,
}

impl CommandArrivalObserver for RecordingObserver {
    fn admits_command(&self, _meta: &BusEnvelope, _payload: &BusCommandPayload) -> bool {
        !self.refuses
    }

    fn observe_command(&self, meta: &BusEnvelope, payload: &BusCommandPayload) {
        self.seen.fetch_add(1, Ordering::Relaxed);
        self.origins.lock().expect("origins lock").push(meta.origin);
        self.variants
            .lock()
            .expect("variants lock")
            .push(payload.variant_name());
    }

    fn withdraw_command(&self, _meta: &BusEnvelope, _payload: &BusCommandPayload) {
        self.withdrawn.fetch_add(1, Ordering::Relaxed);
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

#[test]
fn an_admitted_command_the_full_ingress_drops_is_withdrawn_from_the_observer() {
    let observer = Arc::new(RecordingObserver::default());
    let config = BusConfig { commands_ingress: PARKED_INGRESS, ..BusConfig::default() };
    let (host, publisher, _worker) = BusHost::spawn_with_command_observer(
        config,
        Some(Arc::clone(&observer) as Arc<dyn CommandArrivalObserver>),
        |reg| reg.subscribe_commands(4, &["DaliCommandPayload"]),
    );

    let _parked = hold_the_router(&publisher);
    let dropped = (0..INGRESS_FLOOD)
        .filter(|_| {
            publisher.try_publish(BusChannel::Commands, command_with_origin(Origin::Api))
                == PublishResult::DroppedIngressFull
        })
        .count();
    assert_eq!(dropped, INGRESS_FLOOD - PARKED_INGRESS, "the parked router drained the ingress");

    let dropped = u32::try_from(dropped).expect("the flood fits u32");
    assert_eq!(observer.withdrawn.load(Ordering::Relaxed), dropped, "a dropped command kept its place");
    assert_eq!(host.counters_snapshot().commands.ingress_overflow, dropped);
}

const INGRESS_FLOOD: usize = 16;
const PARKED_INGRESS: usize = 2;

#[test]
fn a_command_the_observer_refuses_is_dropped_at_ingress_counted_and_never_announced() {
    let observer = Arc::new(RecordingObserver { refuses: true, ..RecordingObserver::default() });
    let (host, publisher, worker) = BusHost::spawn_with_command_observer(
        BusConfig::default(),
        Some(Arc::clone(&observer) as Arc<dyn CommandArrivalObserver>),
        |reg| reg.subscribe_commands(4, &["DaliCommandPayload"]),
    );

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, command_with_origin(Origin::Api)),
        PublishResult::DroppedIngressFull
    );
    assert_eq!(observer.seen.load(Ordering::Relaxed), 0, "a refused command preempted the wire");
    let counters = host.counters_snapshot();
    assert_eq!(counters.commands.ingress_overflow, 1, "the refusal went uncounted");
    assert_eq!(counters.commands.publish_queued, 0);
    assert_eq!(observer.withdrawn.load(Ordering::Relaxed), 0, "a refusal took no place to give back");
    assert!(worker.try_recv().is_err(), "a refused command reached its subscriber");
    assert_eq!(
        publisher.try_publish(BusChannel::Events, event_frame()),
        PublishResult::Queued,
        "the refusal reaches only commands"
    );
}
