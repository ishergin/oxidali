use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    PublishResult,
};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeGroup, DaliReadAttributesCommand, DaliReadMemoryBankCommand,
    ErrorCode, MemoryBankReadPreset, OperationWorkerSignal, OperationWorkerSignalEvent, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_platform::dali::DaliTransport;
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{hold_the_events_ingress_full, try_wait_until};

const SHORT: u8 = 5;
const GEAR_ANSWER: u8 = 0x04;
const READ_CORRELATION: u64 = 4_000;
const MEMORY_READ_CORRELATION: u64 = 4_001;
const CHUNK_LOST: &str = "memory_bank_chunk_lost";
const DEMO_SHORT: u8 = 0;
const BANK_ZERO_PREFIX: u16 = 3;
const EVENTS_INGRESS: usize = 4;
const WORKER_INBOX: usize = 16;
const SIGNAL_INBOX: usize = 64;
const STEP_DEADLINE: Duration = Duration::from_secs(10);

fn read_frame() -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        READ_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        DaliReadAttributesCommand {
            registry_adapter_id: 0,
            short_address: SHORT,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            memory_banks: MemoryBankReadPreset::None,
        },
    ))
}

fn spawn_worker<T: DaliTransport + Send + 'static>(
    commands: BusSubscriberRx,
    publisher: &BusPublisher,
    transport: &Arc<Mutex<T>>,
    counters: &Arc<DaliWorkerCounters>,
) -> JoinHandle<()> {
    spawn_dali_worker(
        commands,
        DaliController::new(Arc::clone(transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(counters),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    )
}

fn next_signal(
    signals: &BusSubscriberRx,
    correlation_id: u64,
) -> Option<OperationWorkerSignalEvent> {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())?;
        match signals.recv_timeout(remaining) {
            Ok(BusFrame::Event(envelope)) => match &envelope.payload {
                BusEventPayload::OperationWorkerSignalEvent(body)
                    if body.workflow_correlation_id == correlation_id =>
                {
                    return Some(body.clone())
                }
                _ => {}
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("signal tap died"),
        }
    }
}

fn spawn_bus() -> (BusHost, BusPublisher, BusSubscriberRx, BusSubscriberRx) {
    let (host, publisher, (worker_commands, signals)) = BusHost::spawn(
        BusConfig {
            events_ingress: EVENTS_INGRESS,
            ..BusConfig::default()
        },
        |reg| {
            (
                reg.subscribe_commands(WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(SIGNAL_INBOX, &["OperationWorkerSignalEvent"]),
            )
        },
    );
    (host, publisher, worker_commands, signals)
}

fn start_the_read(publisher: &BusPublisher, signals: &BusSubscriberRx) {
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, read_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        next_signal(signals, READ_CORRELATION).map(|started| started.signal),
        Some(OperationWorkerSignal::WorkerStarted),
        "the read never started"
    );
}

fn wait_for_the_refused_evidence(counters: &DaliWorkerCounters) {
    assert!(
        try_wait_until(
            || counters.evidence_publish_failed.load(Ordering::Relaxed) > 0,
            STEP_DEADLINE
        ),
        "the router was parked and the events ingress full, yet the read's evidence chunk \
         was never refused (reads handled {}, transport aborts {})",
        counters.semantic_read_attributes_handled.load(Ordering::Relaxed),
        counters.read_attributes_transport_aborts.load(Ordering::Relaxed)
    );
}

fn assert_the_read_failed(signals: &BusSubscriberRx) {
    let terminal = next_signal(signals, READ_CORRELATION)
        .expect("no terminal signal within the deadline after the router was released");
    assert_eq!(
        (terminal.signal, terminal.error.as_ref().map(|error| error.code)),
        (OperationWorkerSignal::WorkerFailed, Some(ErrorCode::CommandsIngressOverload)),
        "correlation {READ_CORRELATION} lost its evidence to the full events ingress and \
         still reported {terminal:?} — a green operation over a registry that never heard \
         the read"
    );
}

#[test]
fn a_lost_evidence_chunk_fails_the_read_instead_of_reporting_success() {
    let (_host, publisher, worker_commands, signals) = spawn_bus();
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    transport.lock().expect("mock").set_persistent_response(GEAR_ANSWER);
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker(worker_commands, &publisher, &transport, &counters);

    let wire_held = transport.lock().expect("mock");
    start_the_read(&publisher, &signals);
    let ingress = hold_the_events_ingress_full(&publisher);
    drop(wire_held);

    wait_for_the_refused_evidence(&counters);
    ingress.release();
    assert_the_read_failed(&signals);
}

fn memory_read_frame() -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        MEMORY_READ_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        DaliReadMemoryBankCommand {
            registry_adapter_id: 0,
            short_address: DEMO_SHORT,
            bank: 0,
            start: 0,
            length: BANK_ZERO_PREFIX,
        },
    ))
}

fn memory_read_counters(hold_the_ingress: bool) -> Arc<DaliWorkerCounters> {
    let (_host, publisher, worker_commands, _signals) = spawn_bus();
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker(worker_commands, &publisher, &transport, &counters);
    let wire_held = transport.lock().expect("sim");
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, memory_read_frame()),
        PublishResult::Queued
    );
    let ingress = hold_the_ingress.then(|| hold_the_events_ingress_full(&publisher));
    drop(wire_held);
    assert!(
        try_wait_until(
            || counters.semantic_read_memory_bank_handled.load(Ordering::Relaxed) > 0,
            STEP_DEADLINE
        ),
        "the memory bank read never finished"
    );
    drop(ingress);
    counters
}

#[test]
fn a_memory_bank_chunk_the_full_ingress_refused_is_not_counted_as_published() {
    let open = memory_read_counters(false);
    assert_eq!(
        open.memory_bank_chunk_events_published.load(Ordering::Relaxed),
        1,
        "control: a read of the gear's bank 0 prefix publishes one chunk"
    );
    let held = memory_read_counters(true);
    assert!(
        held.event_publish_failed.load(Ordering::Relaxed) > 0,
        "inconclusive: the events ingress was held full, yet no required publish gave up"
    );
    assert_eq!(
        held.memory_bank_chunk_events_published.load(Ordering::Relaxed),
        0,
        "the full ingress refused the chunk, yet it was counted as published"
    );
}

fn terminal_signal_of_a_memory_read(hold_the_ingress: bool) -> OperationWorkerSignalEvent {
    let (_host, publisher, worker_commands, signals) = spawn_bus();
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker(worker_commands, &publisher, &transport, &counters);
    let wire_held = transport.lock().expect("sim");
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, memory_read_frame()),
        PublishResult::Queued
    );
    assert_eq!(
        next_signal(&signals, MEMORY_READ_CORRELATION).map(|started| started.signal),
        Some(OperationWorkerSignal::WorkerStarted),
        "the memory read never started"
    );
    let ingress = hold_the_ingress.then(|| hold_the_events_ingress_full(&publisher));
    drop(wire_held);
    if let Some(ingress) = ingress {
        assert!(
            try_wait_until(
                || counters.event_publish_failed.load(Ordering::Relaxed) > 0,
                STEP_DEADLINE
            ),
            "inconclusive: the events ingress was held full, yet no required publish gave up"
        );
        ingress.release();
    }
    next_signal(&signals, MEMORY_READ_CORRELATION).expect("the memory read never ended")
}

#[test]
fn a_refused_memory_bank_chunk_fails_the_read() {
    let open = terminal_signal_of_a_memory_read(false);
    assert_eq!(
        open.signal,
        OperationWorkerSignal::WorkerSucceeded,
        "control: with the events ingress open the memory read ends {open:?}"
    );
    let held = terminal_signal_of_a_memory_read(true);
    let error = held.error.as_ref().map(|error| (error.code, error.message.as_str()));
    assert_eq!(
        (held.signal, error),
        (
            OperationWorkerSignal::WorkerFailed,
            Some((ErrorCode::CommandsIngressOverload, CHUNK_LOST))
        ),
        "the full ingress refused a memory bank chunk, yet the read ended {held:?} — the \
         registry holds a bank it never fully heard"
    );
}
