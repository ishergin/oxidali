mod support;

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
    AttributeGroupReadOutcome, BusEventPayload, DaliAttributeGroup, DaliReadAttributesCommand,
    DaliReadMemoryBankCommand, ErrorCode, MemoryBankReadPreset, OperationWorkerSignal,
    OperationWorkerSignalEvent, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{DaliWorkerCounters, DALI_WORKER_HANDLED_COMMANDS};
use dali2rust_test_support::{
    hold_the_events_ingress_full, hold_the_events_ingress_with_room, try_wait_until,
    HeldEventsIngress,
};
use support::{
    assert_a_lost_fact_fails_the_operation, next_signal, spawn_signal_bus, spawn_worker_on,
    DemoBusOperation,
};

const SHORT: u8 = 5;
const GEAR_ANSWER: u8 = 0x04;
const READ_CORRELATION: u64 = 4_000;
const MEMORY_READ_CORRELATION: u64 = 4_001;
const CHUNK_LOST: &str = "memory_bank_chunk_lost";
const DEMO_SHORT: u8 = 0;
const BANK_ZERO_PREFIX: u16 = 3;
const STEP_DEADLINE: Duration = Duration::from_secs(10);
const SWEEP_CORRELATION: u64 = 4_002;
const SWEEP_EVENTS_INGRESS: usize = 4;
const SWEEP_WORKER_INBOX: usize = 16;
const SWEEP_TAP_INBOX: usize = 64;

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

fn start_the_read_on(
    publisher: &BusPublisher,
    signals: &BusSubscriberRx,
    frame: BusFrame,
    correlation_id: u64,
) {
    assert_eq!(publisher.try_publish(BusChannel::Commands, frame), PublishResult::Queued);
    assert_eq!(
        next_signal(signals, correlation_id).map(|started| started.signal),
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
    let (_host, publisher, worker_commands, signals) = spawn_signal_bus();
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    transport.lock().expect("mock").set_persistent_response(GEAR_ANSWER);
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker_on(worker_commands, &publisher, &transport, &counters);

    let wire_held = transport.lock().expect("mock");
    start_the_read_on(&publisher, &signals, read_frame(), READ_CORRELATION);
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
    let (_host, publisher, worker_commands, _signals) = spawn_signal_bus();
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_worker_on(worker_commands, &publisher, &transport, &counters);
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

#[test]
fn a_refused_memory_bank_chunk_fails_the_read() {
    assert_a_lost_fact_fails_the_operation(&DemoBusOperation {
        frame: memory_read_frame,
        correlation_id: MEMORY_READ_CORRELATION,
        announces_start: true,
        lost: CHUNK_LOST,
    });
}

struct SweepStack {
    _host: BusHost,
    publisher: BusPublisher,
    signals: BusSubscriberRx,
    reads: BusSubscriberRx,
    transport: Arc<Mutex<SimDaliTransport>>,
    counters: Arc<DaliWorkerCounters>,
    _worker: JoinHandle<()>,
}

struct SweepRun {
    terminal: OperationWorkerSignalEvent,
    evidence_chunks: usize,
    memory_banks: Option<AttributeGroupReadOutcome>,
    counters: Arc<DaliWorkerCounters>,
}

fn spawn_sweep_stack() -> SweepStack {
    let (host, publisher, (worker_commands, signals, reads)) = BusHost::spawn(
        BusConfig {
            events_ingress: SWEEP_EVENTS_INGRESS,
            ..BusConfig::default()
        },
        |reg| {
            (
                reg.subscribe_commands(SWEEP_WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
                reg.subscribe_events(SWEEP_TAP_INBOX, &["OperationWorkerSignalEvent"]),
                reg.subscribe_events(
                    SWEEP_TAP_INBOX,
                    &["DaliAttributesReadEvent", "DaliAttributeReadOutcomesEvent"],
                ),
            )
        },
    );
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let counters = Arc::new(DaliWorkerCounters::default());
    let worker = spawn_worker_on(worker_commands, &publisher, &transport, &counters);
    SweepStack { _host: host, publisher, signals, reads, transport, counters, _worker: worker }
}

fn sweep_read_frame() -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        SWEEP_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        DaliReadAttributesCommand {
            registry_adapter_id: 0,
            short_address: DEMO_SHORT,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            memory_banks: MemoryBankReadPreset::Identity,
        },
    ))
}

fn read_evidence(reads: &BusSubscriberRx) -> (usize, Option<AttributeGroupReadOutcome>) {
    let deadline = Instant::now() + STEP_DEADLINE;
    let mut chunks = 0;
    while let Some(remaining) = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
    {
        let envelope = match reads.recv_timeout(remaining) {
            Ok(BusFrame::Event(envelope)) => envelope,
            Ok(_) | Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => panic!("read tap died"),
        };
        if envelope.meta.correlation_id != SWEEP_CORRELATION {
            continue;
        }
        match &envelope.payload {
            BusEventPayload::DaliAttributesReadEvent(_) => chunks += 1,
            BusEventPayload::DaliAttributeReadOutcomesEvent(body) => {
                return (chunks, Some(body.memory_banks))
            }
            _ => {}
        }
    }
    (chunks, None)
}

fn release_after_a_refusal(stack: &SweepStack, ingress: HeldEventsIngress) {
    assert!(
        try_wait_until(
            || stack.counters.event_publish_failed.load(Ordering::Relaxed) > 0,
            STEP_DEADLINE
        ),
        "inconclusive: the events ingress was held with no room past the evidence, yet no \
         required publish gave up"
    );
    ingress.release();
}

fn sweep_the_banks(evidence_room: Option<usize>) -> SweepRun {
    let stack = spawn_sweep_stack();
    let wire_held = stack.transport.lock().expect("sim");
    start_the_read_on(&stack.publisher, &stack.signals, sweep_read_frame(), SWEEP_CORRELATION);
    let ingress = evidence_room.map(|room| {
        hold_the_events_ingress_with_room(&stack.publisher, SWEEP_EVENTS_INGRESS, room)
    });
    drop(wire_held);
    if let Some(ingress) = ingress {
        release_after_a_refusal(&stack, ingress);
    }
    let terminal = next_signal(&stack.signals, SWEEP_CORRELATION).expect("the read never ended");
    let (evidence_chunks, memory_banks) = read_evidence(&stack.reads);
    SweepRun { terminal, evidence_chunks, memory_banks, counters: Arc::clone(&stack.counters) }
}

#[test]
fn a_refused_chunk_of_the_bank_sweep_fails_the_attribute_read_and_its_bank_section() {
    let open = sweep_the_banks(None);
    assert_eq!(
        (open.terminal.signal, open.memory_banks),
        (OperationWorkerSignal::WorkerSucceeded, Some(AttributeGroupReadOutcome::Success)),
        "control: with the events ingress open the attribute read and its bank sweep succeed"
    );
    assert!(
        open.counters.memory_bank_chunk_events_published.load(Ordering::Relaxed) > 0,
        "control: the attribute read swept no memory bank, so nothing below exercises it"
    );
    let held = sweep_the_banks(Some(open.evidence_chunks));
    let error = held.terminal.error.as_ref().map(|error| (error.code, error.message.as_str()));
    assert_eq!(
        (held.terminal.signal, error, held.evidence_chunks),
        (
            OperationWorkerSignal::WorkerFailed,
            Some((ErrorCode::CommandsIngressOverload, CHUNK_LOST)),
            open.evidence_chunks
        ),
        "the attribute evidence had room and the first swept bank chunk had none, yet the \
         read ended {:?}",
        held.terminal
    );
    assert_eq!(
        held.memory_banks,
        Some(AttributeGroupReadOutcome::TransportAbort),
        "the sweep ran on the wire and its chunk was lost, so the bank section was attempted \
         and aborted, not left unattempted"
    );
    assert_eq!(
        (
            held.counters.evidence_publish_failed.load(Ordering::Relaxed),
            held.counters.execution_failed.load(Ordering::Relaxed)
        ),
        (1, 1),
        "one lost bank chunk counts as one lost fact, as on the standalone memory bank read"
    );
}
