mod support;

use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusFrame, BusId, CorrelationIdAllocator};
use dali2rust_contracts::msg::{Dali103ScanCommand, ErrorCode, OperationWorkerSignal, Origin};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::{spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock};
use dali2rust_platform::dali::WireActivity;
use dali2rust_test_support::publish_queued;
use support::{next_signal, spawn_signal_bus, HeldAfter, ReadPortDouble};

const SCAN_CORRELATION: u64 = 610;
const FRAMES_BEFORE_THE_HOLD: usize = 5;
const SCANNED_ADDRESSES: usize = 64;

fn scan_held_after(frames24: usize, passive: bool) -> (Arc<Mutex<MockDaliTransport>>, (ErrorCode, String)) {
    let (_host, publisher, worker_commands, signals) = spawn_signal_bus();
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let held_after = HeldAfter { transport: Arc::clone(&transport), frames24, passive };
    spawn_dali_worker(
        worker_commands,
        DaliController::new(Arc::clone(&transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(ReadPortDouble { held_after: Some(held_after), ..ReadPortDouble::default() }),
        publisher.clone(),
        BusId::default(),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(WireActivity::new()),
        Arc::new(CorrelationIdAllocator::new()),
    );
    let scan = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        SCAN_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        Dali103ScanCommand { registry_adapter_id: 0 },
    );
    publish_queued(&publisher, BusChannel::Commands, BusFrame::command(scan));
    let terminal = std::iter::from_fn(|| next_signal(&signals, SCAN_CORRELATION))
        .find(|signal| signal.signal != OperationWorkerSignal::WorkerStarted)
        .expect("the scan ends with an outcome");
    assert_eq!(terminal.signal, OperationWorkerSignal::WorkerFailed);
    let error = terminal.error.expect("a failed scan names its cause");
    (transport, (error.code, error.message.as_str().to_owned()))
}

#[test]
fn another_controller_s_quiescent_mode_stops_a_scan_already_on_the_wire() {
    let (transport, cause) = scan_held_after(FRAMES_BEFORE_THE_HOLD, false);
    assert_eq!(cause, (ErrorCode::Conflict, "controller_quiescent".to_owned()));
    let sent = transport.lock().expect("mock lock").sent_frames24().len();
    assert_eq!(sent, FRAMES_BEFORE_THE_HOLD, "no frame after the START, of {SCANNED_ADDRESSES} addresses");
}

#[test]
fn a_controller_made_passive_mid_scan_sends_nothing_more() {
    let (transport, cause) = scan_held_after(FRAMES_BEFORE_THE_HOLD, true);
    assert_eq!(cause, (ErrorCode::Conflict, "controller_passive".to_owned()));
    assert_eq!(transport.lock().expect("mock lock").sent_frames24().len(), FRAMES_BEFORE_THE_HOLD);
}
