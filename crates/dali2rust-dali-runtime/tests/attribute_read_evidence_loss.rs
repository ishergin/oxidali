use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeGroup, DaliReadAttributesCommand, MemoryBankReadPreset,
    OperationWorkerSignal, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
    DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::{try_wait_until, wait_until};

const SHORT: u8 = 5;
const MAX_ATTEMPTS: u64 = 5;
const EXERCISE_DEADLINE: Duration = Duration::from_secs(20);
const SIGNAL_DEADLINE: Duration = Duration::from_secs(10);
const FLOODERS_PER_CORE: usize = 2;
const MIN_FLOODERS: usize = 4;

fn read_frame(correlation_id: u64) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
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

fn flood_events_ingress(
    publisher: dali2rust_bus::BusPublisher,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let ev = dali2rust_contracts::bus::event_envelope(
                SOURCE_ID_UNSPECIFIED,
                0,
                BusId::default().0,
                Some(Origin::Internal),
                dali2rust_contracts::msg::DaliEventPayload {
                    wire_address: 0xFE,
                    command: 0x00,
                    repeat_count: 1,
                },
            );
            let _ = publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
        }
    })
}

fn flooder_count() -> usize {
    std::thread::available_parallelism()
        .map_or(MIN_FLOODERS, |cores| cores.get() * FLOODERS_PER_CORE)
        .max(MIN_FLOODERS)
}

fn recv_terminal_signal(
    signals: &dali2rust_bus::BusSubscriberRx,
    correlation_id: u64,
) -> OperationWorkerSignal {
    let deadline = Instant::now() + SIGNAL_DEADLINE;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "no terminal signal for correlation {correlation_id} within {SIGNAL_DEADLINE:?} \
             after the flood stopped — the signal itself was lost, which is the \
             ADR-021 saturation limit, not this test's subject; rerun"
        );
        match signals.recv_timeout(remaining) {
            Ok(BusFrame::Event(envelope)) => {
                if let BusEventPayload::OperationWorkerSignalEvent(body) = &envelope.payload {
                    if body.workflow_correlation_id == correlation_id
                        && body.signal != OperationWorkerSignal::WorkerStarted
                    {
                        return body.signal;
                    }
                }
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("signal tap died")
            }
        }
    }
}

#[test]
fn a_lost_evidence_chunk_fails_the_read_instead_of_reporting_success() {
    let config = BusConfig {
        events_ingress: 4,
        ..BusConfig::default()
    };
    let (_host, publisher, (worker_cmd, signals)) = BusHost::spawn(config, |reg| {
        (
            reg.subscribe_commands(16, DALI_WORKER_HANDLED_COMMANDS),
            reg.subscribe_events(64, &["OperationWorkerSignalEvent"]),
        )
    });

    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    transport.lock().expect("mock").set_persistent_response(0x04);
    let counters = Arc::new(DaliWorkerCounters::default());
    let _worker = spawn_dali_worker(
        worker_cmd,
        DaliController::new(Arc::clone(&transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(&counters),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );

    let mut exercised = None;
    for attempt in 0..MAX_ATTEMPTS {
        let correlation_id = 4_000 + attempt;
        let lost_before = counters.evidence_publish_failed.load(Ordering::Relaxed);

        let stop = Arc::new(AtomicBool::new(false));
        let flooders: Vec<_> = (0..flooder_count())
            .map(|_| flood_events_ingress(publisher.clone(), Arc::clone(&stop)))
            .collect();
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, read_frame(correlation_id)),
            PublishResult::Queued,
            "the commands ingress is not what is being flooded"
        );
        let lost = try_wait_until(
            || counters.evidence_publish_failed.load(Ordering::Relaxed) > lost_before,
            EXERCISE_DEADLINE,
        );
        stop.store(true, Ordering::Relaxed);
        for flooder in flooders {
            flooder.join().expect("flooder thread");
        }
        if lost {
            exercised = Some(correlation_id);
            break;
        }
        wait_until(
            || counters.semantic_read_attributes_handled.load(Ordering::Relaxed) > attempt as u32,
            Duration::from_secs(10),
        );
    }

    let Some(correlation_id) = exercised else {
        panic!(
            "inconclusive: {MAX_ATTEMPTS} reads against a saturated events ingress and \
             evidence_publish_failed never moved — the mechanism was not reached, \
             so nothing here was tested (event_publish_failed = {})",
            counters.event_publish_failed.load(Ordering::Relaxed)
        );
    };
    let signal = recv_terminal_signal(&signals, correlation_id);
    assert_eq!(
        signal,
        OperationWorkerSignal::WorkerFailed,
        "correlation {correlation_id} lost its evidence to the flooded ingress \
         (evidence_publish_failed > 0) and still reported {signal:?} — a green \
         operation over a registry that never heard the read"
    );
}
