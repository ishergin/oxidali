mod support;

use support::{
    dapc_frame, dt8_read_frame, interactive_target_state_frame, poller_read_frame, read_frame_of,
    spawn_mock_worker, EVENT_BUDGET,
};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    CommandArrivalObserver, PublishResult,
};
use dali2rust_contracts::msg::{BusEventPayload, DaliAttributeGroup, Origin};
use dali2rust_dali_runtime::{
    DaliWorkerCounters, WireArrivalObserver, DALI_WORKER_HANDLED_COMMANDS,
    OPERATOR_BACKGROUND_QUEUE_DEPTH,
};
use dali2rust_platform::dali::WireActivity;
use dali2rust_test_support::wait_until;

const PRODUCTION_WORKER_INBOX: usize = 128;
const EVENT_INBOX: usize = 256;
const FIRST_READ: u64 = 1_000;
const SETPOINT: u64 = 9_001;
const DRAIN_BUDGET: Duration = Duration::from_secs(20);

struct FullQueue {
    publisher: BusPublisher,
    events: BusSubscriberRx,
    transport: Arc<Mutex<MockDaliTransport>>,
    unblock: Arc<AtomicBool>,
    counters: Arc<DaliWorkerCounters>,
    host: BusHost,
    _worker: JoinHandle<()>,
}

fn spawn_bus(interactive: &Arc<WireActivity>) -> (BusHost, BusPublisher, (BusSubscriberRx, BusSubscriberRx)) {
    let observer: Arc<dyn CommandArrivalObserver> =
        Arc::new(WireArrivalObserver::new(Arc::clone(interactive), BusId::default()));
    BusHost::spawn_with_command_observer(BusConfig::default(), Some(observer), |reg| {
        (
            reg.subscribe_commands(PRODUCTION_WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
            reg.subscribe_events(EVENT_INBOX, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
        )
    })
}

fn operator_read(at: u16) -> BusFrame {
    let short = u8::try_from(usize::from(at) % OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("a short address");
    read_frame_of(FIRST_READ + u64::from(at), Origin::Api, short, DaliAttributeGroup::RuntimeStatus)
}

fn held_wire(
    worker_cmd: BusSubscriberRx,
    publisher: &BusPublisher,
    interactive: Arc<WireActivity>,
) -> (Arc<Mutex<MockDaliTransport>>, Arc<AtomicBool>, Arc<DaliWorkerCounters>, JoinHandle<()>) {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let unblock = transport.lock().expect("mock lock").get_unblock_flag();
    transport.lock().expect("mock lock").block_next_send();
    let counters = Arc::new(DaliWorkerCounters::default());
    let worker = spawn_mock_worker(worker_cmd, &transport, publisher, &counters, interactive);
    (transport, unblock, counters, worker)
}

fn delivered_to_worker(host: &BusHost) -> u32 {
    host.counters_snapshot().command_subscribers.first().map_or(0, |sub| sub.delivered)
}

fn a_full_queue_held_on_the_wire() -> FullQueue {
    let interactive = Arc::new(WireActivity::new());
    let (host, publisher, (worker_cmd, events)) = spawn_bus(&interactive);
    let reads = u16::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("one read per short address");
    for at in 0..reads {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, operator_read(at)),
            PublishResult::Queued,
            "an idle worker refused read {at}"
        );
    }
    wait_until(|| delivered_to_worker(&host) == u32::from(reads), EVENT_BUDGET);

    let (transport, unblock, counters, worker) = held_wire(worker_cmd, &publisher, interactive);
    wait_until(|| counters.commands_handled.load(Ordering::Relaxed) >= 1, EVENT_BUDGET);
    FullQueue { publisher, events, transport, unblock, counters, host, _worker: worker }
}

fn read_outcomes_before_the_setpoint(events: &BusSubscriberRx) -> usize {
    let deadline = Instant::now() + EVENT_BUDGET;
    let mut reads = 0;
    while let Ok(frame) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let BusFrame::Event(event) = frame else {
            continue;
        };
        match event.payload {
            BusEventPayload::DaliTargetStateAppliedEvent(_) if event.meta.correlation_id == SETPOINT => {
                return reads;
            }
            BusEventPayload::DaliAttributeReadOutcomesEvent(_) => reads += 1,
            _ => {}
        }
    }
    panic!("the operator's setpoint was never applied; {reads} reads ended first");
}

#[test]
fn an_operators_read_past_the_queue_depth_is_refused_at_ingress_and_never_queued() {
    let run = a_full_queue_held_on_the_wire();

    assert_eq!(
        run.publisher.try_publish(BusChannel::Commands, dt8_read_frame(FIRST_READ - 1)),
        PublishResult::DroppedIngressFull,
        "a full queue took one more operator read"
    );
    assert_eq!(run.host.counters_snapshot().commands.ingress_overflow, 1);
    assert_eq!(
        delivered_to_worker(&run.host),
        u32::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u32"),
        "the refused read reached the worker's inbox"
    );
    assert_eq!(
        run.publisher.try_publish(BusChannel::Commands, poller_read_frame(FIRST_READ - 2)),
        PublishResult::Queued,
        "the poller paces itself and must not be refused"
    );
}

#[test]
fn an_operators_setpoint_is_admitted_to_a_full_queue_and_runs_before_it() {
    let run = a_full_queue_held_on_the_wire();

    assert_eq!(
        run.publisher.try_publish(BusChannel::Commands, interactive_target_state_frame(SETPOINT)),
        PublishResult::Queued,
        "a full queue refused the operator's setpoint"
    );
    let all = u32::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u32");
    wait_until(|| delivered_to_worker(&run.host) == all + 1, EVENT_BUDGET);
    run.unblock.store(false, Ordering::Release);

    let ahead = read_outcomes_before_the_setpoint(&run.events);
    assert!(ahead <= 1, "{ahead} queued reads ran before the operator's setpoint");
    let frames = run.transport.lock().expect("mock lock").sent_frames();
    assert!(frames.contains(&dapc_frame()), "the setpoint never reached the wire: {frames:?}");
}

#[test]
fn reads_arriving_while_the_wire_is_held_fill_the_queue_only_to_its_depth() {
    let interactive = Arc::new(WireActivity::new());
    let (host, publisher, (worker_cmd, _events)) = spawn_bus(&interactive);
    let (_transport, unblock, counters, _worker) = held_wire(worker_cmd, &publisher, interactive);
    assert_eq!(publisher.try_publish(BusChannel::Commands, operator_read(0)), PublishResult::Queued);
    wait_until(|| counters.commands_handled.load(Ordering::Relaxed) >= 1, EVENT_BUDGET);

    let depth = u16::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u16");
    let admitted = (1..=depth + LATE_EXTRA)
        .filter(|at| publisher.try_publish(BusChannel::Commands, operator_read(*at)) == PublishResult::Queued)
        .count();

    assert_eq!(admitted, OPERATOR_BACKGROUND_QUEUE_DEPTH - 1, "the running read holds the last place");
    assert_eq!(host.counters_snapshot().commands.ingress_overflow, u32::from(LATE_EXTRA) + 1);
    wait_until(|| delivered_to_worker(&host) == u32::from(depth), EVENT_BUDGET);
    unblock.store(false, Ordering::Release);
}

const LATE_EXTRA: u16 = 36;

#[test]
fn paced_reads_held_in_the_queue_take_no_place_from_the_operator() {
    let interactive = Arc::new(WireActivity::new());
    let (host, publisher, (worker_cmd, _events)) = spawn_bus(&interactive);
    let paced = u16::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u16");
    for at in 0..paced {
        let read = read_frame_of(FIRST_READ + u64::from(at), Origin::Poller, 0, DaliAttributeGroup::RuntimeStatus);
        assert_eq!(publisher.try_publish(BusChannel::Commands, read), PublishResult::Queued);
    }
    wait_until(|| delivered_to_worker(&host) == u32::from(paced), EVENT_BUDGET);
    let (_transport, unblock, counters, _worker) = held_wire(worker_cmd, &publisher, interactive);
    wait_until(|| counters.commands_handled.load(Ordering::Relaxed) >= 1, EVENT_BUDGET);

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, operator_read(0)),
        PublishResult::Queued,
        "the poller's queued reads took the operator's places"
    );
    unblock.store(false, Ordering::Release);
}

#[test]
fn re_reads_of_one_device_a_take_collapses_give_their_places_back_at_that_take() {
    let interactive = Arc::new(WireActivity::new());
    let (host, publisher, (worker_cmd, _events)) = spawn_bus(&interactive);
    let depth = u16::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u16");
    for at in 0..depth {
        let reread = read_frame_of(FIRST_READ + u64::from(at), Origin::Api, REREAD_SHORT, DaliAttributeGroup::RuntimeStatus);
        assert_eq!(publisher.try_publish(BusChannel::Commands, reread), PublishResult::Queued);
    }
    assert_eq!(publisher.try_publish(BusChannel::Commands, operator_read(0)), PublishResult::DroppedIngressFull);
    wait_until(|| delivered_to_worker(&host) == u32::from(depth), EVENT_BUDGET);
    let (_transport, unblock, counters, _worker) = held_wire(worker_cmd, &publisher, interactive);
    wait_until(|| counters.commands_handled.load(Ordering::Relaxed) >= 1, EVENT_BUDGET);

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, operator_read(0)),
        PublishResult::Queued,
        "re-reads the take collapsed into one still held their places"
    );
    unblock.store(false, Ordering::Release);
}

const REREAD_SHORT: u8 = 5;

#[test]
fn the_queue_admits_an_operators_read_again_once_the_worker_has_worked_it_off() {
    let run = a_full_queue_held_on_the_wire();
    assert_eq!(
        run.publisher.try_publish(BusChannel::Commands, dt8_read_frame(FIRST_READ - 1)),
        PublishResult::DroppedIngressFull
    );
    run.unblock.store(false, Ordering::Release);

    let all = u32::try_from(OPERATOR_BACKGROUND_QUEUE_DEPTH).expect("depth fits u32");
    wait_until(|| run.counters.commands_handled.load(Ordering::Relaxed) >= all, DRAIN_BUDGET);
    assert_eq!(
        run.publisher.try_publish(BusChannel::Commands, dt8_read_frame(FIRST_READ - 3)),
        PublishResult::Queued,
        "the queue stayed shut after the worker worked it off"
    );
    assert_eq!(run.host.counters_snapshot().commands.ingress_overflow, 1, "only the first was refused");
}
