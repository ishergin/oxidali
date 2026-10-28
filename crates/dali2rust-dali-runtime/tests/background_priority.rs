mod support;

use support::{
    api_read_frame, dapc_frame, interactive_target_state_frame, poller_read_frame,
    spawn_mock_worker, target_state_frame, EVENT_BUDGET, LEVEL, SHORT,
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
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusEventPayload, DaliBusHealthProbeCommand, DaliTargetScope,
    DeliveryStatus, ErrorCode, Origin,
};
use dali2rust_dali_runtime::{
    DaliWorkerCounters, WireArrivalObserver, DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_platform::dali::WireActivity;
use dali2rust_test_support::{
    recv_confirmation_for, recv_event_matching, try_recv_event_matching_envelope, wait_until,
};

type WorkerInboxes = (BusSubscriberRx, BusSubscriberRx, BusSubscriberRx);

fn spawn_bus(
    observer: Option<Arc<dyn CommandArrivalObserver>>,
) -> (BusHost, BusPublisher, WorkerInboxes) {
    BusHost::spawn_with_command_observer(BusConfig::default(), observer, |reg| {
        (
            reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
            reg.subscribe_events(64, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            reg.subscribe_confirmations(16),
        )
    })
}

fn arrival_observer(interactive: &Arc<WireActivity>) -> Option<Arc<dyn CommandArrivalObserver>> {
    Some(Arc::new(WireArrivalObserver::new(
        Arc::clone(interactive),
        BusId::default(),
    )))
}

struct ReadInFlight {
    publisher: BusPublisher,
    events: BusSubscriberRx,
    confirmations: BusSubscriberRx,
    transport: Arc<Mutex<MockDaliTransport>>,
    unblock: Arc<AtomicBool>,
    counters: Arc<DaliWorkerCounters>,
    interactive: Arc<WireActivity>,
    _worker: JoinHandle<()>,
    _host: BusHost,
}

fn start_blocked_on_first_frame(background: BusFrame) -> ReadInFlight {
    let interactive = Arc::new(WireActivity::new());
    let (host, publisher, (worker_cmd, events, confirmations)) = spawn_bus(arrival_observer(&interactive));

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let unblock = transport.lock().expect("mock lock").get_unblock_flag();
    transport.lock().expect("mock lock").block_next_send();

    let counters = Arc::new(DaliWorkerCounters::default());
    let worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        Arc::clone(&interactive),
    );

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, background),
        PublishResult::Queued
    );
    wait_until(
        || counters.commands_handled.load(Ordering::Relaxed) >= 1,
        Duration::from_secs(2),
    );
    ReadInFlight {
        publisher,
        events,
        confirmations,
        transport,
        unblock,
        counters,
        interactive,
        _worker: worker,
        _host: host,
    }
}

#[test]
fn a_queued_operator_command_runs_before_queued_background_reads() {
    let (host, publisher, worker_cmd) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS)
    });

    for corr in [21u64, 22, 23] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, poller_read_frame(corr)),
            PublishResult::Queued
        );
    }
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, interactive_target_state_frame(31)),
        PublishResult::Queued
    );
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .is_some_and(|sub| sub.delivered == 4)
        },
        Duration::from_secs(1),
    );

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        Arc::new(WireActivity::new()),
    );

    wait_until(
        || counters.commands_handled.load(Ordering::Relaxed) >= 4,
        Duration::from_secs(5),
    );
    let frames = transport.lock().expect("mock lock").sent_frames();
    assert!(
        frames.len() > 1,
        "the reads must reach the wire as well, or this proves nothing: {frames:?}"
    );
    assert_eq!(
        frames.first().copied(),
        Some(dapc_frame()),
        "the operator setpoint must be first on the wire: {frames:?}"
    );
}

#[test]
fn an_operator_command_preempts_a_background_read_already_in_flight() {
    let run = start_blocked_on_first_frame(poller_read_frame(41));

    assert_eq!(
        run.publisher
            .try_publish(BusChannel::Commands, interactive_target_state_frame(51)),
        PublishResult::Queued
    );
    assert!(
        !run.interactive.quiet_for(Duration::from_secs(5)),
        "a published-but-not-dequeued command must already count as activity"
    );

    run.unblock.store(false, Ordering::Release);

    let outcomes = wait_for_read_outcomes(&run.events);
    assert_eq!(
        outcomes,
        AttributeGroupReadOutcome::Preempted,
        "the in-flight section must be reported as preempted"
    );
    assert_eq!(run.counters.read_attributes_preempted.load(Ordering::Relaxed), 1);
    assert_eq!(
        run.counters.read_attributes_transport_aborts.load(Ordering::Relaxed),
        0,
        "a yield is not a transport abort"
    );
    assert_eq!(
        run.counters.read_attributes_contended_aborts.load(Ordering::Relaxed),
        0,
        "a yield is not contention"
    );

    assert!(
        wait_for_applied(&run.events, 51),
        "the operator's setpoint must be applied"
    );
    let frames = run.transport.lock().expect("mock lock").sent_frames();
    assert!(
        frames.contains(&dapc_frame()),
        "the operator's setpoint reached the wire: {frames:?}"
    );
}

#[test]
fn one_operator_command_does_not_preempt_another() {
    let interactive = Arc::new(WireActivity::new());
    let (_host, publisher, (worker_cmd, events, _confirmations)) = spawn_bus(arrival_observer(&interactive));

    for (corr, short) in [(61u64, SHORT), (62, SHORT + 1)] {
        assert_eq!(
            publisher.try_publish(
                BusChannel::Commands,
                interactive_target_state_for(corr, short)
            ),
            PublishResult::Queued
        );
    }

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        interactive,
    );

    for corr in [61u64, 62] {
        assert!(
            wait_for_applied(&events, corr),
            "interactive command {corr} must not be displaced by the other"
        );
    }
}

fn bus_health_probe_frame(correlation_id: u64) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(Origin::Poller),
        DaliBusHealthProbeCommand { registry_adapter_id: 0, query_power_failure: true },
    ))
}

fn broadcast_query(command: StandardCommand) -> u16 {
    DaliCommand::Standard { address: DaliAddress::Broadcast, command }.to_forward_frame().raw()
}

#[test]
fn an_operator_command_preempts_a_bus_health_probe_between_its_frames() {
    let run = start_blocked_on_first_frame(bus_health_probe_frame(71));

    assert_eq!(
        run.publisher
            .try_publish(BusChannel::Commands, interactive_target_state_frame(72)),
        PublishResult::Queued
    );
    run.unblock.store(false, Ordering::Release);

    let events = events_until_applied(&run.events, 72);
    let frames = run.transport.lock().expect("mock lock").sent_frames();
    assert_eq!(
        frames,
        vec![broadcast_query(StandardCommand::QueryControlGearPresent), dapc_frame()],
        "the probe yields after the frame on the wire and asks nothing more"
    );
    assert!(
        !events.iter().any(|event| matches!(event, BusEventPayload::DaliBusHealthProbedEvent(_))),
        "a probe cut short has no verdict"
    );
    let ended = recv_confirmation_for(&run.confirmations, 71, EVENT_BUDGET);
    assert_eq!(ended.status, DeliveryStatus::ExecutionFailed, "a yielded probe ends at once");
    assert_eq!(
        ended.confirmation.error.as_ref().map(|error| error.code),
        Some(ErrorCode::Preempted),
        "and says it yielded"
    );
    assert_eq!(
        run.counters.bus_health_probe_failed.load(Ordering::Relaxed),
        0,
        "a yield is not a failed probe"
    );
}

#[test]
fn a_bus_health_probe_the_wire_fails_ends_without_a_verdict_and_counts_as_failed() {
    let interactive = Arc::new(WireActivity::new());
    let (_host, publisher, (worker_cmd, _events, confirmations)) = spawn_bus(arrival_observer(&interactive));
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    transport
        .lock()
        .expect("mock lock")
        .expect_forward_frame_send_error(broadcast_query(StandardCommand::QueryControlGearPresent));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(worker_cmd, &transport, &publisher, &counters, interactive);

    assert_eq!(
        publisher.try_publish(BusChannel::Commands, bus_health_probe_frame(91)),
        PublishResult::Queued
    );
    let ended = recv_confirmation_for(&confirmations, 91, EVENT_BUDGET);
    assert_eq!(ended.status, DeliveryStatus::ExecutionFailed, "a failed probe ends at once");
    let code = ended.confirmation.error.as_ref().map(|error| error.code);
    assert!(
        code.is_some_and(|code| code != ErrorCode::Preempted),
        "and names a cause that is not a yield: {code:?}"
    );
    assert_eq!(counters.bus_health_probe_failed.load(Ordering::Relaxed), 1);
}

fn events_until_applied(events: &BusSubscriberRx, correlation_id: u64) -> Vec<BusEventPayload> {
    let deadline = Instant::now() + EVENT_BUDGET;
    let mut seen = Vec::new();
    while let Ok(frame) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let BusFrame::Event(event) = frame else {
            continue;
        };
        let applied = event.meta.correlation_id == correlation_id
            && matches!(event.payload, BusEventPayload::DaliTargetStateAppliedEvent(_));
        seen.push(event.payload.clone());
        if applied {
            return seen;
        }
    }
    panic!("setpoint {correlation_id} was not applied within {EVENT_BUDGET:?}: {seen:?}");
}

fn interactive_target_state_for(correlation_id: u64, short: u8) -> BusFrame {
    target_state_frame(correlation_id, Origin::Api, DaliTargetScope::Short, short, LEVEL)
}

fn wait_for_read_outcomes(events: &dali2rust_bus::BusSubscriberRx) -> AttributeGroupReadOutcome {
    let event = recv_event_matching(events, EVENT_BUDGET, |payload| {
        matches!(payload, BusEventPayload::DaliAttributeReadOutcomesEvent(_))
    });
    let BusEventPayload::DaliAttributeReadOutcomesEvent(body) = &event.payload else {
        unreachable!("predicate selected this variant");
    };
    body.identity
}

fn wait_for_applied(events: &dali2rust_bus::BusSubscriberRx, correlation_id: u64) -> bool {
    try_recv_event_matching_envelope(events, EVENT_BUDGET, |ev| {
        ev.meta.correlation_id == correlation_id
            && matches!(ev.payload, BusEventPayload::DaliTargetStateAppliedEvent(_))
    })
    .is_some()
}

#[test]
fn an_operator_setpoint_preempts_an_operator_started_read() {
    let run = start_blocked_on_first_frame(api_read_frame(81));

    assert_eq!(
        run.publisher
            .try_publish(BusChannel::Commands, interactive_target_state_frame(91)),
        PublishResult::Queued
    );
    run.unblock.store(false, Ordering::Release);

    assert_eq!(
        wait_for_read_outcomes(&run.events),
        AttributeGroupReadOutcome::Preempted,
        "an attended read yields to the operator like any other non-interactive work"
    );
    assert_eq!(run.counters.read_attributes_preempted.load(Ordering::Relaxed), 1);
    assert!(
        wait_for_applied(&run.events, 91),
        "the operator's setpoint must be applied"
    );
}

#[test]
fn the_queue_serves_interactive_then_attended_then_unattended() {
    let (host, publisher, (worker_cmd, events, _confirmations)) = spawn_bus(None);

    for frame in [
        poller_read_frame(101),
        api_read_frame(102),
        interactive_target_state_frame(103),
    ] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, frame),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .is_some_and(|sub| sub.delivered == 3)
        },
        Duration::from_secs(1),
    );

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        Arc::new(WireActivity::new()),
    );

    wait_until(
        || counters.commands_handled.load(Ordering::Relaxed) >= 3,
        Duration::from_secs(5),
    );

    let frames = transport.lock().expect("mock lock").sent_frames();
    assert_eq!(
        frames.first().copied(),
        Some(dapc_frame()),
        "the operator setpoint must be first on the wire: {frames:?}"
    );
    assert_eq!(
        read_outcome_correlations(&events, 2),
        vec![102, 101],
        "the operator's read must be served before the poller's"
    );
}

fn hcl_broadcast_target_state_frame(correlation_id: u64, level: u8) -> BusFrame {
    target_state_frame(
        correlation_id,
        Origin::Hcl,
        DaliTargetScope::Broadcast,
        0,
        level,
    )
}

#[test]
fn a_queued_scheduler_setpoint_does_not_overwrite_a_later_operator_setpoint() {
    const HCL_LEVEL: u8 = 40;
    let (host, publisher, (worker_cmd, _events, _confirmations)) = spawn_bus(None);

    for frame in [
        hcl_broadcast_target_state_frame(110, HCL_LEVEL),
        interactive_target_state_for(111, SHORT),
    ] {
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, frame),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .command_subscribers
                .first()
                .is_some_and(|sub| sub.delivered == 2)
        },
        Duration::from_secs(1),
    );

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_mock_worker(
        worker_cmd,
        &transport,
        &publisher,
        &counters,
        Arc::new(WireActivity::new()),
    );

    wait_until(
        || transport.lock().expect("mock lock").sent_frames().len() >= 2,
        Duration::from_secs(5),
    );

    let frames = transport.lock().expect("mock lock").sent_frames();
    let broadcast_dapc = DaliCommand::Standard {
        address: DaliAddress::Broadcast,
        command: StandardCommand::DirectArcPower { level: HCL_LEVEL },
    }
    .to_forward_frame()
    .raw();
    let hcl_at = frames
        .iter()
        .position(|f| *f == broadcast_dapc)
        .unwrap_or_else(|| panic!("scheduler broadcast never reached the wire: {frames:?}"));
    let operator_at = frames
        .iter()
        .position(|f| *f == dapc_frame())
        .unwrap_or_else(|| panic!("operator setpoint never reached the wire: {frames:?}"));
    assert!(
        hcl_at < operator_at,
        "the operator's setpoint must be the LAST of the two on the wire, \
         or the scheduler's sweep overwrites it: {frames:?}"
    );
}

fn read_outcome_correlations(events: &dali2rust_bus::BusSubscriberRx, count: usize) -> Vec<u64> {
    let mut seen = Vec::with_capacity(count);
    while seen.len() < count {
        let event = recv_event_matching(events, EVENT_BUDGET, |payload| {
            matches!(payload, BusEventPayload::DaliAttributeReadOutcomesEvent(_))
        });
        seen.push(event.meta.correlation_id);
    }
    seen
}
