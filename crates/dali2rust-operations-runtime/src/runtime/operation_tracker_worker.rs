use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dali2rust_bsp::std_thread_stack;
use dali2rust_bsp::unix_clock::unix_wall_clock_millis;
use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{fixed_text_32, OperationStatusChangedEvent, Origin};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, CompactErrorPayload, DaliAttributeReadOutcomesEvent,
    DaliGroupMembershipProgrammedEvent, DaliSceneProgrammedEvent, ErrorCode, OperationStatus,
    OperationType, OperationWorkerSignal, SceneProgramAction,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::VIRTUAL_LAMP_COUNT;
use dali2rust_domain::registry::{
    OperationApplyResultView, OperationAttributeReadOutcomesView, OperationErrorView,
    OperationGroupApplyOutcomeView, OperationAddressChangeResultView, OperationGroupApplyResultView,
    OperationReplaceDeviceResultView, OperationRestoredSlicesView,
    OperationIdentifyResultView,
    OperationReadPort, OperationSceneApplyOutcomeView, OperationSceneApplyResultView,
    OperationView,
};

use dali2rust_bsp::esp_thread;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedOperationStatus {
    pub operation_key: String,
    pub status: OperationStatus,
    pub error_code: Option<ErrorCode>,
}

#[derive(Debug, Default)]
pub struct OperationTrackerCounters {
    pub accepted: AtomicU32,
    pub running: AtomicU32,
    pub succeeded: AtomicU32,
    pub failed: AtomicU32,
    pub timed_out: AtomicU32,
    pub cancelled: AtomicU32,
    pub ignored_commands: AtomicU32,
    pub ignored_events: AtomicU32,
    pub pending_outcomes_expired: AtomicU32,
}

const MAX_APPLY_DETAIL_ROWS: usize = 128;

const IDLE_RECV_TIMEOUT: Duration = Duration::from_millis(50);

const PENDING_OUTCOME_TTL: Duration = Duration::from_secs(10);

struct PendingEntry<V> {
    expires_at: Instant,
    value: V,
}

impl<V> PendingEntry<V> {
    fn new(value: V) -> Self {
        Self {
            expires_at: Instant::now() + PENDING_OUTCOME_TTL,
            value,
        }
    }
}

type PendingMap<V> = HashMap<u64, PendingEntry<V>>;

trait PendingBufferOps {
    fn remove_correlation(&mut self, corr: u64);
    fn evict_expired(&mut self, now: Instant) -> u32;
    fn expire_now(&mut self, now: Instant);
}

impl<V> PendingBufferOps for PendingMap<V> {
    fn remove_correlation(&mut self, corr: u64) {
        self.remove(&corr);
    }

    fn evict_expired(&mut self, now: Instant) -> u32 {
        let before = self.len();
        self.retain(|_, entry| now < entry.expires_at);
        (before - self.len()) as u32
    }

    fn expire_now(&mut self, now: Instant) {
        for entry in self.values_mut() {
            entry.expires_at = now;
        }
    }
}

const MAX_PENDING_CORRELATIONS: usize = 16;

const MAX_PENDING_OUTCOMES: usize = 256;

const MAX_PENDING_SIGNALS: usize = 4;

fn pending_cap_reached<V>(map: &PendingMap<V>, workflow_c: u64) -> bool {
    map.len() >= MAX_PENDING_CORRELATIONS && !map.contains_key(&workflow_c)
}

fn insert_pending_single<V>(map: &mut PendingMap<V>, workflow_c: u64, value: V) {
    if pending_cap_reached(map, workflow_c) {
        return;
    }
    map.insert(workflow_c, PendingEntry::new(value));
}

fn queue_pending_list<V>(
    map: &mut PendingMap<Vec<V>>,
    workflow_c: u64,
    value: V,
    max_per_correlation: usize,
) {
    if pending_cap_reached(map, workflow_c) {
        return;
    }
    let entry = map
        .entry(workflow_c)
        .or_insert_with(|| PendingEntry::new(Vec::new()));
    if entry.value.len() < max_per_correlation {
        entry.value.push(value);
    }
}

fn replay_pending<V>(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    take: fn(&mut OperationTrackerInner) -> &mut PendingMap<Vec<V>>,
    mut apply: impl FnMut(&mut OperationTrackerInner, &V),
) {
    let pending = take(guard)
        .remove(&workflow_c)
        .map(|entry| entry.value)
        .unwrap_or_default();
    for item in pending {
        apply(guard, &item);
        if !guard.active.contains_key(&workflow_c) {
            break;
        }
    }
}

fn lock_tracker(state: &Mutex<OperationTrackerInner>) -> MutexGuard<'_, OperationTrackerInner> {
    state.lock().unwrap_or_else(|e| {
        log::error!("operation tracker state poisoned by a previous panic; recovering");
        state.clear_poison();
        e.into_inner()
    })
}

#[derive(Clone, Default)]
struct GroupApplyAccumulator {
    expected_outcomes: u16,
    programmed: Vec<OperationGroupApplyOutcomeView>,
    skipped: Vec<OperationGroupApplyOutcomeView>,
    failed: Vec<OperationGroupApplyOutcomeView>,
    programmed_total: u16,
    skipped_total: u16,
    failed_total: u16,
    seen_cells: [u64; 16],
    terminal_error: Option<(ErrorCode, String)>,
}

#[derive(Clone, Default)]
struct SceneApplyAccumulator {
    expected_outcomes: u16,
    written: Vec<OperationSceneApplyOutcomeView>,
    updated: Vec<OperationSceneApplyOutcomeView>,
    cleared: Vec<OperationSceneApplyOutcomeView>,
    skipped: Vec<OperationSceneApplyOutcomeView>,
    failed: Vec<OperationSceneApplyOutcomeView>,
    written_total: u16,
    updated_total: u16,
    cleared_total: u16,
    skipped_total: u16,
    failed_total: u16,
    seen_rows: u64,
    terminal_error: Option<(ErrorCode, String)>,
}

fn push_capped<T>(list: &mut Vec<T>, outcome: T) {
    if list.len() < MAX_APPLY_DETAIL_ROWS {
        list.push(outcome);
    }
}

#[derive(Clone)]
enum OperationDetailState {
    None,
    GroupApply(GroupApplyAccumulator),
    SceneApply(SceneApplyAccumulator),
    AttributeRead(OperationAttributeReadOutcomesView),
    Identify(OperationIdentifyResultView),
    AddressChange(OperationAddressChangeResultView),
    ReplaceDevice(OperationReplaceDeviceResultView),
    HaDiscovery(dali2rust_domain::registry::OperationHaDiscoveryResultView),
}

#[derive(Clone)]
struct TrackedOp {
    workflow_correlation_id: u64,
    operation_key: String,
    op_type: OperationType,
    adapter_id: u32,
    status: OperationStatus,
    deadline: Instant,
    finished_retention_ms: u32,
    started_at_unix_ms: u64,
    detail: OperationDetailState,
}

pub struct HttpOpRow {
    pub view: OperationView,
    pub expires_after: Option<Instant>,
}

pub struct OperationTrackerInner {
    pub events: Vec<RecordedOperationStatus>,
    pub http_by_key: HashMap<String, HttpOpRow>,
    active: HashMap<u64, TrackedOp>,
    coalesce: HashMap<(u8, u32), u64>,
    pending_group_apply: PendingMap<Vec<DaliGroupMembershipProgrammedEvent>>,
    pending_scene_apply: PendingMap<Vec<DaliSceneProgrammedEvent>>,
    pending_attribute_read: PendingMap<OperationAttributeReadOutcomesView>,
    pending_identify: PendingMap<OperationIdentifyResultView>,
    pending_address_change: PendingMap<OperationAddressChangeResultView>,
    pending_replace_device: PendingMap<OperationReplaceDeviceResultView>,
    pending_worker_signals: PendingMap<Vec<dali2rust_contracts::msg::OperationWorkerSignalEvent>>,
}

impl Default for OperationTrackerInner {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationTrackerInner {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            http_by_key: HashMap::new(),
            active: HashMap::new(),
            coalesce: HashMap::new(),
            pending_group_apply: HashMap::new(),
            pending_scene_apply: HashMap::new(),
            pending_attribute_read: HashMap::new(),
            pending_identify: HashMap::new(),
            pending_address_change: HashMap::new(),
            pending_replace_device: HashMap::new(),
            pending_worker_signals: HashMap::new(),
        }
    }

    pub fn pending_worker_signal_correlations(&self) -> usize {
        self.pending_worker_signals.len()
    }

    fn for_each_pending_buffer(&mut self, mut f: impl FnMut(&mut dyn PendingBufferOps)) {
        let Self {
            events: _,
            http_by_key: _,
            active: _,
            coalesce: _,
            pending_group_apply,
            pending_scene_apply,
            pending_attribute_read,
            pending_identify,
            pending_address_change,
            pending_replace_device,
            pending_worker_signals,
        } = self;
        f(pending_group_apply);
        f(pending_scene_apply);
        f(pending_attribute_read);
        f(pending_identify);
        f(pending_address_change);
        f(pending_replace_device);
        f(pending_worker_signals);
    }

    pub fn expire_pending_entries(&mut self) {
        let now = Instant::now();
        self.for_each_pending_buffer(|buf| buf.expire_now(now));
    }

    fn push_event(&mut self, rec: RecordedOperationStatus) {
        const MAX: usize = 512;
        if self.events.len() >= MAX {
            let drop = self.events.len() - MAX + 1;
            self.events.drain(0..drop);
        }
        self.events.push(rec);
    }

    fn status_name(s: OperationStatus) -> &'static str {
        s.rest_name()
    }

    fn operation_type_name(op_type: OperationType) -> &'static str {
        op_type.rest_name()
    }

    fn upsert_http(
        &mut self,
        key: &str,
        view: OperationView,
        retention: Option<Instant>,
    ) {
        self.http_by_key.insert(
            key.to_string(),
            HttpOpRow {
                view,
                expires_after: retention,
            },
        );
    }

    fn remove_http_if_expired(&mut self, now: Instant) {
        self.http_by_key
            .retain(|_, row| row.expires_after.is_none_or(|t| now < t));
    }
}

impl GroupApplyAccumulator {
    fn result_view(&self) -> OperationGroupApplyResultView {
        OperationGroupApplyResultView {
            programmed: self.programmed.clone(),
            skipped: self.skipped.clone(),
            failed: self.failed.clone(),
            programmed_total: self.programmed_total,
            skipped_total: self.skipped_total,
            failed_total: self.failed_total,
        }
    }
}

impl SceneApplyAccumulator {
    fn result_view(&self) -> OperationSceneApplyResultView {
        OperationSceneApplyResultView {
            written: self.written.clone(),
            updated: self.updated.clone(),
            cleared: self.cleared.clone(),
            skipped: self.skipped.clone(),
            failed: self.failed.clone(),
            written_total: self.written_total,
            updated_total: self.updated_total,
            cleared_total: self.cleared_total,
            skipped_total: self.skipped_total,
            failed_total: self.failed_total,
        }
    }
}

impl OperationDetailState {
    fn from_begin(op_type: OperationType, expected_outcomes: u16) -> Self {
        match op_type {
            OperationType::GroupApply => Self::GroupApply(GroupApplyAccumulator {
                expected_outcomes,
                ..GroupApplyAccumulator::default()
            }),
            OperationType::SceneApply => Self::SceneApply(SceneApplyAccumulator {
                expected_outcomes,
                ..SceneApplyAccumulator::default()
            }),
            _ => Self::None,
        }
    }

    fn result_view(&self) -> Option<OperationApplyResultView> {
        match self {
            OperationDetailState::GroupApply(result) => {
                Some(OperationApplyResultView::GroupApply(result.result_view()))
            }
            OperationDetailState::SceneApply(result) => {
                Some(OperationApplyResultView::SceneApply(result.result_view()))
            }
            OperationDetailState::Identify(result) => {
                Some(OperationApplyResultView::Identify(result.clone()))
            }
            OperationDetailState::AddressChange(result) => {
                Some(OperationApplyResultView::AddressChange(result.clone()))
            }
            OperationDetailState::ReplaceDevice(result) => {
                Some(OperationApplyResultView::ReplaceDevice(result.clone()))
            }
            OperationDetailState::HaDiscovery(result) => {
                Some(OperationApplyResultView::HaDiscoveryPublish(*result))
            }
            _ => None,
        }
    }

    fn attribute_read_view(&self) -> Option<OperationAttributeReadOutcomesView> {
        match self {
            OperationDetailState::AttributeRead(view) => Some(view.clone()),
            _ => None,
        }
    }
}

fn attribute_read_outcomes_view(
    body: &DaliAttributeReadOutcomesEvent,
) -> OperationAttributeReadOutcomesView {
    OperationAttributeReadOutcomesView {
        identity: body.identity.as_str().to_string(),
        runtime_status: body.runtime_status.as_str().to_string(),
        common_102: body.common_102.as_str().to_string(),
        dt8_color: body.dt8_color.as_str().to_string(),
        dt6_led: body.dt6_led.as_str().to_string(),
        groups: body.groups.as_str().to_string(),
        scenes: body.scenes.as_str().to_string(),
        extended: body.extended.as_str().to_string(),
        memory_banks: body.memory_banks.as_str().to_string(),
    }
}

fn ttl_remaining_ms(deadline: Instant) -> u32 {
    deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u128::from(u32::MAX)) as u32
}

fn error_code_name(code: ErrorCode) -> &'static str {
    code.rest_name()
}

fn error_view(error_payload: Option<(ErrorCode, &str)>) -> Option<OperationErrorView> {
    error_payload.map(|(code, message)| OperationErrorView {
        code: error_code_name(code).into(),
        message: message.to_string(),
    })
}

fn operation_view_for(
    op: &TrackedOp,
    status: OperationStatus,
    error_payload: Option<(ErrorCode, &str)>,
) -> OperationView {
    OperationView {
        operation_id: op.operation_key.clone(),
        operation_type: OperationTrackerInner::operation_type_name(op.op_type).into(),
        status: OperationTrackerInner::status_name(status).into(),
        error: error_view(error_payload),
        result: op.detail.result_view(),
        attribute_read_outcomes: op.detail.attribute_read_view(),
    }
}

fn publish_operation_status_changed(
    publisher: &BusPublisher,
    bus_id: BusId,
    workflow_correlation_id: u64,
    op_type: OperationType,
    status: OperationStatus,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    ttl_remaining_ms: u32,
    operation_key: &str,
    error_payload: Option<(ErrorCode, &str)>,
) {
    let error = error_payload.map(|(code, msg)| CompactErrorPayload::new(code, msg));
    let env = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow_correlation_id,
        bus_id.0,
        Some(Origin::Internal),
        OperationStatusChangedEvent {
            operation_type: op_type,
            status,
            error,
            started_at_ms,
            finished_at_ms,
            ttl_remaining_ms,
            operation_key: fixed_text_32(operation_key),
        },
    );
    let frame = BusFrame::event(env);
    publish_or_drop(publisher, BusChannel::Events, frame, "operations");
}

fn bump_counter(counters: &OperationTrackerCounters, status: OperationStatus) {
    match status {
        OperationStatus::Accepted => {
            counters.accepted.fetch_add(1, Ordering::Relaxed);
        }
        OperationStatus::Running => {
            counters.running.fetch_add(1, Ordering::Relaxed);
        }
        OperationStatus::Succeeded => {
            counters.succeeded.fetch_add(1, Ordering::Relaxed);
        }
        OperationStatus::Failed => {
            counters.failed.fetch_add(1, Ordering::Relaxed);
        }
        OperationStatus::TimedOut => {
            counters.timed_out.fetch_add(1, Ordering::Relaxed);
        }
        OperationStatus::Cancelled => {
            counters.cancelled.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn transition_ttl_remaining_ms(op: &TrackedOp, status: OperationStatus) -> u32 {
    if matches!(
        status,
        OperationStatus::Succeeded
            | OperationStatus::Failed
            | OperationStatus::TimedOut
            | OperationStatus::Cancelled
    ) {
        0
    } else {
        ttl_remaining_ms(op.deadline)
    }
}

fn emit_transition(
    publisher: &BusPublisher,
    bus_id: BusId,
    inner: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    op: &TrackedOp,
    status: OperationStatus,
    finished_at: Option<u64>,
    error_payload: Option<(ErrorCode, &str)>,
) {
    let ttl_rem = transition_ttl_remaining_ms(op, status);
    publish_operation_status_changed(
        publisher,
        bus_id,
        op.workflow_correlation_id,
        op.op_type,
        status,
        op.started_at_unix_ms,
        finished_at,
        ttl_rem,
        op.operation_key.as_str(),
        error_payload,
    );

    let err_code = error_payload.map(|(c, _)| c);
    inner.push_event(RecordedOperationStatus {
        operation_key: op.operation_key.clone(),
        status,
        error_code: err_code,
    });
    bump_counter(counters, status);

    let view = operation_view_for(op, status, error_payload);
    inner.upsert_http(&op.operation_key, view, None);
}

fn clear_active_entry(
    inner: &mut OperationTrackerInner,
    workflow_correlation_id: u64,
    op: &TrackedOp,
) {
    inner.active.remove(&workflow_correlation_id);
    inner.for_each_pending_buffer(|buf| buf.remove_correlation(workflow_correlation_id));
    let k = (op.op_type as u8, op.adapter_id);
    if inner.coalesce.get(&k).copied() == Some(workflow_correlation_id) {
        inner.coalesce.remove(&k);
    }
}

fn apply_terminal(
    publisher: &BusPublisher,
    bus_id: BusId,
    inner: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_correlation_id: u64,
    op: &TrackedOp,
    status: OperationStatus,
    error_payload: Option<(ErrorCode, &str)>,
) {
    let finished_at = Some(unix_wall_clock_millis());
    let retention =
        Instant::now() + Duration::from_millis(u64::from(op.finished_retention_ms.max(1)));
    emit_transition(
        publisher,
        bus_id,
        inner,
        counters,
        op,
        status,
        finished_at,
        error_payload,
    );
    let view = operation_view_for(op, status, error_payload);
    inner.upsert_http(&op.operation_key, view, Some(retention));
    clear_active_entry(inner, workflow_correlation_id, op);
}

fn emit_terminal_with_retention(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    op: &TrackedOp,
    status: OperationStatus,
    code: ErrorCode,
) {
    publish_operation_status_changed(
        publisher,
        bus_id,
        op.workflow_correlation_id,
        op.op_type,
        status,
        op.started_at_unix_ms,
        Some(unix_wall_clock_millis()),
        0,
        op.operation_key.as_str(),
        Some((code, "")),
    );
    guard.push_event(RecordedOperationStatus {
        operation_key: op.operation_key.clone(),
        status,
        error_code: Some(code),
    });
    bump_counter(counters, status);
    let ret =
        Instant::now() + Duration::from_millis(u64::from(op.finished_retention_ms.max(1)));
    guard.upsert_http(
        &op.operation_key,
        operation_view_for(op, status, Some((code, ""))),
        Some(ret),
    );
}

pub fn spawn_operation_tracker_worker(
    cmd_rx: BusSubscriberRx,
    ev_rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    state: Arc<Mutex<OperationTrackerInner>>,
    counters: Arc<OperationTrackerCounters>,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"operation_tracker_worker",
        std_thread_stack::STATE_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || {
            loop {
                match cmd_rx.recv_timeout(IDLE_RECV_TIMEOUT) {
                    Ok(frame) => {
                        handle_command_frame(&frame, &publisher, bus_id, &state, &counters);
                        while let Ok(frame) = cmd_rx.try_recv() {
                            handle_command_frame(&frame, &publisher, bus_id, &state, &counters);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                while let Ok(frame) = ev_rx.try_recv() {
                    handle_event_frame(&frame, &publisher, bus_id, &state, &counters);
                }
                drain_rejected_deliveries(&conf_rx, &publisher, bus_id, &state, &counters);
                tick_deadlines(&publisher, bus_id, &state, &counters);
            }
        },
    )
}

fn handle_command_frame(
    frame: &BusFrame,
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
) {
    let Some(ce) = frame.command_for(bus_id) else {
        return;
    };
    let meta = &ce.meta;
    dispatch_tracker_command(
        &ce.payload,
        publisher,
        bus_id,
        state,
        counters,
        meta.correlation_id,
        meta.target_adapter_id as u32,
    );
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const OPERATION_TRACKER_HANDLED_COMMANDS;
    fn dispatch_tracker_command(
        payload: &BusCommandPayload,
        publisher: &BusPublisher,
        bus_id: BusId,
        state: &Arc<Mutex<OperationTrackerInner>>,
        counters: &Arc<OperationTrackerCounters>,
        corr: u64,
        adapter_id: u32,
    );
    payload = payload;
    ignored = { counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    OperationBeginCommand(body) =>
        handle_operation_begin(publisher, bus_id, state, counters, corr, adapter_id, body),
    OperationRegistryResetCommand(_) =>
        handle_operation_registry_reset(publisher, bus_id, state, counters),
}

fn handle_operation_begin(
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
    corr: u64,
    adapter_id: u32,
    body: &dali2rust_contracts::msg::OperationBeginCommand,
) {
    let op_type = body.operation_type;
    let deadline = Instant::now() + Duration::from_millis(u64::from(body.ttl_ms.max(1)));
    let mut guard = lock_tracker(state);
    let coalesce_key = (op_type as u8, adapter_id);
    let coalesces = op_type.coalesces_per_adapter();
    if coalesces {
        supersede_stale_operation(publisher, bus_id, &mut guard, counters, coalesce_key, corr);
    }
    let op = TrackedOp {
        workflow_correlation_id: corr,
        operation_key: body.operation_key.as_str().to_string(),
        op_type,
        adapter_id,
        status: OperationStatus::Accepted,
        deadline,
        finished_retention_ms: body.finished_retention_ms.max(1),
        started_at_unix_ms: unix_wall_clock_millis(),
        detail: OperationDetailState::from_begin(op_type, body.expected_outcomes),
    };
    emit_transition(
        publisher,
        bus_id,
        &mut guard,
        counters.as_ref(),
        &op,
        OperationStatus::Accepted,
        None,
        None,
    );
    if coalesces {
        guard.coalesce.insert(coalesce_key, corr);
    }
    guard.active.insert(corr, op);
    flush_begin_backlog(publisher, bus_id, &mut guard, counters, corr, op_type);
}

fn supersede_stale_operation(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &Arc<OperationTrackerCounters>,
    coalesce_key: (u8, u32),
    corr: u64,
) {
    if let Some(old_c) = guard.coalesce.get(&coalesce_key).copied() {
        if old_c != corr {
            if let Some(old_op) = guard.active.remove(&old_c) {
                emit_terminal_with_retention(
                    publisher,
                    bus_id,
                    guard,
                    counters.as_ref(),
                    &old_op,
                    OperationStatus::Cancelled,
                    ErrorCode::Superseded,
                );
                guard.coalesce.remove(&coalesce_key);
            }
        }
    }
}

fn flush_begin_backlog(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &Arc<OperationTrackerCounters>,
    corr: u64,
    op_type: OperationType,
) {
    if matches!(op_type, OperationType::GroupApply) {
        flush_pending_apply_outcomes::<GroupApplyAccumulator>(
            publisher, bus_id, guard, counters.as_ref(), corr,
        );
    }
    if matches!(op_type, OperationType::SceneApply) {
        flush_pending_apply_outcomes::<SceneApplyAccumulator>(
            publisher, bus_id, guard, counters.as_ref(), corr,
        );
    }
    if matches!(op_type, OperationType::AttributeRead) {
        if let Some(entry) = guard.pending_attribute_read.remove(&corr) {
            attach_attribute_read_outcomes(guard, corr, entry.value);
        }
    }
    flush_pending_commissioning_outcome(guard, corr, op_type);
    flush_pending_worker_signals(publisher, bus_id, guard, counters, corr);
}

fn flush_pending_commissioning_outcome(
    guard: &mut OperationTrackerInner,
    corr: u64,
    op_type: OperationType,
) {
    let detail = match op_type {
        OperationType::CommissioningIdentify => guard
            .pending_identify
            .remove(&corr)
            .map(|entry| OperationDetailState::Identify(entry.value)),
        OperationType::CommissioningAddressChange => guard
            .pending_address_change
            .remove(&corr)
            .map(|entry| OperationDetailState::AddressChange(entry.value)),
        OperationType::CommissioningReplaceDevice => guard
            .pending_replace_device
            .remove(&corr)
            .map(|entry| OperationDetailState::ReplaceDevice(entry.value)),
        _ => None,
    };
    let Some(detail) = detail else {
        return;
    };
    if let Some(op) = guard.active.get_mut(&corr) {
        op.detail = detail;
    }
}

fn attach_attribute_read_outcomes(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    view: OperationAttributeReadOutcomesView,
) {
    let Some(op) = guard.active.get_mut(&workflow_c) else {
        return;
    };
    if !matches!(op.op_type, OperationType::AttributeRead) {
        return;
    }
    op.detail = OperationDetailState::AttributeRead(view);
    let current = op.clone();
    guard.upsert_http(
        &current.operation_key,
        operation_view_for(&current, current.status, None),
        None,
    );
}

fn attach_or_buffer_result<V>(
    guard_active: &mut HashMap<u64, TrackedOp>,
    pending: &mut PendingMap<V>,
    workflow_c: u64,
    view: V,
    into_detail: impl FnOnce(V) -> OperationDetailState,
) {
    if let Some(op) = guard_active.get_mut(&workflow_c) {
        op.detail = into_detail(view);
        return;
    }
    insert_pending_single(pending, workflow_c, view);
}

fn apply_identify_outcome(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    body: &dali2rust_contracts::msg::DaliDeviceIdentifiedEvent,
) {
    attach_or_buffer_result(
        &mut guard.active,
        &mut guard.pending_identify,
        workflow_c,
        OperationIdentifyResultView {
            short_address: body.short_address,
            identify_mechanism: identify_mechanism_name(body.mechanism).to_string(),
        },
        OperationDetailState::Identify,
    );
}

fn apply_address_change_outcome(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    body: &dali2rust_contracts::msg::DaliAddressingCompletedEvent,
) {
    attach_or_buffer_result(
        &mut guard.active,
        &mut guard.pending_address_change,
        workflow_c,
        OperationAddressChangeResultView {
            old_short_address: body.old_short_address,
            new_short_address: body.new_short_address,
        },
        OperationDetailState::AddressChange,
    );
}

fn apply_replace_device_outcome(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    body: &dali2rust_contracts::msg::DaliDeviceReplacedEvent,
) {
    attach_or_buffer_result(
        &mut guard.active,
        &mut guard.pending_replace_device,
        workflow_c,
        OperationReplaceDeviceResultView {
            failed_short_address: body.failed_short_address,
            replacement_short_address: body.replacement_short_address,
            restored: OperationRestoredSlicesView {
                metadata_and_overrides: body.restored_metadata_and_overrides,
                attributes: body.restored_attributes,
                groups: body.restored_groups,
                scenes: body.restored_scenes,
            },
        },
        OperationDetailState::ReplaceDevice,
    );
}

fn identify_mechanism_name(mechanism: dali2rust_contracts::msg::IdentifyMechanism) -> &'static str {
    match mechanism {
        dali2rust_contracts::msg::IdentifyMechanism::BlinkRecallMaxMin => "blink_recall_max_min",
        dali2rust_contracts::msg::IdentifyMechanism::IdentifyDevice => "identify_device",
    }
}

fn apply_attribute_read_outcomes(
    guard: &mut OperationTrackerInner,
    workflow_c: u64,
    body: &DaliAttributeReadOutcomesEvent,
    origin: Origin,
) {
    let view = attribute_read_outcomes_view(body);
    if guard.active.contains_key(&workflow_c) {
        attach_attribute_read_outcomes(guard, workflow_c, view);
        return;
    }
    if origin.is_background() {
        return;
    }
    insert_pending_single(&mut guard.pending_attribute_read, workflow_c, view);
}

fn handle_operation_registry_reset(
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
) {
    let mut guard = lock_tracker(state);
    let corrs: Vec<u64> = guard.active.keys().copied().collect();
    for c in corrs {
        let Some(op) = guard.active.get(&c).cloned() else {
            continue;
        };
        emit_terminal_with_retention(
            publisher,
            bus_id,
            &mut guard,
            counters.as_ref(),
            &op,
            OperationStatus::Cancelled,
            ErrorCode::RegistryReset,
        );
    }
    guard.active.clear();
    guard.coalesce.clear();
}


fn group_action_name(action: dali2rust_contracts::msg::GroupMembershipAction) -> &'static str {
    match action {
        dali2rust_contracts::msg::GroupMembershipAction::Add => "add",
        dali2rust_contracts::msg::GroupMembershipAction::Remove => "remove",
    }
}

fn target_virtual_lamp_id(target: &dali2rust_contracts::msg::DaliProgramTarget) -> u8 {
    match target {
        dali2rust_contracts::msg::DaliProgramTarget::VirtualLamp { virtual_lamp_id } => *virtual_lamp_id,
        dali2rust_contracts::msg::DaliProgramTarget::Short { short_address } => *short_address,
    }
}

fn promote_running(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_c: u64,
    op: &TrackedOp,
) {
    if op.status != OperationStatus::Accepted {
        return;
    }
    publish_operation_status_changed(
        publisher,
        bus_id,
        op.workflow_correlation_id,
        op.op_type,
        OperationStatus::Running,
        op.started_at_unix_ms,
        None,
        ttl_remaining_ms(op.deadline),
        op.operation_key.as_str(),
        None,
    );
    guard.push_event(RecordedOperationStatus {
        operation_key: op.operation_key.clone(),
        status: OperationStatus::Running,
        error_code: None,
    });
    bump_counter(counters, OperationStatus::Running);
    if let Some(slot) = guard.active.get_mut(&workflow_c) {
        slot.status = OperationStatus::Running;
    }
    let mut running_op = op.clone();
    running_op.status = OperationStatus::Running;
    guard.upsert_http(
        &op.operation_key,
        operation_view_for(&running_op, OperationStatus::Running, None),
        None,
    );
}

type Bucket<'a, O> = (&'a mut Vec<O>, &'a mut u16);

fn bump_bucket<O>((rows, total): Bucket<'_, O>, outcome: O) {
    *total = total.saturating_add(1);
    push_capped(rows, outcome);
}

enum OutcomeBucket {
    Success,
    Skipped,
    Failed,
}

fn row_bit_already_set(row: &mut u64, virtual_lamp_id: u8) -> bool {
    let bit = 1u64 << virtual_lamp_id;
    if *row & bit != 0 {
        return true;
    }
    *row |= bit;
    false
}

trait ApplyFamily: Sized {
    type Body: Clone;
    type Outcome;

    fn from_detail(detail: &mut OperationDetailState) -> Option<&mut Self>;
    fn pending(inner: &mut OperationTrackerInner) -> &mut PendingMap<Vec<Self::Body>>;
    fn already_counted(&mut self, body: &Self::Body) -> bool;
    fn outcome_view(body: &Self::Body) -> Self::Outcome;
    fn error(body: &Self::Body) -> Option<&CompactErrorPayload>;
    fn bucket_mut(
        &mut self,
        body: &Self::Body,
        bucket: OutcomeBucket,
    ) -> Bucket<'_, Self::Outcome>;
    fn terminal_error_mut(&mut self) -> &mut Option<(ErrorCode, String)>;
    fn finished(&self) -> bool;
}

impl ApplyFamily for GroupApplyAccumulator {
    type Body = DaliGroupMembershipProgrammedEvent;
    type Outcome = OperationGroupApplyOutcomeView;

    fn from_detail(detail: &mut OperationDetailState) -> Option<&mut Self> {
        match detail {
            OperationDetailState::GroupApply(result) => Some(result),
            _ => None,
        }
    }

    fn pending(inner: &mut OperationTrackerInner) -> &mut PendingMap<Vec<Self::Body>> {
        &mut inner.pending_group_apply
    }

    fn already_counted(&mut self, body: &Self::Body) -> bool {
        let dali2rust_contracts::msg::DaliProgramTarget::VirtualLamp { virtual_lamp_id } =
            body.target
        else {
            return false;
        };
        if virtual_lamp_id >= VIRTUAL_LAMP_COUNT || usize::from(body.group_id) >= self.seen_cells.len() {
            return false;
        }
        let row = &mut self.seen_cells[usize::from(body.group_id)];
        row_bit_already_set(row, virtual_lamp_id)
    }

    fn outcome_view(body: &Self::Body) -> Self::Outcome {
        OperationGroupApplyOutcomeView {
            virtual_lamp_id: target_virtual_lamp_id(&body.target),
            group_id: body.group_id,
            action: group_action_name(body.action).to_string(),
            physical_short_address: body.physical_short_address,
            reason: body
                .error
                .as_ref()
                .map(|error| error.message.as_str().to_string()),
        }
    }

    fn error(body: &Self::Body) -> Option<&CompactErrorPayload> {
        body.error.as_ref()
    }

    fn bucket_mut(
        &mut self,
        _body: &Self::Body,
        bucket: OutcomeBucket,
    ) -> Bucket<'_, Self::Outcome> {
        match bucket {
            OutcomeBucket::Success => (&mut self.programmed, &mut self.programmed_total),
            OutcomeBucket::Skipped => (&mut self.skipped, &mut self.skipped_total),
            OutcomeBucket::Failed => (&mut self.failed, &mut self.failed_total),
        }
    }

    fn terminal_error_mut(&mut self) -> &mut Option<(ErrorCode, String)> {
        &mut self.terminal_error
    }

    fn finished(&self) -> bool {
        let total = usize::from(self.programmed_total)
            + usize::from(self.skipped_total)
            + usize::from(self.failed_total);
        total >= usize::from(self.expected_outcomes)
    }
}

impl ApplyFamily for SceneApplyAccumulator {
    type Body = DaliSceneProgrammedEvent;
    type Outcome = OperationSceneApplyOutcomeView;

    fn from_detail(detail: &mut OperationDetailState) -> Option<&mut Self> {
        match detail {
            OperationDetailState::SceneApply(result) => Some(result),
            _ => None,
        }
    }

    fn pending(inner: &mut OperationTrackerInner) -> &mut PendingMap<Vec<Self::Body>> {
        &mut inner.pending_scene_apply
    }

    fn already_counted(&mut self, body: &Self::Body) -> bool {
        let dali2rust_contracts::msg::DaliProgramTarget::VirtualLamp { virtual_lamp_id } =
            body.target
        else {
            return false;
        };
        if virtual_lamp_id >= VIRTUAL_LAMP_COUNT {
            return false;
        }
        row_bit_already_set(&mut self.seen_rows, virtual_lamp_id)
    }

    fn outcome_view(body: &Self::Body) -> Self::Outcome {
        OperationSceneApplyOutcomeView {
            virtual_lamp_id: target_virtual_lamp_id(&body.target),
            action: scene_action_name(body.action).to_string(),
            physical_short_address: body.physical_short_address,
            reason: body
                .error
                .as_ref()
                .map(|error| error.message.as_str().to_string()),
        }
    }

    fn error(body: &Self::Body) -> Option<&CompactErrorPayload> {
        body.error.as_ref()
    }

    fn bucket_mut(
        &mut self,
        body: &Self::Body,
        bucket: OutcomeBucket,
    ) -> Bucket<'_, Self::Outcome> {
        match bucket {
            OutcomeBucket::Success => match body.action {
                SceneProgramAction::Write => (&mut self.written, &mut self.written_total),
                SceneProgramAction::Update => (&mut self.updated, &mut self.updated_total),
                SceneProgramAction::Clear => (&mut self.cleared, &mut self.cleared_total),
            },
            OutcomeBucket::Skipped => (&mut self.skipped, &mut self.skipped_total),
            OutcomeBucket::Failed => (&mut self.failed, &mut self.failed_total),
        }
    }

    fn terminal_error_mut(&mut self) -> &mut Option<(ErrorCode, String)> {
        &mut self.terminal_error
    }

    fn finished(&self) -> bool {
        let total = usize::from(self.written_total)
            + usize::from(self.updated_total)
            + usize::from(self.cleared_total)
            + usize::from(self.skipped_total)
            + usize::from(self.failed_total);
        total >= usize::from(self.expected_outcomes)
    }
}

fn route_apply_outcome<F: ApplyFamily>(result: &mut F, body: &F::Body) {
    let outcome = F::outcome_view(body);
    let bucket = match F::error(body).map(|error| error.code) {
        None => OutcomeBucket::Success,
        Some(ErrorCode::VlUnbound) => OutcomeBucket::Skipped,
        Some(code) => {
            let message = F::error(body)
                .map(|error| error.message.as_str().to_string())
                .unwrap_or_default();
            *result.terminal_error_mut() = Some((code, message));
            OutcomeBucket::Failed
        }
    };
    bump_bucket(result.bucket_mut(body, bucket), outcome);
}

fn record_apply_outcome<F: ApplyFamily>(
    detail: &mut OperationDetailState,
    body: &F::Body,
) -> Option<(bool, Option<(ErrorCode, String)>)> {
    let result = F::from_detail(detail)?;
    if result.already_counted(body) {
        return None;
    }
    route_apply_outcome(result, body);
    Some((result.finished(), result.terminal_error_mut().clone()))
}

fn apply_apply_outcome<F: ApplyFamily>(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_c: u64,
    body: &F::Body,
) {
    let Some(op) = guard.active.get(&workflow_c).cloned() else {
        queue_pending_list(F::pending(guard), workflow_c, body.clone(), MAX_PENDING_OUTCOMES);
        return;
    };
    promote_running(publisher, bus_id, guard, counters, workflow_c, &op);
    let Some(slot) = guard.active.get_mut(&workflow_c) else {
        return;
    };
    let Some((finished, terminal_error)) = record_apply_outcome::<F>(&mut slot.detail, body)
    else {
        return;
    };
    let current = slot.clone();
    guard.upsert_http(
        &current.operation_key,
        operation_view_for(&current, current.status, None),
        None,
    );
    if !finished {
        return;
    }
    finish_apply_operation(
        publisher, bus_id, guard, counters, workflow_c, &current, terminal_error,
    );
}

fn flush_pending_apply_outcomes<F: ApplyFamily>(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_c: u64,
) {
    replay_pending(guard, workflow_c, F::pending, |guard, body| {
        apply_apply_outcome::<F>(publisher, bus_id, guard, counters, workflow_c, body);
    });
}

fn finish_apply_operation(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_c: u64,
    current: &TrackedOp,
    terminal_error: Option<(ErrorCode, String)>,
) {
    let (status, error_payload) = match &terminal_error {
        Some((code, message)) => (OperationStatus::Failed, Some((*code, message.as_str()))),
        None => (OperationStatus::Succeeded, None),
    };
    apply_terminal(
        publisher, bus_id, guard, counters, workflow_c, current, status, error_payload,
    );
}

fn scene_action_name(action: SceneProgramAction) -> &'static str {
    match action {
        SceneProgramAction::Write => "write",
        SceneProgramAction::Update => "update",
        SceneProgramAction::Clear => "clear",
    }
}

fn handle_event_frame(
    frame: &BusFrame,
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
) {
    let Some(ev) = frame.event_for(bus_id) else {
        return;
    };
    let meta = &ev.meta;
    let workflow_c = meta.correlation_id;
    dispatch_tracker_event(&ev.payload, publisher, bus_id, state, counters, workflow_c, meta.origin);
}

dali2rust_contracts::dispatch_bus_events! {
    pub const OPERATION_TRACKER_HANDLED_EVENTS;
    fn dispatch_tracker_event(
        payload: &BusEventPayload,
        publisher: &BusPublisher,
        bus_id: BusId,
        state: &Arc<Mutex<OperationTrackerInner>>,
        counters: &Arc<OperationTrackerCounters>,
        workflow_c: u64,
        origin: Origin,
    );
    payload = payload;
    ignored = { counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    DaliGroupMembershipProgrammedEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_apply_outcome::<GroupApplyAccumulator>(
            publisher, bus_id, &mut guard, counters.as_ref(), workflow_c, body,
        );
    },
    DaliSceneProgrammedEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_apply_outcome::<SceneApplyAccumulator>(
            publisher, bus_id, &mut guard, counters.as_ref(), workflow_c, body,
        );
    },
    DaliAttributeReadOutcomesEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_attribute_read_outcomes(&mut guard, workflow_c, body, origin);
    },
    DaliDeviceIdentifiedEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_identify_outcome(&mut guard, workflow_c, body);
    },
    DaliAddressingCompletedEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_address_change_outcome(&mut guard, workflow_c, body);
    },
    HomeAssistantDiscoveryPublishedEvent(body) => {
        let mut guard = lock_tracker(state);
        if let Some(op) = guard.active.get_mut(&workflow_c) {
            op.detail = OperationDetailState::HaDiscovery(
                dali2rust_domain::registry::OperationHaDiscoveryResultView {
                    entities_published: body.entities_published,
                    entities_failed: body.entities_failed,
                },
            );
        }
    },
    DaliDeviceReplacedEvent(body) => {
        let mut guard = lock_tracker(state);
        apply_replace_device_outcome(&mut guard, workflow_c, body);
    },
    OperationWorkerSignalEvent(sig) => handle_worker_signal(
        publisher,
        bus_id,
        state,
        counters,
        workflow_c,
        sig,
        origin,
    ),
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
fn handle_worker_signal(
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
    workflow_c: u64,
    sig: &dali2rust_contracts::msg::OperationWorkerSignalEvent,
    origin: Origin,
) {
    let mut guard = lock_tracker(state);
    apply_worker_signal(publisher, bus_id, &mut guard, counters, workflow_c, sig, origin);
}

fn pending_worker_signals_mut(
    inner: &mut OperationTrackerInner,
) -> &mut PendingMap<Vec<dali2rust_contracts::msg::OperationWorkerSignalEvent>> {
    &mut inner.pending_worker_signals
}

fn flush_pending_worker_signals(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &Arc<OperationTrackerCounters>,
    workflow_c: u64,
) {
    replay_pending(guard, workflow_c, pending_worker_signals_mut, |guard, sig| {
        apply_worker_signal(publisher, bus_id, guard, counters, workflow_c, sig, Origin::Internal);
    });
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
fn apply_worker_signal(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &Arc<OperationTrackerCounters>,
    workflow_c: u64,
    sig: &dali2rust_contracts::msg::OperationWorkerSignalEvent,
    origin: Origin,
) {
    let Some(op) = guard.active.get(&workflow_c).cloned() else {
        if !origin.is_background() {
            let signals = pending_worker_signals_mut(guard);
            queue_pending_list(signals, workflow_c, sig.clone(), MAX_PENDING_SIGNALS);
        }
        return;
    };
    match sig.signal {
        OperationWorkerSignal::WorkerStarted => {
            promote_running(publisher, bus_id, guard, counters.as_ref(), workflow_c, &op);
        }
        OperationWorkerSignal::WorkerSucceeded => {
            apply_terminal(
                publisher,
                bus_id,
                guard,
                counters.as_ref(),
                workflow_c,
                &op,
                OperationStatus::Succeeded,
                None,
            );
        }
        OperationWorkerSignal::WorkerFailed => {
            apply_worker_failure(publisher, bus_id, guard, counters.as_ref(), workflow_c, &op, sig);
        }
    }
}

fn apply_worker_failure(
    publisher: &BusPublisher,
    bus_id: BusId,
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    workflow_c: u64,
    op: &TrackedOp,
    sig: &dali2rust_contracts::msg::OperationWorkerSignalEvent,
) {
    let error = sig.error.as_ref();
    let code = error.map_or(ErrorCode::OperationFailed, |e| e.code);
    let msg_owned = error.map(|e| e.message.clone()).unwrap_or_default();
    apply_terminal(
        publisher,
        bus_id,
        guard,
        counters,
        workflow_c,
        op,
        OperationStatus::Failed,
        Some((code, msg_owned.as_str())),
    );
}

fn due_operation_correlations(guard: &OperationTrackerInner, now: Instant) -> Vec<u64> {
    guard
        .active
        .iter()
        .filter(|(_, o)| now >= o.deadline)
        .filter(|(_, o)| {
            matches!(
                o.status,
                OperationStatus::Accepted | OperationStatus::Running
            )
        })
        .map(|(c, _)| *c)
        .collect()
}

fn drain_rejected_deliveries(
    conf_rx: &BusSubscriberRx,
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
) {
    while let Ok(frame) = conf_rx.try_recv() {
        let BusFrame::Confirmation(conf) = frame else {
            continue;
        };
        if conf.status != dali2rust_contracts::msg::DeliveryStatus::DeliveryRejected {
            continue;
        }
        let correlation = conf.meta.correlation_id;
        let mut guard = lock_tracker(state);
        let Some(op) = guard.active.get(&correlation).cloned() else {
            continue;
        };
        emit_terminal_with_retention(
            publisher,
            bus_id,
            &mut guard,
            counters.as_ref(),
            &op,
            OperationStatus::Failed,
            ErrorCode::CommandsIngressOverload,
        );
        clear_active_entry(&mut guard, correlation, &op);
    }
}

fn evict_expired_pending_outcomes(
    guard: &mut OperationTrackerInner,
    counters: &OperationTrackerCounters,
    now: Instant,
) {
    let mut expired = 0u32;
    guard.for_each_pending_buffer(|buf| {
        expired = expired.saturating_add(buf.evict_expired(now));
    });
    if expired > 0 {
        counters
            .pending_outcomes_expired
            .fetch_add(expired, Ordering::Relaxed);
    }
}

fn tick_deadlines(
    publisher: &BusPublisher,
    bus_id: BusId,
    state: &Arc<Mutex<OperationTrackerInner>>,
    counters: &Arc<OperationTrackerCounters>,
) {
    let now = Instant::now();
    let mut guard = lock_tracker(state);
    guard.remove_http_if_expired(now);
    evict_expired_pending_outcomes(&mut guard, counters.as_ref(), now);
    let due = due_operation_correlations(&guard, now);
    for c in due {
        let Some(op) = guard.active.get(&c).cloned() else {
            continue;
        };
        emit_terminal_with_retention(
            publisher,
            bus_id,
            &mut guard,
            counters.as_ref(),
            &op,
            OperationStatus::TimedOut,
            ErrorCode::ConfirmationTimeout,
        );
        clear_active_entry(&mut guard, c, &op);
    }
}

pub struct OperationTrackerHttpRead(pub Arc<Mutex<OperationTrackerInner>>);

impl OperationReadPort for OperationTrackerHttpRead {
    fn operation_status(&self, id: &str) -> Option<Cow<'static, str>> {
        self.0
            .lock()
            .ok()?
            .http_by_key
            .get(id)
            .map(|row| row.view.status.clone())
    }

    fn operation_view(&self, id: &str) -> Option<OperationView> {
        self.0
            .lock()
            .ok()?
            .http_by_key
            .get(id)
            .map(|row| row.view.clone())
    }

    fn list_operation_keys(&self) -> Vec<String> {
        self.0
            .lock()
            .ok()
            .map(|g| g.http_by_key.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn has_active_operation(&self, operation_type: OperationType, adapter_id: u8) -> bool {
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        let adapter_prefix = adapter_scoped_key_prefix(operation_type, adapter_id);
        guard.active.values().any(|op| {
            op.op_type == operation_type
                && matches!(
                    op.status,
                    OperationStatus::Accepted | OperationStatus::Running
                )
                && match &adapter_prefix {
                    Some(prefix) => op.operation_key.starts_with(prefix.as_str()),
                    None => op.adapter_id == u32::from(adapter_id),
                }
        })
    }
}

fn adapter_scoped_key_prefix(operation_type: OperationType, adapter_id: u8) -> Option<String> {
    let family = match operation_type {
        OperationType::GroupApply => "grp-apply",
        OperationType::SceneApply => "scn-apply",
        OperationType::CommissioningIdentify => "comm-ident",
        OperationType::CommissioningAddressChange => "comm-addr",
        OperationType::CommissioningReplaceDevice => "comm-repl",
        _ => return None,
    };
    Some(format!("{family}-{adapter_id}-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_tracker_recovers_and_heals_http_reader() {
        let state = Arc::new(Mutex::new(OperationTrackerInner::new()));

        let poisoner = Arc::clone(&state);
        let handle = std::thread::spawn(move || {
            let _g = poisoner.lock().expect("operation tracker state");
            panic!("poison the tracker state");
        });
        assert!(handle.join().is_err(), "holder thread must panic");
        assert!(state.lock().is_err(), "mutex must be poisoned");

        {
            let mut g = lock_tracker(&state);
            g.http_by_key.insert(
                "op-1".to_string(),
                HttpOpRow {
                    view: OperationView {
                        operation_id: "op-1".to_string(),
                        operation_type: "discovery".into(),
                        status: "running".into(),
                        error: None,
                        result: None,
                        attribute_read_outcomes: None,
                    },
                    expires_after: None,
                },
            );
        }

        let reader = OperationTrackerHttpRead(Arc::clone(&state));
        assert_eq!(reader.operation_status("op-1").as_deref(), Some("running"));
    }
}
