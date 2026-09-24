use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx};
use dali2rust_contracts::bus::{command_envelope, event_envelope};
use dali2rust_contracts::msg::{
    CommandEnvelope, ErrorCode, EventEnvelope, OperationStatus, OperationType,
    OperationWorkerSignalEvent, Origin, OPERATION_TTL_MS,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::OperationReadPort;
use log::warn;

const CELL_OUTCOME_TIMEOUT_MS: u64 = 5_000;
const SKIP_BATCH: usize = 8;
const INGRESS_BACKOFF_MS: [u64; 10] = [0, 25, 50, 100, 150, 200, 250, 250, 250, 250];

#[derive(Debug, Default)]
pub struct ApplyOrchestratorCounters {
    pub runs_started: AtomicU32,
    pub cells_published: AtomicU32,
    pub skips_published: AtomicU32,
    pub cell_retries: AtomicU32,
    pub outcome_timeouts: AtomicU32,
    pub ingress_backoffs: AtomicU32,
    pub runs_aborted: AtomicU32,
    pub ignored_commands: AtomicU32,
    pub terminal_signal_publish_failed: AtomicU32,
}

pub(crate) trait PacedCell: Copy {
    const OUTCOME_EVENT: &'static str;

    fn is_bound(&self) -> bool;
    fn program_command(&self, run: &RunState<'_>) -> CommandEnvelope;
    fn synthetic_outcome(
        &self,
        run: &RunState<'_>,
        code: ErrorCode,
        message: &str,
    ) -> EventEnvelope;
    fn match_outcome(&self, ev: &EventEnvelope) -> Option<Option<(ErrorCode, String)>>;
    fn describe(&self) -> String;
}

pub(crate) enum CellWait {
    Done { hard_error: Option<(ErrorCode, String)> },
    TimedOut,
    Aborted,
}

enum WaitStep {
    Continue,
    Done,
}

pub(crate) struct RunState<'a> {
    pub ev_rx: &'a BusSubscriberRx,
    pub publisher: &'a BusPublisher,
    pub bus_id: BusId,
    pub counters: &'a ApplyOrchestratorCounters,
    pub workflow: u64,
    pub registry_adapter_id: u8,
    pub operation_key: &'a str,
    pub op_type: OperationType,
    pub operations: &'a dyn OperationReadPort,
    deadline: Instant,
}

impl<'a> RunState<'a> {
    #[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-arm signature")]
    pub(crate) fn begin(
        ev_rx: &'a BusSubscriberRx,
        publisher: &'a BusPublisher,
        bus_id: BusId,
        counters: &'a ApplyOrchestratorCounters,
        workflow: u64,
        registry_adapter_id: u8,
        operation_key: &'a str,
        op_type: OperationType,
        operations: &'a dyn OperationReadPort,
    ) -> Self {
        Self {
            ev_rx,
            publisher,
            bus_id,
            counters,
            workflow,
            registry_adapter_id,
            operation_key,
            op_type,
            operations,
            deadline: Instant::now() + Duration::from_millis(u64::from(OPERATION_TTL_MS)),
        }
    }

    fn operation_is_terminal(&self) -> bool {
        self.operations
            .operation_status(self.operation_key)
            .is_some_and(|status| {
                OperationStatus::from_rest_name(status.as_ref())
                    .is_some_and(OperationStatus::is_terminal)
            })
    }
}

pub(crate) fn execute_apply<C: PacedCell>(run: &RunState<'_>, diff: Option<Vec<C>>) {
    let Some(diff) = diff else {
        publish_begin(run, 0);
        publish_terminal_signal(run, Some((ErrorCode::NotFound, "adapter_not_found".to_string())));
        return;
    };
    if !publish_begin(run, diff.len()) {
        warn!(
            "apply-orchestrator: OperationBegin publish failed for {}",
            run.operation_key
        );
        return;
    }
    if let Some(hard_error) = pace_cells(run, &diff) {
        publish_terminal_signal(run, hard_error);
    }
}

fn pace_cells<C: PacedCell>(run: &RunState<'_>, diff: &[C]) -> Option<Option<(ErrorCode, String)>> {
    debug_assert!(
        super::apply_orchestrator_worker::APPLY_ORCHESTRATOR_HANDLED_EVENTS
            .contains(&C::OUTCOME_EVENT),
        "a paced family must be routed its own outcome kind (ADR-015)",
    );
    let mut hard_error: Option<(ErrorCode, String)> = None;
    let mut i = 0;
    while i < diff.len() {
        let (wait, advance) = if diff[i].is_bound() {
            (pace_one_cell(run, diff[i]), 1)
        } else {
            let batch_len = diff[i..]
                .iter()
                .take_while(|cell| !cell.is_bound())
                .count()
                .min(SKIP_BATCH);
            (pace_skip_batch(run, &diff[i..i + batch_len]), batch_len)
        };
        match wait {
            CellWait::Done { hard_error: err } => {
                if hard_error.is_none() {
                    hard_error = err;
                }
                i += advance;
            }
            CellWait::Aborted => {
                run.counters.runs_aborted.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            CellWait::TimedOut => {
                i += advance;
            }
        }
    }
    Some(hard_error)
}

fn pace_skip_batch<C: PacedCell>(run: &RunState<'_>, cells: &[C]) -> CellWait {
    for cell in cells {
        run.counters.skips_published.fetch_add(1, Ordering::Relaxed);
        let skipped = cell.synthetic_outcome(run, ErrorCode::VlUnbound, "vl_unbound");
        if !publish_with_backoff(run, BusChannel::Events, BusFrame::event(skipped)) {
            return CellWait::Done {
                hard_error: Some((
                    ErrorCode::CommandsIngressOverload,
                    "skip_ingress_overload".to_string(),
                )),
            };
        }
    }
    wait_skip_loopbacks(run, cells)
}

fn wait_skip_loopbacks<C: PacedCell>(run: &RunState<'_>, cells: &[C]) -> CellWait {
    let mut seen = vec![false; cells.len()];
    let mut remaining_cells = cells.len();
    if remaining_cells == 0 {
        return CellWait::Done { hard_error: None };
    }
    let wait_deadline = Instant::now() + Duration::from_millis(CELL_OUTCOME_TIMEOUT_MS);
    let wait = pump_events_until(run, wait_deadline, |ev| {
        if ev.meta.correlation_id != run.workflow {
            return WaitStep::Continue;
        }
        for (idx, cell) in cells.iter().enumerate() {
            if !seen[idx] && cell.match_outcome(ev).is_some() {
                seen[idx] = true;
                remaining_cells -= 1;
                break;
            }
        }
        if remaining_cells == 0 {
            WaitStep::Done
        } else {
            WaitStep::Continue
        }
    });
    match wait {
        CellWait::TimedOut => {
            run.counters.outcome_timeouts.fetch_add(1, Ordering::Relaxed);
            warn!(
                "apply-orchestrator: {remaining_cells} skip loopbacks not observed before timeout"
            );
            CellWait::Done { hard_error: None }
        }
        other => other,
    }
}

fn pace_one_cell<C: PacedCell>(run: &RunState<'_>, cell: C) -> CellWait {
    for attempt in 0..2 {
        if attempt > 0 {
            run.counters.cell_retries.fetch_add(1, Ordering::Relaxed);
        }
        let command = cell.program_command(run);
        run.counters.cells_published.fetch_add(1, Ordering::Relaxed);
        if !publish_with_backoff(run, BusChannel::Commands, BusFrame::command(command)) {
            return synthesize_failed_outcome(run, cell, "cell_ingress_overload");
        }
        match wait_for_cell_outcome(run, cell) {
            CellWait::TimedOut => {
                run.counters.outcome_timeouts.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            done => return done,
        }
    }
    synthesize_failed_outcome(run, cell, "outcome_timeout")
}

fn synthesize_failed_outcome<C: PacedCell>(
    run: &RunState<'_>,
    cell: C,
    message: &'static str,
) -> CellWait {
    let outcome = cell.synthetic_outcome(run, ErrorCode::ExecutionFailed, message);
    if !publish_with_backoff(run, BusChannel::Events, BusFrame::event(outcome)) {
        warn!(
            "apply-orchestrator: failed to publish synthetic outcome for {}",
            cell.describe()
        );
    }
    match wait_for_cell_outcome(run, cell) {
        CellWait::Aborted => CellWait::Aborted,
        _ => CellWait::Done {
            hard_error: Some((ErrorCode::ExecutionFailed, message.to_string())),
        },
    }
}

fn wait_for_cell_outcome<C: PacedCell>(run: &RunState<'_>, cell: C) -> CellWait {
    let mut hard_error: Option<(ErrorCode, String)> = None;
    let wait_deadline = Instant::now() + Duration::from_millis(CELL_OUTCOME_TIMEOUT_MS);
    let wait = pump_events_until(run, wait_deadline, |ev| {
        if ev.meta.correlation_id != run.workflow {
            return WaitStep::Continue;
        }
        let Some(outcome_error) = cell.match_outcome(ev) else {
            return WaitStep::Continue;
        };
        hard_error = outcome_error;
        WaitStep::Done
    });
    match wait {
        CellWait::Done { .. } => CellWait::Done { hard_error },
        other => other,
    }
}

fn pump_events_until(
    run: &RunState<'_>,
    wait_deadline: Instant,
    mut on_event: impl FnMut(&EventEnvelope) -> WaitStep,
) -> CellWait {
    loop {
        if Instant::now() >= run.deadline {
            return CellWait::Aborted;
        }
        if run.operation_is_terminal() {
            return CellWait::Aborted;
        }
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return CellWait::TimedOut;
        }
        let frame = match run.ev_rx.recv_timeout(remaining.min(ABORT_POLL)) {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= wait_deadline {
                    return CellWait::TimedOut;
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return CellWait::Aborted,
        };
        let BusFrame::Event(ev_arc) = frame else {
            continue;
        };
        if matches!(on_event(ev_arc.as_ref()), WaitStep::Done) {
            return CellWait::Done { hard_error: None };
        }
    }
}

const ABORT_POLL: Duration = Duration::from_millis(250);

fn publish_begin(run: &RunState<'_>, expected_outcomes: usize) -> bool {
    let begin = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        run.workflow,
        run.bus_id.0,
        Some(Origin::Internal),
        dali2rust_contracts::msg::OperationBeginCommand::with_defaults(
            run.operation_key,
            run.op_type,
            expected_outcomes.min(usize::from(u16::MAX)) as u16,
        ),
    );
    publish_with_backoff(run, BusChannel::Commands, BusFrame::command(begin))
}

fn publish_terminal_signal(run: &RunState<'_>, hard_error: Option<(ErrorCode, String)>) {
    let signal = match hard_error {
        None => OperationWorkerSignalEvent::succeeded(run.workflow),
        Some((code, message)) => {
            OperationWorkerSignalEvent::failed(run.workflow, code, message.as_str())
        }
    };
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        run.workflow,
        run.bus_id.0,
        Some(Origin::Internal),
        signal,
    );
    if !publish_with_backoff(run, BusChannel::Events, BusFrame::event(ev)) {
        note_terminal_signal_dropped(run.counters, run.operation_key);
    }
}

pub(crate) fn note_terminal_signal_dropped(
    counters: &ApplyOrchestratorCounters,
    operation_key: &str,
) {
    counters
        .terminal_signal_publish_failed
        .fetch_add(1, Ordering::Relaxed);
    warn!("apply-orchestrator: terminal signal publish dropped after backoff for {operation_key}");
}

pub const APPLY_ORCHESTRATOR_REQUIRED_EVENTS: &[&str] = &[
    "OperationWorkerSignalEvent",
    "DaliGroupMembershipProgrammedEvent",
    "DaliSceneProgrammedEvent",
    "DaliAttributesWrittenEvent",
];

fn publish_with_backoff(run: &RunState<'_>, channel: BusChannel, frame: BusFrame) -> bool {
    debug_assert!(
        !matches!(frame, BusFrame::Event(ref ev)
            if !APPLY_ORCHESTRATOR_REQUIRED_EVENTS.contains(&ev.payload.variant_name())),
        "an event published on the required path is missing from \
         APPLY_ORCHESTRATOR_REQUIRED_EVENTS",
    );
    let outcome = dali2rust_bus::publish_required(
        run.publisher,
        channel,
        frame,
        &INGRESS_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "apply-orchestrator",
    );
    if outcome.retries > 0 {
        run.counters
            .ingress_backoffs
            .fetch_add(outcome.retries, Ordering::Relaxed);
    }
    outcome.queued
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_signal_drop_is_counted() {
        let counters = ApplyOrchestratorCounters::default();
        note_terminal_signal_dropped(&counters, "grp-apply-0-1");
        note_terminal_signal_dropped(&counters, "grp-apply-0-1");
        assert_eq!(
            counters
                .terminal_signal_publish_failed
                .load(Ordering::Relaxed),
            2
        );
    }
}
