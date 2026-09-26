use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{
    DaliProgramTarget, ErrorCode, GroupMembershipAction, OperationStatus, OperationType,
    OperationWorkerSignal, SceneProgramAction,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::OperationReadPort;
use dali2rust_operations_runtime::{
    spawn_operation_tracker_worker, OperationTrackerCounters, OperationTrackerHttpRead,
    OperationTrackerInner, OPERATION_TRACKER_HANDLED_EVENTS,
};
use dali2rust_test_support::wait_until;

fn spawn_harness() -> (
    dali2rust_bus::BusPublisher,
    Arc<Mutex<OperationTrackerInner>>,
    BusHost,
) {
    let (publisher, tracker, _counters, host) = spawn_harness_with_counters();
    (publisher, tracker, host)
}

fn spawn_harness_with_counters() -> (
    dali2rust_bus::BusPublisher,
    Arc<Mutex<OperationTrackerInner>>,
    Arc<OperationTrackerCounters>,
    BusHost,
) {
    let (host, publisher, (cmd_rx, ev_rx, conf_rx)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_events(32, OPERATION_TRACKER_HANDLED_EVENTS),
                reg.subscribe_confirmations(16),
            )
        },
    );
    let tracker = Arc::new(Mutex::new(OperationTrackerInner::new()));
    let counters = Arc::new(OperationTrackerCounters::default());
    let _join = spawn_operation_tracker_worker(
        cmd_rx,
        ev_rx,
        conf_rx,
        publisher.clone(),
        BusId::default(),
        Arc::clone(&tracker),
        Arc::clone(&counters),
    );
    (publisher, tracker, counters, host)
}

fn publish(publisher: &dali2rust_bus::BusPublisher, channel: BusChannel, frame: BusFrame) {
    assert_eq!(publisher.try_publish(channel, frame), PublishResult::Queued);
}

const DEFAULT_FINISHED_RETENTION_MS: u32 = 60_000;

fn begin_envelope_full(
    correlation_id: u64,
    key: &str,
    op_type: OperationType,
    ttl_ms: u32,
    finished_retention_ms: u32,
    expected_outcomes: u16,
    target_adapter_id: u16,
) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, target_adapter_id, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::OperationBeginCommand { operation_key: dali2rust_contracts::msg::fixed_text_32(key), operation_type: op_type, ttl_ms, finished_retention_ms, expected_outcomes }))
}

fn begin_envelope(correlation_id: u64, key: &str, op_type: OperationType, ttl_ms: u32) -> BusFrame {
    begin_envelope_full(correlation_id, key, op_type, ttl_ms, DEFAULT_FINISHED_RETENTION_MS, 0, BusId::default().0)
}

fn begin_envelope_with_expected_outcomes(
    correlation_id: u64,
    key: &str,
    op_type: OperationType,
    ttl_ms: u32,
    expected_outcomes: u16,
) -> BusFrame {
    begin_envelope_full(correlation_id, key, op_type, ttl_ms, DEFAULT_FINISHED_RETENTION_MS, expected_outcomes, BusId::default().0)
}

fn signal_event(correlation_id: u64, signal: OperationWorkerSignal, code: ErrorCode) -> BusFrame {
    signal_event_with_origin(
        correlation_id,
        signal,
        code,
        dali2rust_contracts::msg::Origin::Internal,
    )
}

fn signal_event_with_origin(
    correlation_id: u64,
    signal: OperationWorkerSignal,
    code: ErrorCode,
    origin: dali2rust_contracts::msg::Origin,
) -> BusFrame {
    use dali2rust_contracts::msg::OperationWorkerSignalEvent as Sig;
    let body = match signal {
        OperationWorkerSignal::WorkerStarted => Sig::started(correlation_id),
        OperationWorkerSignal::WorkerSucceeded => Sig::succeeded(correlation_id),
        OperationWorkerSignal::WorkerFailed => Sig::failed(correlation_id, code, ""),
    };
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(origin),
        body,
    ))
}

fn poller_signal_event(correlation_id: u64, signal: OperationWorkerSignal, code: ErrorCode) -> BusFrame {
    signal_event_with_origin(
        correlation_id,
        signal,
        code,
        dali2rust_contracts::msg::Origin::Poller,
    )
}

fn tail_for(tracker: &Arc<Mutex<OperationTrackerInner>>, key: &str) -> Vec<OperationStatus> {
    tracker
        .lock()
        .expect("tracker")
        .events
        .iter()
        .filter(|e| e.operation_key == key)
        .map(|e| e.status)
        .collect::<Vec<_>>()
}

fn last_error_for(tracker: &Arc<Mutex<OperationTrackerInner>>, key: &str) -> Option<ErrorCode> {
    tracker
        .lock()
        .expect("tracker")
        .events
        .iter()
        .rev()
        .find(|e| e.operation_key == key)
        .and_then(|e| e.error_code)
}

#[test]
fn operation_accepted_then_running_op001() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9001, "op-lifecycle", OperationType::Discovery, 600_000),
    );
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(9001, OperationWorkerSignal::WorkerStarted, ErrorCode::InvalidJson),
    );
    wait_until(
        || tail_for(&tracker, "op-lifecycle") == vec![OperationStatus::Accepted, OperationStatus::Running],
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-lifecycle"), vec![OperationStatus::Accepted, OperationStatus::Running]);
}

#[test]
fn operation_failed_with_worker_error_op002() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9002, "op-failed", OperationType::MemoryBankRead, 600_000),
    );
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(9002, OperationWorkerSignal::WorkerFailed, ErrorCode::OperationFailed),
    );
    wait_until(
        || tail_for(&tracker, "op-failed").last().copied() == Some(OperationStatus::Failed),
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-failed").last().copied(), Some(OperationStatus::Failed));
    assert_eq!(last_error_for(&tracker, "op-failed"), Some(ErrorCode::OperationFailed));
}

#[test]
fn a_rejected_delivery_fails_the_operation_instead_of_waiting_out_the_ttl() {
    let (publisher, tracker, _host) = spawn_harness();
    let workflow = 9101;
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(workflow, "op-rejected", OperationType::AttributeWrite, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-rejected") == vec![OperationStatus::Accepted],
        Duration::from_millis(500),
    );

    let semantic = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::DaliWriteAttributesCommand {
            short_address: 3,
            fade_time_ms: Some(400),
            fade_rate: None,
            power_on_level: None,
            system_failure_level: None,
            extended_fade_time_ms: None,
            registry_adapter_id: 0,
            tc_coolest_mirek: None,
            tc_warmest_mirek: None,
            min_level: None,
            max_level: None,
            dimming_curve: None,
            signals_operation: true,
        },
    );
    let rejection = dali2rust_contracts::bus::synthetic_delivery_rejected_envelope(&semantic)
        .expect("the bus builds one for every rejected command");
    publish(
        &publisher,
        BusChannel::Confirmations,
        BusFrame::Confirmation(std::sync::Arc::new(rejection)),
    );

    wait_until(
        || tail_for(&tracker, "op-rejected").last().copied() == Some(OperationStatus::Failed),
        Duration::from_millis(500),
    );
    assert_eq!(
        tail_for(&tracker, "op-rejected").last().copied(),
        Some(OperationStatus::Failed),
        "an undelivered command must not leave the operation accepted for its TTL"
    );
    assert_eq!(
        last_error_for(&tracker, "op-rejected"),
        Some(ErrorCode::CommandsIngressOverload)
    );
}

#[test]
fn operation_times_out_without_worker_signals_op003() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9003, "op-timeout", OperationType::Discovery, 80),
    );
    wait_until(
        || tail_for(&tracker, "op-timeout").last().copied() == Some(OperationStatus::TimedOut),
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-timeout").last().copied(), Some(OperationStatus::TimedOut));
    assert_eq!(
        last_error_for(&tracker, "op-timeout"),
        Some(ErrorCode::ConfirmationTimeout)
    );
}

#[test]
fn second_begin_supersedes_prior_active_operation_op004() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9100, "op-a", OperationType::GroupApply, 600_000),
    );
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9101, "op-b", OperationType::GroupApply, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-a").last().copied() == Some(OperationStatus::Cancelled),
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-a").last().copied(), Some(OperationStatus::Cancelled));
    assert_eq!(last_error_for(&tracker, "op-a"), Some(ErrorCode::Superseded));
}

#[test]
fn registry_reset_cancels_active_operations_op005() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9200, "op-reset", OperationType::Discovery, 600_000),
    );
    let reset = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 77_777, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::OperationRegistryResetCommand {});
    publish(&publisher, BusChannel::Commands, BusFrame::command(reset));
    wait_until(
        || tail_for(&tracker, "op-reset").last().copied() == Some(OperationStatus::Cancelled),
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-reset").last().copied(), Some(OperationStatus::Cancelled));
    assert_eq!(last_error_for(&tracker, "op-reset"), Some(ErrorCode::RegistryReset));
}

#[test]
fn terminal_operation_is_evicted_after_retention_op006() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_full(9600, "op-evict", OperationType::Discovery, 600_000, 40, 0, BusId::default().0),
    );
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(9600, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
    );
    wait_until(
        || tail_for(&tracker, "op-evict").last().copied() == Some(OperationStatus::Succeeded),
        Duration::from_millis(500),
    );
    wait_until(
        || read.operation_view("op-evict").is_none() && read.list_operation_keys().is_empty(),
        Duration::from_millis(1_000),
    );
    assert!(read.operation_view("op-evict").is_none());
    assert!(read.list_operation_keys().is_empty());
    assert_eq!(tail_for(&tracker, "op-evict").last().copied(), Some(OperationStatus::Succeeded));
}

#[test]
fn begin_for_another_adapter_does_not_supersede_op141() {
    let (publisher, tracker, _host) = spawn_harness();
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9700, "op-a", OperationType::GroupApply, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-a").last().copied() == Some(OperationStatus::Accepted),
        Duration::from_millis(500),
    );
    let other_adapter = BusId::default().0 + 1;
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_full(9701, "op-b", OperationType::GroupApply, 600_000, DEFAULT_FINISHED_RETENTION_MS, 0, other_adapter),
    );
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9702, "op-c", OperationType::Discovery, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-c").last().copied() == Some(OperationStatus::Accepted),
        Duration::from_millis(500),
    );
    assert_eq!(tail_for(&tracker, "op-a"), vec![OperationStatus::Accepted]);
    assert!(tail_for(&tracker, "op-b").is_empty(), "foreign-adapter begin must be filtered out");
    assert_eq!(last_error_for(&tracker, "op-a"), None);
}

#[test]
fn group_apply_outcomes_aggregate_into_operation_view() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9300,
            "grp-apply-0-9300",
            OperationType::GroupApply,
            600_000,
            3,
        ),
    );
    publish(
        &publisher,
        BusChannel::Events,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 9300, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 1 }, group_id: 2, action: GroupMembershipAction::Add, physical_short_address: Some(7), membership: Some(1 << 2), error: None })),
    );
    publish(
        &publisher,
        BusChannel::Events,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 9300, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 2 }, group_id: 5, action: GroupMembershipAction::Remove, physical_short_address: None, membership: None, error: (Some((ErrorCode::VlUnbound, "vl_unbound"))).map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)) })),
    );
    publish(
        &publisher,
        BusChannel::Events,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 9300, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 3 }, group_id: 6, action: GroupMembershipAction::Add, physical_short_address: Some(9), membership: None, error: (Some((ErrorCode::OperationFailed, "dali_transport_error"))).map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)) })),
    );

    wait_until(
        || {
            read.operation_view("grp-apply-0-9300")
                .map(|view| view.status == "failed")
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );

    let view = read
        .operation_view("grp-apply-0-9300")
        .expect("operation view");
    assert_eq!(view.operation_id, "grp-apply-0-9300");
    assert_eq!(view.operation_type, "group_apply");
    assert_eq!(view.status, "failed");
    assert_eq!(view.error.as_ref().map(|e| e.code.as_ref()), Some("operation_failed"));
    assert_eq!(
        view.error.as_ref().map(|e| e.message.as_str()),
        Some("dali_transport_error")
    );
    let result = view.result.expect("group apply result");
    let result = result.as_group_apply().expect("group apply variant").clone();
    assert_eq!(result.programmed.len(), 1);
    assert_eq!(result.programmed[0].virtual_lamp_id, 1);
    assert_eq!(result.programmed[0].group_id, 2);
    assert_eq!(result.programmed[0].action, "add");
    assert_eq!(result.programmed[0].physical_short_address, Some(7));
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].virtual_lamp_id, 2);
    assert_eq!(result.skipped[0].reason.as_deref(), Some("vl_unbound"));
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].virtual_lamp_id, 3);
    assert_eq!(result.failed[0].reason.as_deref(), Some("dali_transport_error"));
    assert!(!read.has_active_operation(OperationType::GroupApply, 0));
}

#[test]
fn group_apply_succeeds_when_all_programmed() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9400,
            "grp-apply-0-9400",
            OperationType::GroupApply,
            600_000,
            3,
        ),
    );
    for (vl, group, action, short) in [
        (1u8, 2u8, GroupMembershipAction::Add, 1u8),
        (1u8, 7u8, GroupMembershipAction::Remove, 1u8),
        (2u8, 1u8, GroupMembershipAction::Add, 2u8),
    ] {
        publish(
            &publisher,
            BusChannel::Events,
            BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 9400, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: vl }, group_id: group, action, physical_short_address: Some(short), membership: None, error: None })),
        );
    }
    wait_until(
        || {
            read.operation_view("grp-apply-0-9400")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let view = read
        .operation_view("grp-apply-0-9400")
        .expect("operation view");
    assert_eq!(view.status, "succeeded");
    assert!(!read.has_active_operation(OperationType::GroupApply, 0));
}

#[test]
fn group_apply_buffers_outcome_until_begin() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Events,
        BusFrame::event(dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, 9500, BusId::default().0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id: 0, target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: 3 }, group_id: 2, action: GroupMembershipAction::Add, physical_short_address: None, membership: None, error: (Some((ErrorCode::VlUnbound, "vl_unbound"))).map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)) })),
    );
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9500,
            "grp-apply-0-9500",
            OperationType::GroupApply,
            600_000,
            1,
        ),
    );
    wait_until(
        || {
            read.operation_view("grp-apply-0-9500")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let view = read
        .operation_view("grp-apply-0-9500")
        .expect("operation view");
    assert_eq!(view.status, "succeeded");
    let result = view.result.expect("group apply result");
    let result = result.as_group_apply().expect("group apply variant").clone();
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].virtual_lamp_id, 3);
}

#[test]
fn worker_signals_before_begin_are_replayed_after_registration() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(9600, OperationWorkerSignal::WorkerStarted, ErrorCode::InvalidJson),
    );
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(9600, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
    );
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 1,
        Duration::from_millis(500),
    );
    assert!(tail_for(&tracker, "op-early-signal").is_empty());

    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(9600, "op-early-signal", OperationType::AttributeRead, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-early-signal").last().copied() == Some(OperationStatus::Succeeded),
        Duration::from_millis(500),
    );
    assert_eq!(
        tail_for(&tracker, "op-early-signal"),
        vec![
            OperationStatus::Accepted,
            OperationStatus::Running,
            OperationStatus::Succeeded
        ]
    );
    assert_eq!(
        read.operation_view("op-early-signal").map(|v| v.status.to_string()),
        Some("succeeded".to_string())
    );
    assert_eq!(
        tracker.lock().expect("tracker").pending_worker_signal_correlations(),
        0
    );
}

#[test]
fn group_apply_detail_rows_are_capped_but_totals_stay_exact() {
    const EXPECTED: u16 = 200;
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9700,
            "grp-apply-0-9700",
            OperationType::GroupApply,
            600_000,
            EXPECTED,
        ),
    );
    for i in 0..EXPECTED {
        let vl = (i % 64) as u8;
        let group_id = (i / 64) as u8;
        let frame = BusFrame::event(dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            9700,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent {
                registry_adapter_id: 0,
                target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: vl },
                group_id,
                action: GroupMembershipAction::Add,
                physical_short_address: Some(vl),
                membership: Some(1),
                error: None,
            },
        ));
        wait_until(
            || publisher.try_publish(BusChannel::Events, frame.clone()) == PublishResult::Queued,
            Duration::from_secs(1),
        );
        let expected_counted = i + 1;
        wait_until(
            || {
                read.operation_view("grp-apply-0-9700")
                    .and_then(|view| view.result)
                    .and_then(|r| r.as_group_apply().map(|g| g.programmed_total))
                    .map(|total| total == expected_counted)
                    .unwrap_or(false)
            },
            Duration::from_secs(1),
        );
    }
    wait_until(
        || {
            read.operation_view("grp-apply-0-9700")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_secs(2),
    );
    let view = read
        .operation_view("grp-apply-0-9700")
        .expect("operation view");
    assert_eq!(view.status, "succeeded");
    let result = view.result.expect("group apply result");
    let result = result.as_group_apply().expect("group apply variant").clone();
    assert_eq!(result.programmed.len(), 128, "detail rows capped");
    assert_eq!(result.programmed_total, EXPECTED, "totals stay exact");
    assert_eq!(result.failed_total, 0);
}

#[test]
fn duplicate_cell_outcome_is_counted_once() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9800,
            "grp-apply-0-9800",
            OperationType::GroupApply,
            600_000,
            2,
        ),
    );
    let cell_outcome = |vl: u8, group: u8| {
        BusFrame::event(dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            9800,
            BusId::default().0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent {
                registry_adapter_id: 0,
                target: DaliProgramTarget::VirtualLamp { virtual_lamp_id: vl },
                group_id: group,
                action: GroupMembershipAction::Add,
                physical_short_address: Some(vl),
                membership: Some(1 << group),
                error: None,
            },
        ))
    };
    publish(&publisher, BusChannel::Events, cell_outcome(1, 2));
    publish(&publisher, BusChannel::Events, cell_outcome(1, 2));
    wait_until(
        || {
            read.operation_view("grp-apply-0-9800")
                .and_then(|view| view.result)
                .and_then(|r| r.as_group_apply().map(|g| g.programmed_total))
                .map(|total| total == 1)
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let view = read
        .operation_view("grp-apply-0-9800")
        .expect("operation view");
    assert_eq!(view.status, "running", "duplicate must not finish the run");
    assert_eq!(
        view.result
            .expect("result")
            .as_group_apply()
            .expect("group apply variant")
            .programmed_total,
        1
    );

    publish(&publisher, BusChannel::Events, cell_outcome(2, 2));
    wait_until(
        || {
            read.operation_view("grp-apply-0-9800")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let result = read
        .operation_view("grp-apply-0-9800")
        .expect("operation view")
        .result
        .expect("group apply result");
    let result = result.as_group_apply().expect("group apply variant");
    assert_eq!(result.programmed_total, 2);
    assert_eq!(result.programmed.len(), 2);
}

fn scene_row_outcome(
    correlation_id: u64,
    virtual_lamp_id: u8,
    action: SceneProgramAction,
    error: Option<(ErrorCode, &str)>,
) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliSceneProgrammedEvent {
            registry_adapter_id: 0,
            target: DaliProgramTarget::VirtualLamp { virtual_lamp_id },
            scene_id: 3,
            action,
            physical_short_address: error.is_none().then_some(7),
            target_state: None,
            scene_level: None,
            error: error
                .map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)),
        },
    ))
}

#[test]
fn scene_apply_buckets_written_updated_cleared_skipped_and_failed() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope_with_expected_outcomes(
            9900,
            "scn-apply-0-3-9900",
            OperationType::SceneApply,
            600_000,
            5,
        ),
    );
    publish(&publisher, BusChannel::Events, scene_row_outcome(9900, 1, SceneProgramAction::Write, None));
    publish(&publisher, BusChannel::Events, scene_row_outcome(9900, 2, SceneProgramAction::Update, None));
    publish(&publisher, BusChannel::Events, scene_row_outcome(9900, 3, SceneProgramAction::Clear, None));
    publish(
        &publisher,
        BusChannel::Events,
        scene_row_outcome(9900, 9, SceneProgramAction::Write, Some((ErrorCode::VlUnbound, "vl_unbound"))),
    );
    publish(&publisher, BusChannel::Events, scene_row_outcome(9900, 1, SceneProgramAction::Write, None));
    publish(
        &publisher,
        BusChannel::Events,
        scene_row_outcome(9900, 4, SceneProgramAction::Update, Some((ErrorCode::OperationFailed, "dali_transport_error"))),
    );
    wait_until(
        || {
            read.operation_view("scn-apply-0-3-9900")
                .map(|view| view.status == "failed")
                .unwrap_or(false)
        },
        Duration::from_millis(500),
    );
    let view = read
        .operation_view("scn-apply-0-3-9900")
        .expect("operation view");
    assert_eq!(view.operation_type, "scene_apply");
    let result = view.result.expect("scene apply result");
    let result = result.as_scene_apply().expect("scene apply variant");
    assert_eq!(result.written_total, 1);
    assert_eq!(result.updated_total, 1);
    assert_eq!(result.cleared_total, 1);
    assert_eq!(result.skipped_total, 1);
    assert_eq!(result.failed_total, 1);
    assert_eq!(result.written[0].virtual_lamp_id, 1);
    assert_eq!(result.skipped[0].reason.as_deref(), Some("vl_unbound"));
    assert_eq!(result.failed[0].virtual_lamp_id, 4);
    assert!(!read.has_active_operation(OperationType::SceneApply, 0));
}

#[test]
fn group_apply_result_json_is_byte_identical_through_the_untagged_wrapper() {
    use dali2rust_domain::registry::{
        OperationApplyResultView, OperationGroupApplyOutcomeView, OperationGroupApplyResultView,
        OperationView,
    };
    let plain = OperationGroupApplyResultView {
        programmed: vec![OperationGroupApplyOutcomeView {
            virtual_lamp_id: 1,
            group_id: 2,
            action: "add".to_string(),
            physical_short_address: Some(7),
            reason: None,
        }],
        skipped: Vec::new(),
        failed: Vec::new(),
        programmed_total: 1,
        skipped_total: 0,
        failed_total: 0,
    };
    let view = OperationView {
        operation_id: "grp-apply-0-1".to_string(),
        operation_type: "group_apply".into(),
        status: "succeeded".into(),
        error: None,
        result: Some(OperationApplyResultView::GroupApply(plain.clone())),
        attribute_read_outcomes: None,
    };
    let wrapped = serde_json::to_value(&view).expect("serialize view");
    let direct = serde_json::to_value(&plain).expect("serialize plain result");
    assert_eq!(wrapped["result"], direct, "untagged wrapper must be invisible on the wire");
    let back: OperationView = serde_json::from_value(wrapped).expect("deserialize view");
    assert_eq!(
        back.result.expect("result").as_group_apply(),
        Some(&plain)
    );
}

#[test]
fn poller_origin_signals_do_not_starve_a_racing_real_operation() {
    let (publisher, tracker, _host) = spawn_harness();
    for corr in 20_200..20_220 {
        publish(
            &publisher,
            BusChannel::Events,
            poller_signal_event(corr, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
        );
    }
    publish(
        &publisher,
        BusChannel::Events,
        signal_event(20_999, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
    );
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 1,
        Duration::from_millis(500),
    );

    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(20_999, "op-after-poller-flood", OperationType::AttributeRead, 600_000),
    );
    wait_until(
        || {
            tail_for(&tracker, "op-after-poller-flood").last().copied()
                == Some(OperationStatus::Succeeded)
        },
        Duration::from_millis(500),
    );
    assert_eq!(
        tail_for(&tracker, "op-after-poller-flood"),
        vec![OperationStatus::Accepted, OperationStatus::Succeeded]
    );
}

#[test]
fn commissioning_active_is_visible_for_its_registry_adapter() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    for (corr, key, op_type) in [
        (9700, "comm-ident-0-5-9700", OperationType::CommissioningIdentify),
        (9710, "comm-addr-0-5-9-9710", OperationType::CommissioningAddressChange),
        (9720, "comm-repl-0-5-9-9720", OperationType::CommissioningReplaceDevice),
    ] {
        publish(
            &publisher,
            BusChannel::Commands,
            begin_envelope(corr, key, op_type, 600_000),
        );
        wait_until(
            || tail_for(&tracker, key).last().copied() == Some(OperationStatus::Accepted),
            Duration::from_millis(500),
        );
        assert!(
            read.has_active_operation(op_type, 0),
            "active {key} must report busy for registry adapter 0"
        );
    }
}

fn identified_event(correlation_id: u64, key: &str, short_address: u8) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDeviceIdentifiedEvent {
            registry_adapter_id: 0,
            short_address,
            mechanism: dali2rust_contracts::msg::IdentifyMechanism::IdentifyDevice,
            operation_key: dali2rust_contracts::msg::fixed_text_32(key),
            error: None,
        },
    ))
}

fn addressing_completed_event(correlation_id: u64, key: &str, old: u8, new: u8) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliAddressingCompletedEvent {
            registry_adapter_id: 0,
            old_short_address: old,
            new_short_address: new,
            operation_key: dali2rust_contracts::msg::fixed_text_32(key),
            error: None,
        },
    ))
}

fn device_replaced_event(correlation_id: u64, key: &str, failed: u8, replacement: u8) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        BusId::default().0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDeviceReplacedEvent {
            registry_adapter_id: 0,
            failed_short_address: failed,
            replacement_short_address: replacement,
            restored_metadata_and_overrides: true,
            restored_attributes: true,
            restored_groups: false,
            restored_scenes: true,
            operation_key: dali2rust_contracts::msg::fixed_text_32(key),
            error: None,
        },
    ))
}

fn drive_raced_commissioning_op(
    publisher: &dali2rust_bus::BusPublisher,
    tracker: &Arc<Mutex<OperationTrackerInner>>,
    corr: u64,
    key: &str,
    op_type: OperationType,
    outcome: BusFrame,
) -> dali2rust_domain::registry::OperationView {
    let read = OperationTrackerHttpRead(Arc::clone(tracker));
    publish(publisher, BusChannel::Events, outcome);
    publish(
        publisher,
        BusChannel::Events,
        signal_event(corr, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
    );
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 1,
        Duration::from_millis(500),
    );
    publish(publisher, BusChannel::Commands, begin_envelope(corr, key, op_type, 600_000));
    wait_until(
        || tail_for(tracker, key).last().copied() == Some(OperationStatus::Succeeded),
        Duration::from_millis(500),
    );
    read.operation_view(key).expect("operation view")
}

#[test]
fn raced_identify_outcome_is_attached_after_begin_m10() {
    let (publisher, tracker, _host) = spawn_harness();
    let key = "comm-ident-0-5-30000";
    let view = drive_raced_commissioning_op(
        &publisher,
        &tracker,
        30_000,
        key,
        OperationType::CommissioningIdentify,
        identified_event(30_000, key, 5),
    );
    assert_eq!(view.status, "succeeded");
    let result = view
        .result
        .expect("raced identify outcome must be attached to the operation (M10)");
    let identify = result.as_identify().expect("identify result variant");
    assert_eq!(identify.short_address, 5);
    assert_eq!(identify.identify_mechanism, "identify_device");
}

#[test]
fn raced_address_change_outcome_is_attached_after_begin_m10() {
    let (publisher, tracker, _host) = spawn_harness();
    let key = "comm-addr-0-5-9-30100";
    let view = drive_raced_commissioning_op(
        &publisher,
        &tracker,
        30_100,
        key,
        OperationType::CommissioningAddressChange,
        addressing_completed_event(30_100, key, 5, 9),
    );
    assert_eq!(view.status, "succeeded");
    let result = view
        .result
        .expect("raced address-change outcome must be attached to the operation (M10)");
    let change = result.as_address_change().expect("address change result variant");
    assert_eq!(change.old_short_address, 5);
    assert_eq!(change.new_short_address, 9);
}

#[test]
fn raced_replace_device_outcome_is_attached_after_begin_m10() {
    let (publisher, tracker, _host) = spawn_harness();
    let key = "comm-repl-0-5-9-30200";
    let view = drive_raced_commissioning_op(
        &publisher,
        &tracker,
        30_200,
        key,
        OperationType::CommissioningReplaceDevice,
        device_replaced_event(30_200, key, 5, 9),
    );
    assert_eq!(view.status, "succeeded");
    let result = view
        .result
        .expect("raced replace-device outcome must be attached to the operation (M10)");
    let replace = result.as_replace_device().expect("replace device result variant");
    assert_eq!(replace.failed_short_address, 5);
    assert_eq!(replace.replacement_short_address, 9);
    assert!(replace.restored.metadata_and_overrides);
    assert!(!replace.restored.groups);
}

#[test]
fn expired_pending_entries_age_out_and_free_the_cap_m10() {
    let (publisher, tracker, counters, _host) = spawn_harness_with_counters();
    for corr in 31_000..31_016u64 {
        publish(
            &publisher,
            BusChannel::Events,
            signal_event(corr, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
        );
    }
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 16,
        Duration::from_millis(500),
    );
    tracker.lock().expect("tracker").expire_pending_entries();
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 0,
        Duration::from_millis(500),
    );
    assert_eq!(
        tracker.lock().expect("tracker").pending_worker_signal_correlations(),
        0,
        "pending entries older than the TTL must be evicted (M10)"
    );
    assert!(
        counters
            .pending_outcomes_expired
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 16,
        "eviction must be visible in pending_outcomes_expired (M10)"
    );

    publish(
        &publisher,
        BusChannel::Events,
        signal_event(31_999, OperationWorkerSignal::WorkerSucceeded, ErrorCode::InvalidJson),
    );
    wait_until(
        || tracker.lock().expect("tracker").pending_worker_signal_correlations() == 1,
        Duration::from_millis(500),
    );
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(31_999, "op-after-expiry", OperationType::AttributeRead, 600_000),
    );
    wait_until(
        || tail_for(&tracker, "op-after-expiry").last().copied() == Some(OperationStatus::Succeeded),
        Duration::from_millis(500),
    );
    assert_eq!(
        tail_for(&tracker, "op-after-expiry"),
        vec![OperationStatus::Accepted, OperationStatus::Succeeded],
        "a raced outcome after eviction must land and complete the operation (M10)"
    );
}

#[test]
fn commissioning_active_does_not_block_another_registry_adapter() {
    let (publisher, tracker, _host) = spawn_harness();
    let read = OperationTrackerHttpRead(Arc::clone(&tracker));
    publish(
        &publisher,
        BusChannel::Commands,
        begin_envelope(
            9730,
            "comm-ident-0-5-9730",
            OperationType::CommissioningIdentify,
            600_000,
        ),
    );
    wait_until(
        || tail_for(&tracker, "comm-ident-0-5-9730").last().copied()
            == Some(OperationStatus::Accepted),
        Duration::from_millis(500),
    );
    assert!(
        !read.has_active_operation(OperationType::CommissioningIdentify, 1),
        "an identify on registry adapter 0 must not 409 registry adapter 1"
    );
}
