use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx,
    PublishResult,
};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeGroup, DaliReadAttributesCommand, ErrorCode,
    MemoryBankReadPreset, OperationWorkerSignal, OperationWorkerSignalEvent, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{hold_the_events_ingress_full, try_wait_until};

const SHORT: u8 = 5;
const GEAR_ANSWER: u8 = 0x04;
const READ_CORRELATION: u64 = 4_000;
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

fn spawn_worker(
    commands: BusSubscriberRx,
    publisher: &BusPublisher,
    transport: &Arc<Mutex<MockDaliTransport>>,
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

fn next_signal(signals: &BusSubscriberRx) -> Option<OperationWorkerSignalEvent> {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())?;
        match signals.recv_timeout(remaining) {
            Ok(BusFrame::Event(envelope)) => match &envelope.payload {
                BusEventPayload::OperationWorkerSignalEvent(body)
                    if body.workflow_correlation_id == READ_CORRELATION =>
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
        next_signal(signals).map(|started| started.signal),
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
    let terminal = next_signal(signals)
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
