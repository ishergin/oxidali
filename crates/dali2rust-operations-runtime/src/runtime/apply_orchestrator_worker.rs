use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{BusFrame, BusId, BusPublisher, BusSubscriberRx};
use dali2rust_contracts::bus::{command_envelope, event_envelope};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, CommandEnvelope, CompactErrorPayload, DaliProgramTarget,
    ErrorCode, EventEnvelope, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    collect_group_apply_diff, collect_scene_apply_diff, ApplyReadPort, GroupApplyDiffCell,
    OperationReadPort, SceneApplyDiffRow,
};

use super::apply_pacing::{execute_apply, PacedCell, RunState};
pub use super::apply_pacing::ApplyOrchestratorCounters;

const IDLE_RECV_TIMEOUT: Duration = Duration::from_millis(50);

pub fn spawn_apply_orchestrator_worker(
    cmd_rx: BusSubscriberRx,
    ev_rx: BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    read_port: Arc<dyn ApplyReadPort>,
    operations: Arc<dyn OperationReadPort>,
    counters: Arc<ApplyOrchestratorCounters>,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"apply_orchestrator",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || loop {
            match cmd_rx.recv_timeout(IDLE_RECV_TIMEOUT) {
                Ok(frame) => handle_command_frame(
                    &frame,
                    &ApplyWorker {
                        ev_rx: &ev_rx,
                        publisher: &publisher,
                        bus_id,
                        read_port: read_port.as_ref(),
                        operations: operations.as_ref(),
                        counters: &counters,
                    },
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while ev_rx.try_recv().is_ok() {}
        },
    )
}

pub(crate) struct ApplyWorker<'a> {
    pub(crate) ev_rx: &'a BusSubscriberRx,
    pub(crate) publisher: &'a BusPublisher,
    pub(crate) bus_id: BusId,
    read_port: &'a dyn ApplyReadPort,
    pub(crate) operations: &'a dyn OperationReadPort,
    pub(crate) counters: &'a ApplyOrchestratorCounters,
}

fn handle_command_frame(frame: &BusFrame, cx: &ApplyWorker<'_>) {
    let Some(ce) = frame.command_for(cx.bus_id) else {
        return;
    };
    dispatch_apply_command(&ce.payload, cx, ce.meta.correlation_id);
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const APPLY_ORCHESTRATOR_HANDLED_COMMANDS;
    fn dispatch_apply_command(
        payload: &BusCommandPayload,
        cx: &ApplyWorker<'_>,
        workflow: u64,
    );
    payload = payload;
    ignored = { cx.counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    GroupApplyExecuteCommand(body) => {
        run_apply(
            cx,
            workflow,
            body.registry_adapter_id,
            body.operation_key.as_str(),
            |read_port| {
                read_port
                    .group_apply_snapshot(body.registry_adapter_id)
                    .map(|snapshot| collect_group_apply_diff(&snapshot))
            },
        );
    },
    PolicyApplyExecuteCommand(body) => {
        run_apply(
            cx,
            workflow,
            body.registry_adapter_id,
            body.operation_key.as_str(),
            |read_port| read_port.policy_apply_targets(body.registry_adapter_id),
        );
    },
    SceneApplyExecuteCommand(body) => {
        run_apply(
            cx,
            workflow,
            body.registry_adapter_id,
            body.operation_key.as_str(),
            |read_port| {
                read_port
                    .scene_apply_snapshot(body.registry_adapter_id, body.scene_id)
                    .map(|snapshot| collect_scene_apply_diff(&snapshot))
            },
        );
    },
}

pub const APPLY_ORCHESTRATOR_HANDLED_EVENTS: &[&str] = &[
    "DaliGroupMembershipProgrammedEvent",
    "DaliSceneProgrammedEvent",
    "DaliAttributesWrittenEvent",
];

fn run_apply<C: PacedCell>(
    cx: &ApplyWorker<'_>,
    workflow: u64,
    registry_adapter_id: u8,
    operation_key: &str,
    read_diff: impl FnOnce(&dyn ApplyReadPort) -> Option<Vec<C>>,
) {
    cx.counters.runs_started.fetch_add(1, Ordering::Relaxed);
    let diff = read_diff(cx.read_port);
    while cx.ev_rx.try_recv().is_ok() {}

    let run = RunState::begin(cx, workflow, registry_adapter_id, operation_key);
    execute_apply(&run, diff);
}

impl PacedCell for GroupApplyDiffCell {
    const OUTCOME_EVENT: &'static str = "DaliGroupMembershipProgrammedEvent";
    const SKIP: ErrorCode = ErrorCode::VlUnbound;

    fn is_bound(&self) -> bool {
        self.binding_short.is_some()
    }

    fn program_command(&self, run: &RunState<'_>) -> CommandEnvelope {
        command_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Api),
            dali2rust_contracts::msg::DaliProgramGroupMembershipCommand {
                registry_adapter_id: run.registry_adapter_id,
                target: DaliProgramTarget::VirtualLamp {
                    virtual_lamp_id: self.virtual_lamp_id,
                },
                group_id: self.group_id,
                action: self.action,
            },
        )
    }

    fn synthetic_outcome(
        &self,
        run: &RunState<'_>,
        code: ErrorCode,
        message: &str,
    ) -> EventEnvelope {
        event_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Internal),
            dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent {
                registry_adapter_id: run.registry_adapter_id,
                target: DaliProgramTarget::VirtualLamp {
                    virtual_lamp_id: self.virtual_lamp_id,
                },
                group_id: self.group_id,
                action: self.action,
                physical_short_address: None,
                membership: None,
                error: Some(CompactErrorPayload::new(code, message)),
            },
        )
    }

    fn match_outcome(&self, ev: &EventEnvelope) -> Option<Option<(ErrorCode, String)>> {
        let BusEventPayload::DaliGroupMembershipProgrammedEvent(body) = &ev.payload else {
            return None;
        };
        let matches = body.group_id == self.group_id
            && body.action == self.action
            && matches!(
                body.target,
                DaliProgramTarget::VirtualLamp { virtual_lamp_id }
                    if virtual_lamp_id == self.virtual_lamp_id
            );
        matches.then(|| hard_error_of(body.error.as_ref(), Self::SKIP))
    }

    fn describe(&self) -> String {
        format!("vl{} g{}", self.virtual_lamp_id, self.group_id)
    }
}

impl PacedCell for dali2rust_domain::registry::PolicyApplyCell {
    const OUTCOME_EVENT: &'static str = "DaliAttributesWrittenEvent";
    const SKIP: ErrorCode = ErrorCode::PolicyException;

    fn is_bound(&self) -> bool {
        !self.excepted
    }

    fn program_command(&self, run: &RunState<'_>) -> CommandEnvelope {
        command_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Api),
            dali2rust_contracts::msg::DaliWriteAttributesCommand {
                registry_adapter_id: run.registry_adapter_id,
                short_address: self.short_address,
                system_failure_level: self.system_failure_level,
                power_on_level: self.power_on_level,
                fade_time_ms: None,
                fade_rate: None,
                extended_fade_time_ms: None,
                tc_coolest_mirek: None,
                tc_warmest_mirek: None,
                min_level: None,
                max_level: None,
                dimming_curve: None,
                signals_operation: false,
                known_device_types: self.known_device_types,
                refusal_fails: true,
            },
        )
    }

    fn synthetic_outcome(
        &self,
        run: &RunState<'_>,
        code: ErrorCode,
        message: &str,
    ) -> EventEnvelope {
        event_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Internal),
            dali2rust_contracts::msg::DaliAttributesWrittenEvent {
                short_address: self.short_address,
                registry_adapter_id: run.registry_adapter_id,
                fade_time_ms: None,
                fade_rate: None,
                power_on_level: None,
                system_failure_level: None,
                extended_fade_time_ms: None,
                tc_coolest_mirek: None,
                tc_warmest_mirek: None,
                min_level: None,
                max_level: None,
                dimming_curve: None,
                error: Some(CompactErrorPayload::new(code, message)),
            },
        )
    }

    fn match_outcome(&self, ev: &EventEnvelope) -> Option<Option<(ErrorCode, String)>> {
        let BusEventPayload::DaliAttributesWrittenEvent(body) = &ev.payload else {
            return None;
        };
        (body.short_address == self.short_address)
            .then(|| hard_error_of(body.error.as_ref(), Self::SKIP))
    }

    fn describe(&self) -> String {
        format!("policy a{}", self.short_address)
    }
}

impl PacedCell for SceneApplyDiffRow {
    const OUTCOME_EVENT: &'static str = "DaliSceneProgrammedEvent";
    const SKIP: ErrorCode = ErrorCode::VlUnbound;

    fn is_bound(&self) -> bool {
        self.binding_short.is_some()
    }

    fn program_command(&self, run: &RunState<'_>) -> CommandEnvelope {
        command_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Api),
            dali2rust_contracts::msg::DaliProgramSceneCommand {
                registry_adapter_id: run.registry_adapter_id,
                target: DaliProgramTarget::VirtualLamp {
                    virtual_lamp_id: self.virtual_lamp_id,
                },
                scene_id: self.scene_id,
                action: self.action,
                target_state: self.target_state,
            },
        )
    }

    fn synthetic_outcome(
        &self,
        run: &RunState<'_>,
        code: ErrorCode,
        message: &str,
    ) -> EventEnvelope {
        event_envelope(
            SOURCE_ID_UNSPECIFIED,
            run.workflow,
            run.bus_id.0,
            Some(Origin::Internal),
            dali2rust_contracts::msg::DaliSceneProgrammedEvent {
                registry_adapter_id: run.registry_adapter_id,
                target: DaliProgramTarget::VirtualLamp {
                    virtual_lamp_id: self.virtual_lamp_id,
                },
                scene_id: self.scene_id,
                action: self.action,
                physical_short_address: None,
                target_state: None,
                scene_level: None,
                error: Some(CompactErrorPayload::new(code, message)),
            },
        )
    }

    fn match_outcome(&self, ev: &EventEnvelope) -> Option<Option<(ErrorCode, String)>> {
        let BusEventPayload::DaliSceneProgrammedEvent(body) = &ev.payload else {
            return None;
        };
        let matches = body.scene_id == self.scene_id
            && body.action == self.action
            && matches!(
                body.target,
                DaliProgramTarget::VirtualLamp { virtual_lamp_id }
                    if virtual_lamp_id == self.virtual_lamp_id
            );
        matches.then(|| hard_error_of(body.error.as_ref(), Self::SKIP))
    }

    fn describe(&self) -> String {
        format!("vl{} s{}", self.virtual_lamp_id, self.scene_id)
    }
}

fn hard_error_of(error: Option<&CompactErrorPayload>, skip: ErrorCode) -> Option<(ErrorCode, String)> {
    match error {
        None => None,
        Some(error) if error.code == skip => None,
        Some(error) => Some((error.code, error.message.as_str().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_paced_family_hears_its_own_outcome() {
        for kind in [
            GroupApplyDiffCell::OUTCOME_EVENT,
            SceneApplyDiffRow::OUTCOME_EVENT,
            dali2rust_domain::registry::PolicyApplyCell::OUTCOME_EVENT,
        ] {
            assert!(
                APPLY_ORCHESTRATOR_HANDLED_EVENTS.contains(&kind),
                "{kind} is a paced outcome and must be routed to this worker",
            );
        }
    }

    const CELL: dali2rust_domain::registry::PolicyApplyCell =
        dali2rust_domain::registry::PolicyApplyCell {
            short_address: 4,
            system_failure_level: None,
            power_on_level: Some(200),
            known_device_types: None,
            excepted: false,
        };

    fn written(power_on_level: Option<u8>, error: Option<CompactErrorPayload>) -> EventEnvelope {
        event_envelope(
            SOURCE_ID_UNSPECIFIED,
            1,
            BusId::default().0,
            Some(Origin::Internal),
            dali2rust_contracts::msg::DaliAttributesWrittenEvent {
                short_address: CELL.short_address,
                registry_adapter_id: 0,
                fade_time_ms: None,
                fade_rate: None,
                power_on_level,
                system_failure_level: None,
                extended_fade_time_ms: None,
                tc_coolest_mirek: None,
                tc_warmest_mirek: None,
                min_level: None,
                max_level: None,
                dimming_curve: None,
                error,
            },
        )
    }

    #[test]
    fn a_policy_cell_ends_on_what_its_written_event_says() {
        assert_eq!(CELL.match_outcome(&written(Some(200), None)), Some(None));
        let refused = CompactErrorPayload::new(ErrorCode::VerifyFailed, "verify_failed");
        assert_eq!(
            CELL.match_outcome(&written(None, Some(refused))),
            Some(Some((ErrorCode::VerifyFailed, "verify_failed".to_string()))),
            "the worker that saw the read-back names the refusal"
        );
        let skipped = CompactErrorPayload::new(ErrorCode::PolicyException, "policy_exception");
        assert_eq!(CELL.match_outcome(&written(None, Some(skipped))), Some(None));
        let unanswered = CompactErrorPayload::new(ErrorCode::VerifyUnanswered, "verify_unanswered");
        assert_eq!(
            CELL.match_outcome(&written(None, Some(unanswered))),
            Some(Some((ErrorCode::VerifyUnanswered, "verify_unanswered".to_string()))),
            "silence keeps its own name"
        );
    }
}
