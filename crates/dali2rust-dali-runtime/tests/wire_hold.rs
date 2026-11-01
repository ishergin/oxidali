mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusSubscriberRx, CorrelationIdAllocator};
use dali2rust_contracts::msg::{
    BusCommandPayload, Dali103ArbitrationProbeCommand, Dali103CommissionCommand, Dali103PowerNotificationCommand, Dali103TestFrameCommand, Dali103ScanCommand, DaliBusHealthProbeCommand, DaliCommandPayload, DaliProgramGroupMembershipCommand, DaliProgramTarget, ErrorCode,
    DaliSetTargetStateCommand, DaliStopFadeCommand, DaliTargetScope, DaliWriteAttributesCommand, GroupMembershipAction,
    LightSetpoint, OperationWorkerSignal, Origin, PowerState,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_dali_runtime::runtime::executor::PROGRAM_VERIFY_REPAIRS;
use dali2rust_domain::dali::dev103::{Device103Address, Device103Command, Special103Command};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock, DALI_WORKER_HANDLED_COMMANDS,
};
use dali2rust_platform::dali::WireActivity;
use dali2rust_test_support::publish_queued;
use support::{next_signal, HeldAfter, ReadPortDouble, SentFrames, SIGNAL_DEADLINE};

const HELD_CORRELATION: u64 = 610;
const EARLIER_CORRELATION: u64 = 609;
const FRAMES_BEFORE_THE_HOLD: usize = 5;
const SCANNED_ADDRESSES: usize = 64;
const SHORT: u8 = 4;
const GROUP: u8 = 3;
const SEND_TWICE: usize = 2;
const GROUP_QUERIES: usize = 2;
const DRIVE_FRAMES: usize = SEND_TWICE + GROUP_QUERIES;
const DRIVES: usize = 1 + PROGRAM_VERIFY_REPAIRS as usize;
const NOT_IN_THE_GROUP: u8 = 0x00;
const GATE_QUERIES: usize = 2;
const START_QUIESCENT_SENT_TWICE: usize = 2;
const RAW_ADDRESS: u8 = 0xFF;
const RAW_OFF: u8 = 0x00;
const WORKER_INBOX: usize = 16;
const ANSWER_INBOX: usize = 64;
const FIRST_PROBE_QUERY: usize = 1;
const READ_BACK_QUERY: usize = 3;
const TESTFRAME_24_BIT_REPEATED_TWICE: u8 = 0b0001_0101;
const TESTFRAME: [u8; 3] = [0x07, 0xFE, 0x10];
const HELD_AT_THE_GATE: usize = 0;
const FRAMES_TO_THE_FIRST_SEARCH_BYTE: usize = 8;
const REST_OF_THE_PROBE: usize = 3;
const TARGET_LEVEL: u8 = 128;
const FADE_RATE: u8 = 7;

#[derive(Clone, Copy)]
enum Answer {
    Operation,
    Confirmation,
}

fn scan() -> BusCommandPayload {
    Dali103ScanCommand { registry_adapter_id: 0 }.into()
}

type Held = (Arc<Mutex<MockDaliTransport>>, Result<(), (ErrorCode, String)>, Arc<DaliWorkerCounters>);

fn scan_held_after(frames24: usize, passive: bool) -> Held {
    held_after(scan(), Answer::Operation, SentFrames::Of24(frames24), passive)
}

fn group_add_held_after(frames16: usize) -> Held {
    let add = DaliProgramGroupMembershipCommand {
        registry_adapter_id: 0,
        target: DaliProgramTarget::Short { short_address: SHORT },
        group_id: GROUP,
        action: GroupMembershipAction::Add,
    };
    held_after(add.into(), Answer::Confirmation, SentFrames::Of16(frames16), false)
}

fn group_add_held_at_query(query: usize) -> Held {
    held_after(group_add().into(), Answer::Confirmation, SentFrames::AtQuery(query), false)
}

fn group_add() -> DaliProgramGroupMembershipCommand {
    DaliProgramGroupMembershipCommand {
        registry_adapter_id: 0,
        target: DaliProgramTarget::Short { short_address: SHORT },
        group_id: GROUP,
        action: GroupMembershipAction::Add,
    }
}

fn quiescent_cause() -> Result<(), (ErrorCode, String)> {
    Err((ErrorCode::Conflict, "controller_quiescent".to_owned()))
}

fn held_after(
    command: BusCommandPayload,
    answer: Answer,
    sent: SentFrames,
    passive: bool,
) -> Held {
    let held = held_after_each(vec![command], answer, sent, passive);
    assert_eq!(held.2.commands_handled.load(Ordering::SeqCst), 1, "the command passed the gates");
    held
}

fn held_after_each(
    commands: Vec<BusCommandPayload>,
    answer: Answer,
    sent: SentFrames,
    passive: bool,
) -> Held {
    let (_host, publisher, (worker_commands, signals, confirmations)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(WORKER_INBOX, DALI_WORKER_HANDLED_COMMANDS),
            reg.subscribe_events(ANSWER_INBOX, &["OperationWorkerSignalEvent"]),
            reg.subscribe_confirmations(ANSWER_INBOX),
        )
    });
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    if matches!(answer, Answer::Confirmation) {
        transport.lock().expect("mock lock").set_persistent_response(NOT_IN_THE_GROUP);
    }
    let queries = AtomicUsize::new(0);
    let held_after = HeldAfter { transport: Arc::clone(&transport), sent, passive, queries };
    let counters = Arc::new(DaliWorkerCounters::default());
    spawn_dali_worker(
        worker_commands,
        DaliController::new(Arc::clone(&transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(ReadPortDouble { held_after: Some(held_after), ..ReadPortDouble::default() }),
        publisher.clone(),
        BusId::default(),
        Arc::clone(&counters),
        Arc::new(WireActivity::new()),
        Arc::new(CorrelationIdAllocator::new()),
    );
    let count = commands.len();
    for (at, command) in commands.into_iter().enumerate() {
        let correlation = if at + 1 == count { HELD_CORRELATION } else { EARLIER_CORRELATION };
        let command =
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation, BusId::default().0, Some(Origin::Api), command);
        publish_queued(&publisher, BusChannel::Commands, BusFrame::command(command));
    }
    let outcome = match answer {
        Answer::Operation => operation_outcome(&signals),
        Answer::Confirmation => Err(confirmed_failure(&confirmations)),
    };
    (transport, outcome, counters)
}

fn operation_outcome(signals: &BusSubscriberRx) -> Result<(), (ErrorCode, String)> {
    let terminal = std::iter::from_fn(|| next_signal(signals, HELD_CORRELATION))
        .find(|signal| signal.signal != OperationWorkerSignal::WorkerStarted)
        .expect("the command ends with an outcome");
    if terminal.signal == OperationWorkerSignal::WorkerSucceeded {
        return Ok(());
    }
    let error = terminal.error.expect("a failed command names its cause");
    Err((error.code, error.message.as_str().to_owned()))
}

fn confirmed_failure(confirmations: &BusSubscriberRx) -> (ErrorCode, String) {
    let deadline = Instant::now() + SIGNAL_DEADLINE;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).expect("the command is confirmed in time");
        if let Ok(BusFrame::Confirmation(confirmed)) = confirmations.recv_timeout(remaining) {
            if confirmed.meta.correlation_id == HELD_CORRELATION {
                let error = confirmed.confirmation.error.as_ref().expect("a refused command names its cause");
                return (error.code, error.message.as_str().to_owned());
            }
        }
    }
}

#[test]
fn another_controller_s_quiescent_mode_stops_a_scan_already_on_the_wire() {
    let (transport, cause, _) = scan_held_after(FRAMES_BEFORE_THE_HOLD, false);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_quiescent".to_owned())));
    let sent = transport.lock().expect("mock lock").sent_frames24().len();
    assert_eq!(sent, FRAMES_BEFORE_THE_HOLD, "no frame after the START, of {SCANNED_ADDRESSES} addresses");
}

#[test]
fn a_controller_made_passive_mid_scan_sends_nothing_more() {
    let (transport, cause, _) = scan_held_after(FRAMES_BEFORE_THE_HOLD, true);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_passive".to_owned())));
    assert_eq!(transport.lock().expect("mock lock").sent_frames24().len(), FRAMES_BEFORE_THE_HOLD);
}

#[test]
fn a_hold_between_a_group_add_and_its_read_back_is_reported_not_taken_for_silence() {
    let (transport, cause, _) = group_add_held_after(SEND_TWICE);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_quiescent".to_owned())));
    assert_eq!(transport.lock().expect("mock lock").sent_frames().len(), SEND_TWICE, "only the ADD went out");
}

#[test]
fn a_hold_before_a_repair_ends_the_programming_with_its_cause() {
    let (transport, cause, _) = group_add_held_after(DRIVE_FRAMES);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_quiescent".to_owned())));
    assert_eq!(transport.lock().expect("mock lock").sent_frames().len(), DRIVE_FRAMES, "no repair went out");
}

#[test]
fn a_hold_before_the_last_repair_s_read_back_is_reported_not_taken_for_silence() {
    let last_add = DRIVES * DRIVE_FRAMES - GROUP_QUERIES;
    let (transport, cause, _) = group_add_held_after(last_add);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_quiescent".to_owned())));
    assert_eq!(transport.lock().expect("mock lock").sent_frames().len(), last_add);
}

#[test]
fn a_raw_frame_held_after_the_gates_is_refused_with_its_cause() {
    let raw = DaliCommandPayload {
        wire_address: RAW_ADDRESS,
        command: RAW_OFF,
        repeat_count: 1,
        raw_mode: true,
        raw_expects_backward: false,
    };
    let (transport, cause, _) = held_after(raw.into(), Answer::Confirmation, SentFrames::AfterQueries(GATE_QUERIES), false);
    assert_eq!(cause, Err((ErrorCode::Conflict, "controller_quiescent".to_owned())));
    assert!(transport.lock().expect("mock lock").sent_frames().is_empty());
}

#[test]
fn a_bus_health_probe_held_between_its_queries_yields_and_is_not_a_failed_probe() {
    let probe = DaliBusHealthProbeCommand { registry_adapter_id: 0, query_power_failure: true };
    let (transport, cause, counters) =
        held_after(probe.into(), Answer::Confirmation, SentFrames::Of16(FIRST_PROBE_QUERY), false);
    assert_eq!(cause, Err((ErrorCode::Preempted, "preempted".to_owned())), "a hold cuts a probe as a yield does");
    assert_eq!(transport.lock().expect("mock lock").sent_frames().len(), FIRST_PROBE_QUERY);
    assert_eq!(counters.bus_health_probe_failed.load(Ordering::SeqCst), 0, "and is not a failed probe");
}

// IEC 62386-103 §9.9.3
#[test]
fn our_own_commissioning_made_passive_mid_run_still_ends_the_quiescence_it_began() {
    let commission = Dali103CommissionCommand { registry_adapter_id: 0, include_addressed: false };
    let passive_after_start = SentFrames::Of24(START_QUIESCENT_SENT_TWICE);
    let (transport, outcome, _) = held_after(commission.into(), Answer::Operation, passive_after_start, true);
    assert_eq!(outcome, Ok(()));
    let stop = Device103Command::StopQuiescentMode.frame(Device103Address::Broadcast).as_bytes();
    let sent = transport.lock().expect("mock lock").sent_frames24();
    assert_eq!(sent[sent.len() - 2..], [stop, stop], "the run ends with its STOP QUIESCENT MODE");
}

// IEC 62386-103 §9.9.3
#[test]
fn another_controller_s_quiescent_mode_stops_our_own_commissioning_at_its_next_unit() {
    let commission = Dali103CommissionCommand { registry_adapter_id: 0, include_addressed: false };
    let (transport, outcome, _) = held_after(commission.into(), Answer::Operation, SentFrames::Of24(1), false);
    assert_eq!(outcome, quiescent_cause());
    let start = Device103Command::StartQuiescentMode.frame(Device103Address::Broadcast).as_bytes();
    assert_eq!(
        transport.lock().expect("mock lock").sent_frames24(),
        vec![start; START_QUIESCENT_SENT_TWICE],
        "the started pair ends whole, and neither TERMINATE nor STOP QUIESCENT MODE follows"
    );
}

// IEC 62386-103 §9.9.3
#[test]
fn a_held_arbitration_probe_is_not_a_failed_probe() {
    let probe = Dali103ArbitrationProbeCommand { registry_adapter_id: 0 };
    let worker_done = DaliBusHealthProbeCommand { registry_adapter_id: 0, query_power_failure: false };
    let commands = vec![probe.into(), worker_done.into()];
    let (transport, _, counters) =
        held_after_each(commands, Answer::Confirmation, SentFrames::AfterQueries(GATE_QUERIES), false);
    assert_eq!(counters.commands_handled.load(Ordering::SeqCst), 1, "the probe passed the gates, the next did not");
    assert!(transport.lock().expect("mock lock").sent_frames24().is_empty());
    assert_eq!(counters.arbitration_probe_failed.load(Ordering::SeqCst), 0);
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0, "nor is the command the gate held back");
}

// IEC 62386-103 §9.9.3, §11.10.21
#[test]
fn a_frame_held_at_the_gate_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let test_frame =
        Dali103TestFrameCommand { registry_adapter_id: 0, data: TESTFRAME_24_BIT_REPEATED_TWICE, dtr: TESTFRAME };
    let held = SentFrames::AfterQueries(HELD_AT_THE_GATE);
    let (transport, cause, counters) = held_after_each(vec![test_frame.into()], Answer::Confirmation, held, false);
    assert_eq!(cause, quiescent_cause());
    assert_eq!(counters.commands_handled.load(Ordering::SeqCst), 0, "the gate held it back");
    assert!(transport.lock().expect("mock lock").sent_frames24().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

// IEC 62386-103 §9.9.1
#[test]
fn a_controller_made_passive_before_its_commissioning_s_first_frame_sends_nothing() {
    let commission = Dali103CommissionCommand { registry_adapter_id: 0, include_addressed: false };
    let passive_after_the_gate = SentFrames::AfterQueries(GATE_QUERIES);
    let (transport, outcome, _) = held_after(commission.into(), Answer::Operation, passive_after_the_gate, true);
    assert_eq!(outcome, Err((ErrorCode::Conflict, "controller_passive".to_owned())));
    assert!(transport.lock().expect("mock lock").sent_frames24().is_empty());
}

// IEC 62386-103 §9.9.3, §11.10.6
#[test]
fn a_hold_inside_a_compare_probe_lets_the_probe_finish() {
    let commission = Dali103CommissionCommand { registry_adapter_id: 0, include_addressed: false };
    let inside_the_probe = SentFrames::Of24(FRAMES_TO_THE_FIRST_SEARCH_BYTE);
    let (transport, outcome, _) = held_after(commission.into(), Answer::Operation, inside_the_probe, false);
    assert_eq!(outcome, quiescent_cause());
    let sent = transport.lock().expect("mock lock").sent_frames24();
    let compare = Special103Command::Compare.frame(0x00).as_bytes();
    assert_eq!(sent.len(), FRAMES_TO_THE_FIRST_SEARCH_BYTE + REST_OF_THE_PROBE);
    assert_eq!(sent.last(), Some(&compare), "the started probe ends with its COMPARE, and nothing follows");
}

#[test]
fn a_held_target_state_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let target = DaliSetTargetStateCommand {
        scope: DaliTargetScope::Short,
        virtual_lamp_id: 0,
        short_address: SHORT,
        group_id: 0,
        setpoint: LightSetpoint { power: PowerState::On, level: Some(TARGET_LEVEL), color: None },
        registry_adapter_id: 0,
        hold_hcl: false,
    };
    let held = SentFrames::AfterQueries(GATE_QUERIES);
    let (transport, cause, counters) = held_after(target.into(), Answer::Confirmation, held, false);
    assert_eq!(cause, quiescent_cause());
    assert!(transport.lock().expect("mock lock").sent_frames().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_held_stop_fade_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let stop = DaliStopFadeCommand {
        registry_adapter_id: 0,
        scope: DaliTargetScope::Short,
        virtual_lamp_id: 0,
        short_address: SHORT,
        group_id: 0,
    };
    let held = SentFrames::AfterQueries(GATE_QUERIES);
    let (transport, cause, counters) = held_after(stop.into(), Answer::Confirmation, held, false);
    assert_eq!(cause, quiescent_cause());
    assert!(transport.lock().expect("mock lock").sent_frames().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_held_attribute_write_ends_with_its_cause_and_is_not_an_execution_failure() {
    let write = DaliWriteAttributesCommand {
        short_address: SHORT,
        fade_time_ms: None,
        fade_rate: Some(FADE_RATE),
        power_on_level: None,
        system_failure_level: None,
        extended_fade_time_ms: None,
        registry_adapter_id: 0,
        tc_coolest_mirek: None,
        tc_warmest_mirek: None,
        min_level: None,
        max_level: None,
        dimming_curve: None, switching: Default::default(),
        signals_operation: true,
        known_device_types: None,
        refusal_fails: false,
    };
    let held = SentFrames::AfterQueries(GATE_QUERIES);
    let (transport, cause, counters) = held_after(write.into(), Answer::Operation, held, false);
    assert_eq!(cause, quiescent_cause());
    assert!(transport.lock().expect("mock lock").sent_frames().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

// IEC 62386-103 §9.9.3, §11.10.21
#[test]
fn a_held_test_frame_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let test_frame =
        Dali103TestFrameCommand { registry_adapter_id: 0, data: TESTFRAME_24_BIT_REPEATED_TWICE, dtr: TESTFRAME };
    let (transport, cause, counters) = held_after(test_frame.into(), Answer::Confirmation, SentFrames::Of24(1), false);
    assert_eq!(cause, quiescent_cause());
    assert_eq!(transport.lock().expect("mock lock").sent_frames24(), vec![TESTFRAME]);
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

// IEC 62386-103 §9.9.3, §9.12.2
#[test]
fn a_held_power_notification_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let notification = Dali103PowerNotificationCommand { registry_adapter_id: 0, short_address: None, device_groups: 0 };
    let (transport, cause, counters) =
        held_after(notification.into(), Answer::Confirmation, SentFrames::AfterQueries(GATE_QUERIES), false);
    assert_eq!(cause, quiescent_cause());
    assert!(transport.lock().expect("mock lock").sent_frames24().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_typed_frame_held_after_the_gates_is_refused_with_its_cause_and_is_not_an_execution_failure() {
    let typed = DaliCommandPayload {
        wire_address: RAW_ADDRESS,
        command: RAW_OFF,
        repeat_count: 1,
        raw_mode: false,
        raw_expects_backward: false,
    };
    let (transport, cause, counters) =
        held_after(typed.into(), Answer::Confirmation, SentFrames::AfterQueries(GATE_QUERIES), false);
    assert_eq!(cause, quiescent_cause());
    assert!(transport.lock().expect("mock lock").sent_frames().is_empty());
    assert_eq!(counters.execution_failed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_hold_on_the_read_back_alone_ends_the_programming_with_its_cause_rather_than_a_repair() {
    let (transport, cause, _) = group_add_held_at_query(READ_BACK_QUERY);
    assert_eq!(cause, quiescent_cause());
    assert_eq!(transport.lock().expect("mock lock").sent_frames().len(), SEND_TWICE, "no repair follows");
}
