use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx, PublishResult,
};
use dali2rust_contracts::msg::{
    BusCommandPayload, DaliProgramTarget, GroupApplyExecuteCommand, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    AdapterReadPort, AdapterView, GroupApplyRowView, GroupApplySnapshot,
    GroupMembershipMatrixView, GroupReadPort, GroupView, OperationReadPort, SceneApplySnapshot,
    SceneMatrixView, SceneReadPort, SceneView,
};
use dali2rust_operations_runtime::{
    spawn_apply_orchestrator_worker, spawn_operation_tracker_worker, ApplyOrchestratorCounters,
    OperationTrackerCounters, OperationTrackerHttpRead, OperationTrackerInner,
    APPLY_ORCHESTRATOR_HANDLED_COMMANDS, APPLY_ORCHESTRATOR_HANDLED_EVENTS,
    OPERATION_TRACKER_HANDLED_COMMANDS, OPERATION_TRACKER_HANDLED_EVENTS,
};
use dali2rust_test_support::wait_until;

struct StubGroupReadPort {
    rows: Vec<GroupApplyRowView>,
    policy_cells: Vec<dali2rust_domain::registry::PolicyApplyCell>,
}

impl AdapterReadPort for StubGroupReadPort {
    fn adapter_count(&self) -> u8 {
        1
    }
    fn adapter_view(&self, _adapter_id: u8) -> Option<AdapterView> {
        None
    }
    fn list_adapter_views(&self) -> Vec<AdapterView> {
        Vec::new()
    }
}

impl GroupReadPort for StubGroupReadPort {
    fn group_view(&self, _adapter_id: u8, _group_id: u8) -> Option<GroupView> {
        None
    }
    fn list_group_views(&self, _adapter_id: u8) -> Vec<GroupView> {
        Vec::new()
    }
    fn group_membership_matrix_view(&self, _adapter_id: u8) -> Option<GroupMembershipMatrixView> {
        None
    }
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        (adapter_id == 0).then(|| GroupApplySnapshot {
            adapter_id,
            rows: self.rows.clone(),
        })
    }

}

impl dali2rust_domain::registry::PolicyApplyReadPort for StubGroupReadPort {
    fn policy_apply_targets(
        &self,
        adapter_id: u8,
    ) -> Option<Vec<dali2rust_domain::registry::PolicyApplyCell>> {
        (adapter_id == 0).then(|| self.policy_cells.clone())
    }
}

impl SceneReadPort for StubGroupReadPort {
    fn scene_view(&self, _adapter_id: u8, _scene_id: u8) -> Option<SceneView> {
        None
    }
    fn list_scene_views(&self, _adapter_id: u8) -> Vec<SceneView> {
        Vec::new()
    }
    fn scene_matrix_view(&self, _adapter_id: u8, _scene_id: u8) -> Option<SceneMatrixView> {
        None
    }
    fn scene_apply_snapshot(&self, _adapter_id: u8, _scene_id: u8) -> Option<SceneApplySnapshot> {
        None
    }
}

struct Harness {
    _host: dali2rust_bus::BusHost,
    publisher: BusPublisher,
    read: OperationTrackerHttpRead,
    _fake_dali: std::thread::JoinHandle<()>,
    fake_dali_peak_backlog: Arc<AtomicU32>,
    orchestrator_counters: Arc<ApplyOrchestratorCounters>,
    attribute_writes: Arc<Mutex<Vec<u8>>>,
}

fn spawn_harness(rows: Vec<GroupApplyRowView>) -> Harness {
    spawn_harness_with_mute(rows, Arc::new(AtomicBool::new(false)))
}

fn spawn_harness_with_mute(rows: Vec<GroupApplyRowView>, mute: Arc<AtomicBool>) -> Harness {
    spawn_harness_full(rows, Vec::new(), mute)
}

fn spawn_harness_with_policy(
    policy_cells: Vec<dali2rust_domain::registry::PolicyApplyCell>,
) -> Harness {
    spawn_harness_full(Vec::new(), policy_cells, Arc::new(AtomicBool::new(false)))
}

fn spawn_harness_full(
    rows: Vec<GroupApplyRowView>,
    policy_cells: Vec<dali2rust_domain::registry::PolicyApplyCell>,
    mute: Arc<AtomicBool>,
) -> Harness {
    const FAKE_DALI_HANDLED: &[&str] = &[
        "DaliProgramGroupMembershipCommand",
        "DaliWriteAttributesCommand",
    ];
    let config = BusConfig {
        commands_ingress: 4,
        ..BusConfig::default()
    };
    let (host, publisher, (apply_cmd_rx, tracker_cmd_rx, dali_cmd_rx, apply_ev_rx, tracker_ev_rx, tracker_conf_rx)) =
        BusHost::spawn(config, |reg| {
            (
                reg.subscribe_commands(4, APPLY_ORCHESTRATOR_HANDLED_COMMANDS),
                reg.subscribe_commands(16, OPERATION_TRACKER_HANDLED_COMMANDS),
                reg.subscribe_commands(1, FAKE_DALI_HANDLED),
                reg.subscribe_events(16, APPLY_ORCHESTRATOR_HANDLED_EVENTS),
                reg.subscribe_events(64, OPERATION_TRACKER_HANDLED_EVENTS),
                reg.subscribe_confirmations(16),
            )
        });

    let tracker = Arc::new(Mutex::new(OperationTrackerInner::new()));
    let _tracker_join = spawn_operation_tracker_worker(
        tracker_cmd_rx,
        tracker_ev_rx,
        tracker_conf_rx,
        publisher.clone(),
        BusId::default(),
        Arc::clone(&tracker),
        Arc::new(OperationTrackerCounters::default()),
    );
    let orchestrator_counters = Arc::new(ApplyOrchestratorCounters::default());
    let _orch_join = spawn_apply_orchestrator_worker(
        apply_cmd_rx,
        apply_ev_rx,
        publisher.clone(),
        BusId::default(),
        Arc::new(StubGroupReadPort { rows, policy_cells }),
        Arc::new(OperationTrackerHttpRead(Arc::clone(&tracker))),
        Arc::clone(&orchestrator_counters),
    );

    let fake_dali_peak_backlog = Arc::new(AtomicU32::new(0));
    let peak = Arc::clone(&fake_dali_peak_backlog);
    let attribute_writes = Arc::new(Mutex::new(Vec::new()));
    let writes = Arc::clone(&attribute_writes);
    let fake_publisher = publisher.clone();
    let dali_rx: BusSubscriberRx = dali_cmd_rx;
    let fake_dali = std::thread::spawn(move || {
        while let Ok(frame) = dali_rx.recv() {
            let mut backlog = 1u32;
            let mut frames = vec![frame];
            while let Ok(extra) = dali_rx.try_recv() {
                backlog += 1;
                frames.push(extra);
            }
            peak.fetch_max(backlog, Ordering::Relaxed);
            if mute.load(Ordering::Relaxed) {
                continue;
            }
            for frame in frames {
                answer_program_command(&fake_publisher, &frame, &writes);
            }
        }
    });

    Harness {
        _host: host,
        publisher,
        read: OperationTrackerHttpRead(tracker),
        _fake_dali: fake_dali,
        fake_dali_peak_backlog,
        orchestrator_counters,
        attribute_writes,
    }
}

fn answer_program_command(
    publisher: &BusPublisher,
    frame: &BusFrame,
    attribute_writes: &Arc<Mutex<Vec<u8>>>,
) {
    let BusFrame::Command(ce) = frame else {
        return;
    };
    match &ce.payload {
        BusCommandPayload::DaliProgramGroupMembershipCommand(cmd) => {
            answer_group_membership(publisher, ce.meta.correlation_id, ce.meta.target_adapter_id, cmd);
        }
        BusCommandPayload::DaliWriteAttributesCommand(cmd) => {
            attribute_writes.lock().expect("writes").push(cmd.short_address);
            answer_write_attributes(publisher, ce.meta.correlation_id, ce.meta.target_adapter_id, cmd);
        }
        _ => {}
    }
}

fn answer_write_attributes(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    cmd: &dali2rust_contracts::msg::DaliWriteAttributesCommand,
) {
    let written = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        target_adapter_id,
        Some(Origin::Internal),
        dali2rust_contracts::msg::DaliAttributesWrittenEvent {
            short_address: cmd.short_address,
            registry_adapter_id: cmd.registry_adapter_id,
            fade_time_ms: None,
            fade_rate: None,
            power_on_level: cmd.power_on_level,
            system_failure_level: cmd.system_failure_level,
            extended_fade_time_ms: None,
            tc_coolest_mirek: None,
            tc_warmest_mirek: None,
            min_level: None,
            max_level: None,
            dimming_curve: None,
        },
    );
    publish_until_queued(publisher, written);
    if !cmd.signals_operation {
        return;
    }
    for signal in [
        dali2rust_contracts::msg::OperationWorkerSignalEvent::started(correlation_id),
        dali2rust_contracts::msg::OperationWorkerSignalEvent::succeeded(correlation_id),
    ] {
        let ev = dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            target_adapter_id,
            Some(Origin::Internal),
            signal,
        );
        publish_until_queued(publisher, ev);
    }
}

fn answer_group_membership(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    cmd: &dali2rust_contracts::msg::DaliProgramGroupMembershipCommand,
) {
    let DaliProgramTarget::VirtualLamp { virtual_lamp_id } = cmd.target else {
        return;
    };
    let outcome = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        target_adapter_id,
        Some(Origin::Internal),
        dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            target: cmd.target,
            group_id: cmd.group_id,
            action: cmd.action,
            physical_short_address: Some(virtual_lamp_id),
            membership: Some(1u16 << cmd.group_id),
            error: None,
        },
    );
    publish_until_queued(publisher, outcome);
}

fn publish_until_queued(
    publisher: &BusPublisher,
    event: dali2rust_contracts::msg::EventEnvelope,
) {
    wait_until(
        || {
            publisher.try_publish(BusChannel::Events, BusFrame::event(event.clone()))
                == PublishResult::Queued
        },
        Duration::from_secs(1),
    );
}

fn bound_row(vl: u8, changed_groups: u16) -> GroupApplyRowView {
    GroupApplyRowView {
        virtual_lamp_id: vl,
        desired_groups_mask: changed_groups,
        applied_groups_mask: 0,
        binding_short: Some(vl),
    }
}

fn unbound_row(vl: u8, changed_groups: u16) -> GroupApplyRowView {
    GroupApplyRowView {
        virtual_lamp_id: vl,
        desired_groups_mask: changed_groups,
        applied_groups_mask: 0,
        binding_short: None,
    }
}

fn publish_execute(publisher: &BusPublisher, workflow: u64, operation_key: &str) {
    let execute = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        BusId::default().0,
        Some(Origin::Api),
        GroupApplyExecuteCommand {
            registry_adapter_id: 0,
            operation_key: dali2rust_contracts::msg::fixed_text_32(operation_key),
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, BusFrame::command(execute)),
        PublishResult::Queued
    );
}

#[test]
fn paced_apply_converges_through_a_one_slot_worker_inbox() {
    let mut rows: Vec<GroupApplyRowView> = (0..4).map(|vl| bound_row(vl, 0xFFFF)).collect();
    rows.push(unbound_row(10, 0xFFFF));
    rows.push(unbound_row(11, 0xFFFF));
    let harness = spawn_harness(rows);

    publish_execute(&harness.publisher, 7001, "grp-apply-0-7001");
    wait_until(
        || {
            harness
                .read
                .operation_view("grp-apply-0-7001")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_secs(10),
    );
    let view = harness
        .read
        .operation_view("grp-apply-0-7001")
        .expect("operation view");
    assert_eq!(view.status, "succeeded", "view: {view:?}");
    let result = view.result.expect("group apply result");
    let result = result.as_group_apply().expect("group apply variant").clone();
    assert_eq!(result.programmed_total, 64);
    assert_eq!(result.skipped_total, 32);
    assert_eq!(result.failed_total, 0);
    assert_eq!(
        harness.fake_dali_peak_backlog.load(Ordering::Relaxed),
        1,
        "window-of-1: never more than one program command in flight"
    );
    assert_eq!(
        harness
            .orchestrator_counters
            .cells_published
            .load(Ordering::Relaxed),
        64
    );
    assert_eq!(
        harness
            .orchestrator_counters
            .terminal_signal_publish_failed
            .load(Ordering::Relaxed),
        0,
        "terminal safety-net signal must publish on the success path"
    );
}

#[test]
fn missing_adapter_snapshot_fails_the_operation() {
    let harness = spawn_harness(Vec::new());
    let execute = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        7002,
        BusId::default().0,
        Some(Origin::Api),
        GroupApplyExecuteCommand {
            registry_adapter_id: 3,
            operation_key: dali2rust_contracts::msg::fixed_text_32("grp-apply-3-7002"),
        },
    );
    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Commands, BusFrame::command(execute)),
        PublishResult::Queued
    );
    wait_until(
        || {
            harness
                .read
                .operation_view("grp-apply-3-7002")
                .map(|view| view.status == "failed")
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    );
    let view = harness
        .read
        .operation_view("grp-apply-3-7002")
        .expect("operation view");
    assert_eq!(view.status, "failed");
    assert_eq!(view.error.expect("error").code, "not_found");
}

#[test]
fn empty_diff_run_succeeds_with_zero_outcomes() {
    let rows = vec![GroupApplyRowView {
        virtual_lamp_id: 0,
        desired_groups_mask: 0b1010,
        applied_groups_mask: 0b1010,
        binding_short: Some(0),
    }];
    let harness = spawn_harness(rows);
    publish_execute(&harness.publisher, 7003, "grp-apply-0-7003");
    wait_until(
        || {
            harness
                .read
                .operation_view("grp-apply-0-7003")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    );
    assert_eq!(
        harness
            .read
            .operation_view("grp-apply-0-7003")
            .map(|view| view.status.to_string()),
        Some("succeeded".to_string())
    );
}

#[test]
fn a_run_aborts_when_its_operation_ends_even_with_no_event_to_hear() {
    assert!(
        !APPLY_ORCHESTRATOR_HANDLED_EVENTS.contains(&"OperationStatusChangedEvent"),
        "this test proves the POLL carries the abort; if the event is routed \
         here again it could be carrying it instead"
    );

    let mute = Arc::new(AtomicBool::new(true));
    let rows: Vec<GroupApplyRowView> = (0..4).map(|vl| bound_row(vl, 0xFFFF)).collect();
    let harness = spawn_harness_with_mute(rows, Arc::clone(&mute));

    publish_execute(&harness.publisher, 7010, "grp-apply-0-7010");
    wait_until(
        || {
            harness
                .orchestrator_counters
                .cells_published
                .load(Ordering::Relaxed)
                > 0
        },
        Duration::from_secs(5),
    );

    let reset = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        7011,
        BusId::default().0,
        Some(Origin::Api),
        dali2rust_contracts::msg::OperationRegistryResetCommand::default(),
    );
    assert_eq!(
        harness
            .publisher
            .try_publish(BusChannel::Commands, BusFrame::command(reset)),
        PublishResult::Queued
    );

    wait_until(
        || {
            harness
                .orchestrator_counters
                .runs_aborted
                .load(Ordering::Relaxed)
                > 0
        },
        Duration::from_millis(2_000),
    );
    assert_eq!(
        harness
            .orchestrator_counters
            .runs_aborted
            .load(Ordering::Relaxed),
        1,
        "the run must abandon an operation nobody is waiting on"
    );
    let published = harness
        .orchestrator_counters
        .cells_published
        .load(Ordering::Relaxed);
    assert!(
        published < 64,
        "the run kept driving the wire after its operation ended: {published} of 64 cells"
    );
    drop(mute);
}

#[test]
fn a_policy_apply_reaches_every_device_and_not_only_the_first() {
    let cells: Vec<dali2rust_domain::registry::PolicyApplyCell> = (0..6)
        .map(|short_address| dali2rust_domain::registry::PolicyApplyCell {
            short_address,
            system_failure_level: Some(0),
            power_on_level: Some(0),
        })
        .collect();
    let harness = spawn_harness_with_policy(cells);
    publish_policy_execute(&harness.publisher, 7020, "policy-apply-0-7020");

    wait_until(
        || {
            harness
                .read
                .operation_view("policy-apply-0-7020")
                .map(|view| view.status == "succeeded")
                .unwrap_or(false)
        },
        Duration::from_secs(10),
    );
    let view = harness
        .read
        .operation_view("policy-apply-0-7020")
        .expect("operation view");
    assert_eq!(view.status, "succeeded", "view: {view:?}");
    assert_eq!(
        *harness.attribute_writes.lock().expect("writes"),
        vec![0u8, 1, 2, 3, 4, 5],
        "every known device is written, in order",
    );
    assert_eq!(
        harness
            .orchestrator_counters
            .runs_aborted
            .load(Ordering::Relaxed),
        0,
        "a cell's own worker signal must not end the run's operation",
    );
    assert_eq!(
        harness
            .orchestrator_counters
            .outcome_timeouts
            .load(Ordering::Relaxed),
        0,
        "the written event is the cell's outcome and must be routed here",
    );
}

fn publish_policy_execute(publisher: &BusPublisher, workflow: u64, operation_key: &str) {
    let execute = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        BusId::default().0,
        Some(Origin::Api),
        dali2rust_contracts::msg::PolicyApplyExecuteCommand {
            registry_adapter_id: 0,
            operation_key: dali2rust_contracts::msg::fixed_text_32(operation_key),
        },
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, BusFrame::command(execute)),
        PublishResult::Queued
    );
}
