use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, PublishResult};
use dali2rust_contracts::bus::{command_envelope, event_envelope};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_contracts::msg::{
    BusEventPayload, Dali103ArbitrationProbeCommand, DaliSettingsUpdateCommand, Origin,
    RedundancyTransitionEvent,
};
use dali2rust_domain::registry::{
    AdapterEnabledReadPort, ApplicationActiveMover, DaliSettingsReadPort, RedundancySettingsReadPort,
};

use super::arbitration::{
    arbitration_step, ArbitrationAction, ArbitrationState, TransitionReason,
};

pub const TRANSITION_LOG_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedTransition {
    pub now_active: bool,
    pub reason: u8,
    pub detected_at_ms: u32,
    pub completed_at_ms: u32,
    pub last_peer_answer_ms: u32,
    pub missed_probes: u8,
}

pub type SharedTransitionLog = Arc<Mutex<Vec<RecordedTransition>>>;

#[derive(Debug, Default)]
pub struct ArbitrationWorkerCounters {
    pub probes_published: AtomicU32,
    pub probes_ingress_rejected: AtomicU32,
    pub probes_owned: AtomicU32,
    pub probes_unowned: AtomicU32,
    pub takeovers: AtomicU32,
    pub stand_downs: AtomicU32,
    pub role_publish_failed: AtomicU32,
    pub ignored_events: AtomicU32,
}

pub struct ArbitrationWorkerDeps {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub registry_adapter_id: u8,
    pub redundancy: Arc<dyn RedundancySettingsReadPort>,
    pub dali: Arc<dyn DaliSettingsReadPort>,
    pub adapters: Arc<dyn AdapterEnabledReadPort>,
    pub counters: Arc<ArbitrationWorkerCounters>,
    pub transitions: SharedTransitionLog,
}

struct WorkerState {
    fsm: ArbitrationState,
    verdict: Option<bool>,
    last_peer_answer_ms: u32,
    missed: u8,
    last_probe_ms: Option<u32>,
    probe_pending: bool,
    handover_sent: bool,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            fsm: ArbitrationState::Disabled,
            verdict: None,
            last_peer_answer_ms: 0,
            missed: 0,
            last_probe_ms: None,
            probe_pending: false,
            handover_sent: false,
        }
    }
}

fn is_active(state: ArbitrationState) -> bool {
    matches!(state, ArbitrationState::Active)
}

pub fn run_turn(state: &mut WorkerStateHandle, deps: &ArbitrationWorkerDeps, now_ms: u32) -> ArbitrationAction {
    let settings = deps.redundancy.redundancy_settings_view();
    let (active, moved_by) = deps.dali.application_active_moved_by();
    let verdict = state.0.verdict.take();
    if verdict == Some(true) {
        state.0.last_peer_answer_ms = now_ms;
    }
    let probe_interval_ms = settings.probe_interval_ms;
    let (next, action) = arbitration_step(state.0.fsm, now_ms, settings, active, verdict);
    state.0.missed = match (next, action) {
        (_, ArbitrationAction::Claim(_)) => state.0.missed.saturating_add(1),
        (ArbitrationState::Passive { missed }, _) => missed,
        _ => 0,
    };
    let prev = state.0.fsm;
    state.0.fsm = next;
    act(state, deps, now_ms, action, probe_interval_ms);
    if matches!(action, ArbitrationAction::Claim(_)) {
        state.0.missed = 0;
    }
    record_followed(state, deps, now_ms, prev, next, action, moved_by);
    action
}

fn record_followed(
    state: &mut WorkerStateHandle,
    deps: &ArbitrationWorkerDeps,
    now_ms: u32,
    prev: ArbitrationState,
    next: ArbitrationState,
    action: ArbitrationAction,
    moved_by: ApplicationActiveMover,
) {
    if matches!(action, ArbitrationAction::Claim(_) | ArbitrationAction::StandDown(_)) {
        return;
    }
    let settled = matches!(prev, ArbitrationState::Listening { .. } | ArbitrationState::Disabled)
        && !matches!(next, ArbitrationState::Listening { .. } | ArbitrationState::Disabled);
    let reason = if settled {
        TransitionReason::Boot
    } else if is_active(prev) == is_active(next) {
        return;
    } else if state.0.handover_sent || moved_by == ApplicationActiveMover::Wire {
        TransitionReason::Handover
    } else {
        TransitionReason::Manual
    };
    state.0.handover_sent = false;
    let record = RecordedTransition {
        now_active: is_active(next),
        reason: reason.code(),
        detected_at_ms: now_ms,
        completed_at_ms: now_ms,
        last_peer_answer_ms: state.0.last_peer_answer_ms,
        missed_probes: 0,
    };
    push_transition(&deps.transitions, record);
    publish_transition(deps, record);
}

fn act(
    state: &mut WorkerStateHandle,
    deps: &ArbitrationWorkerDeps,
    now_ms: u32,
    action: ArbitrationAction,
    probe_interval_ms: u32,
) {
    match action {
        ArbitrationAction::Idle => {}
        ArbitrationAction::Probe => probe_when_due(state, deps, now_ms, probe_interval_ms),
        ArbitrationAction::Claim(reason) => {
            deps.counters.takeovers.fetch_add(1, Ordering::Relaxed);
            set_role(state, deps, now_ms, true, reason);
        }
        ArbitrationAction::StandDown(reason) => {
            deps.counters.stand_downs.fetch_add(1, Ordering::Relaxed);
            set_role(state, deps, now_ms, false, reason);
        }
    }
}

fn probe_when_due(
    state: &mut WorkerStateHandle,
    deps: &ArbitrationWorkerDeps,
    now_ms: u32,
    probe_interval_ms: u32,
) {
    if probe_due(state, now_ms, probe_interval_ms) {
        state.0.probe_pending = false;
        state.0.last_probe_ms = Some(now_ms);
        publish_probe(deps);
    } else {
        state.0.probe_pending = true;
    }
}

fn probe_due(state: &WorkerStateHandle, now_ms: u32, probe_interval_ms: u32) -> bool {
    match state.0.last_probe_ms {
        None => true,
        Some(last) => now_ms.wrapping_sub(last) >= probe_interval_ms,
    }
}

fn wait_ms(state: &WorkerStateHandle, now_ms: u32, probe_interval_ms: u32) -> u32 {
    let interval = probe_interval_ms.max(1);
    if !state.0.probe_pending {
        return interval;
    }
    match state.0.last_probe_ms {
        None => interval,
        Some(last) => interval
            .saturating_sub(now_ms.wrapping_sub(last))
            .max(1),
    }
}

fn publish_probe(deps: &ArbitrationWorkerDeps) {
    if !deps.adapters.adapter_enabled(deps.registry_adapter_id) {
        return;
    }
    let ce = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        CORRELATION_NONE,
        deps.bus_id.0,
        Some(Origin::Poller),
        Dali103ArbitrationProbeCommand {
            registry_adapter_id: deps.registry_adapter_id,
        },
    );
    if deps
        .publisher
        .try_publish(BusChannel::Commands, BusFrame::command(ce))
        == PublishResult::Queued
    {
        deps.counters.probes_published.fetch_add(1, Ordering::Relaxed);
    } else {
        deps.counters
            .probes_ingress_rejected
            .fetch_add(1, Ordering::Relaxed);
    }
}

fn set_role(
    state: &WorkerStateHandle,
    deps: &ArbitrationWorkerDeps,
    detected_at_ms: u32,
    active: bool,
    reason: TransitionReason,
) {
    let ce = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        CORRELATION_NONE,
        deps.bus_id.0,
        Some(Origin::Poller),
        DaliSettingsUpdateCommand {
            patch_mask: DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE,
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: active,
            device_short_address: u8::MAX,
        },
    );
    if deps
        .publisher
        .try_publish(BusChannel::Commands, BusFrame::command(ce))
        != PublishResult::Queued
    {
        deps.counters
            .role_publish_failed
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    let record = RecordedTransition {
        now_active: active,
        reason: reason.code(),
        detected_at_ms,
        completed_at_ms: dali2rust_platform::liveness::monotonic_ms(),
        last_peer_answer_ms: state.0.last_peer_answer_ms,
        missed_probes: state.0.missed,
    };
    push_transition(&deps.transitions, record);
    publish_transition(deps, record);
}

fn push_transition(log: &SharedTransitionLog, record: RecordedTransition) {
    let Ok(mut rows) = log.lock() else {
        return;
    };
    if rows.len() >= TRANSITION_LOG_DEPTH {
        rows.remove(0);
    }
    rows.push(record);
}

fn publish_transition(deps: &ArbitrationWorkerDeps, record: RecordedTransition) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        CORRELATION_NONE,
        deps.bus_id.0,
        Some(Origin::Poller),
        RedundancyTransitionEvent {
            now_active: record.now_active,
            reason: record.reason,
            detected_at_ms: record.detected_at_ms,
            completed_at_ms: record.completed_at_ms,
            last_peer_answer_ms: record.last_peer_answer_ms,
            missed_probes: record.missed_probes,
        },
    );
    let _ = deps
        .publisher
        .try_publish(BusChannel::Events, BusFrame::event(ev));
}

pub struct WorkerStateHandle(WorkerState);

impl WorkerStateHandle {
    #[must_use]
    pub fn new() -> Self {
        Self(WorkerState::new())
    }

    #[must_use]
    pub fn state(&self) -> ArbitrationState {
        self.0.fsm
    }

    pub fn note_verdict(&mut self, owned: bool) {
        self.0.verdict = Some(owned);
    }

    pub fn note_event(&mut self, deps: &ArbitrationWorkerDeps, payload: &BusEventPayload) {
        dispatch_arbitration_event(payload, self, deps, dali2rust_contracts::CORRELATION_NONE);
    }
}

impl Default for WorkerStateHandle {
    fn default() -> Self {
        Self::new()
    }
}

dali2rust_contracts::dispatch_bus_events! {
    pub const ARBITRATION_HANDLED_EVENTS;
    fn dispatch_arbitration_event(
        payload: &BusEventPayload,
        state: &mut WorkerStateHandle,
        deps: &ArbitrationWorkerDeps,
        corr: u64,
    );
    payload = payload;
    ignored = { deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    Dali103ArbitrationProbedEvent(body) => note_probe(state, deps, corr, body),
    Dali103HandoverSentEvent(_body) => { state.0.handover_sent = true; },
    DaliSettingsChangedEvent(_body) => {},
}

fn note_probe(
    state: &mut WorkerStateHandle,
    deps: &ArbitrationWorkerDeps,
    _corr: u64,
    body: &dali2rust_contracts::msg::Dali103ArbitrationProbedEvent,
) {
    let counter = if body.owned {
        &deps.counters.probes_owned
    } else {
        &deps.counters.probes_unowned
    };
    counter.fetch_add(1, Ordering::Relaxed);
    state.note_verdict(body.owned);
}

pub fn spawn_arbitration_worker(
    ev_rx: BusSubscriberRx,
    deps: ArbitrationWorkerDeps,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"arbitration",
        dali2rust_bsp::std_thread_stack::ARBITRATION_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || run_loop(ev_rx, deps),
    )
}

fn run_loop(ev_rx: BusSubscriberRx, deps: ArbitrationWorkerDeps) {
    let mut state = WorkerStateHandle::new();
    loop {
        let interval = deps.redundancy.redundancy_settings_view().probe_interval_ms;
        let wait = std::time::Duration::from_millis(u64::from(wait_ms(
            &state,
            dali2rust_platform::liveness::monotonic_ms(),
            interval,
        )));
        match ev_rx.recv_timeout(wait) {
            Ok(dali2rust_bus::BusFrame::Event(ev)) => {
                dispatch_arbitration_event(&ev.payload, &mut state, &deps, ev.meta.correlation_id);
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        run_turn(&mut state, &deps, dali2rust_platform::liveness::monotonic_ms());
    }
}
