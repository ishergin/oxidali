use super::*;

use std::sync::atomic::AtomicBool;

use dali2rust_contracts::msg::{Dali103PowerNotificationCommand, Dali103TestFrameCommand};
use dali2rust_domain::dali::dev103::bus_unit::{TestFrame, TestFrameBits};

// IEC 62386-103 §9.9.1
pub(super) fn controller_passive_blocked(
    ce: &CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    bus_id: BusId,
    answer: Answer<'_>,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    if hold_cause(read_port, passive_allowed(&ce.payload)) != Some(HoldCause::Passive) {
        return false;
    }
    counters.tx_suppressed_passive.fetch_add(1, Ordering::Relaxed);
    refuse_every_frame(ce, bus_id, answer, publisher, counters, HoldCause::Passive.message())
}

// IEC 62386-103 §9.9.3
pub(super) fn controller_quiescent_blocked(
    ce: &CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    bus_id: BusId,
    answer: Answer<'_>,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    if hold_cause(read_port, passive_allowed(&ce.payload)) != Some(HoldCause::Quiescent) {
        return false;
    }
    refuse_every_frame(ce, bus_id, answer, publisher, counters, HoldCause::Quiescent.message())
}

// IEC 62386-103 §9.9.1, §9.9.3
fn hold_cause(read_port: &dyn RegistryReadPort, passive_allowed: bool) -> Option<HoldCause> {
    if !passive_allowed && !read_port.application_controller_active() {
        Some(HoldCause::Passive)
    } else if read_port.controller_quiescent() {
        Some(HoldCause::Quiescent)
    } else {
        None
    }
}

pub(super) struct GateHold {
    read_port: Arc<dyn RegistryReadPort>,
    passive_allowed: bool,
}

impl WireHold for GateHold {
    fn held(&self) -> Option<HoldCause> {
        hold_cause(self.read_port.as_ref(), self.passive_allowed)
    }
}

struct OnceStarted {
    read_port: Arc<dyn RegistryReadPort>,
    started: AtomicBool,
}

// IEC 62386-103 §9.9.1, §9.9.3
impl WireHold for OnceStarted {
    fn held(&self) -> Option<HoldCause> {
        let cause = hold_cause(self.read_port.as_ref(), self.started.load(Ordering::Relaxed));
        if cause.is_none() {
            self.started.store(true, Ordering::Relaxed);
        }
        cause
    }
}

pub(super) struct GateHolds {
    read_port: Arc<dyn RegistryReadPort>,
    strict: Arc<dyn WireHold>,
    passive_allowed: Arc<dyn WireHold>,
}

impl GateHolds {
    pub(super) fn new(read_port: &Arc<dyn RegistryReadPort>) -> Self {
        let hold = |passive_allowed| -> Arc<dyn WireHold> {
            Arc::new(GateHold { read_port: Arc::clone(read_port), passive_allowed })
        };
        Self { read_port: Arc::clone(read_port), strict: hold(false), passive_allowed: hold(true) }
    }

    pub(super) fn for_frame(&self, frame: &BusFrame) -> Arc<dyn WireHold> {
        let BusFrame::Command(ce) = frame else {
            return Arc::clone(&self.strict);
        };
        if matches!(ce.payload, BusCommandPayload::Dali103CommissionCommand(_)) {
            let read_port = Arc::clone(&self.read_port);
            return Arc::new(OnceStarted { read_port, started: AtomicBool::new(false) });
        }
        Arc::clone(if passive_allowed(&ce.payload) { &self.passive_allowed } else { &self.strict })
    }
}

fn refuse_every_frame(
    ce: &CommandEnvelope,
    bus_id: BusId,
    answer: Answer<'_>,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
    cause: &'static str,
) -> bool {
    match blocked_refusal(ce, counters) {
        BlockedRefusal::Operation(handled) => {
            if let Some(counter) = handled {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            let correlation_id = answer.correlation_id;
            publish_worker_failed(publisher, bus_id, correlation_id, ce.meta.origin, ErrorCode::Conflict, cause, counters);
            true
        }
        BlockedRefusal::Confirmation | BlockedRefusal::NotGated => {
            answer.refused(publisher, counters, (ErrorCode::Conflict, cause));
            true
        }
    }
}

// DiiA 351 §7
// IEC 62386-103 §9.9.1, §9.12.2, §12.3.16
fn passive_allowed(payload: &BusCommandPayload) -> bool {
    match payload {
        BusCommandPayload::Dali103ArbitrationProbeCommand(_)
        | BusCommandPayload::Dali103PowerNotificationCommand(_) => true,
        BusCommandPayload::Dali103TestFrameCommand(test) => {
            matches!(TestFrame { data: test.data, dtr: test.dtr }.bits(), TestFrameBits::Forward24(_))
        }
        _ => false,
    }
}

pub(super) fn adapter_command_blocked(
    ce: &CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    bus_id: BusId,
    answer: Answer<'_>,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    let Some(registry_adapter_id) = command_registry_adapter_id(ce, bus_id) else {
        return false;
    };
    if read_port.adapter_snapshot(registry_adapter_id).enabled {
        return false;
    }

    match blocked_refusal(ce, counters) {
        BlockedRefusal::Operation(handled) => fail_blocked_semantic_operation(
            publisher,
            bus_id,
            answer.correlation_id,
            ce.meta.origin,
            handled,
            counters,
        ),
        BlockedRefusal::Confirmation => {
            answer.failed(publisher, counters, (ErrorCode::Conflict, "adapter_disabled"));
            true
        }
        BlockedRefusal::NotGated => false,
    }
}


enum BlockedRefusal<'c> {
    Operation(Option<&'c AtomicU32>),
    Confirmation,
    NotGated,
}

fn blocked_refusal<'c>(ce: &CommandEnvelope, counters: &'c DaliWorkerCounters) -> BlockedRefusal<'c> {
    match &ce.payload {
        BusCommandPayload::DaliReadAttributesCommand(_) if ce.meta.origin == Origin::Poller => {
            BlockedRefusal::Confirmation
        }
        BusCommandPayload::DaliDiscoverDevicesCommand(_) => {
            BlockedRefusal::Operation(Some(&counters.semantic_discover_devices_handled))
        }
        BusCommandPayload::DaliReadAttributesCommand(_) => {
            BlockedRefusal::Operation(Some(&counters.semantic_read_attributes_handled))
        }
        BusCommandPayload::DaliReadMemoryBankCommand(_) => {
            BlockedRefusal::Operation(Some(&counters.semantic_read_memory_bank_handled))
        }
        BusCommandPayload::DaliWriteAttributesCommand(_) => {
            BlockedRefusal::Operation(Some(&counters.semantic_write_attributes_handled))
        }
        BusCommandPayload::DaliProgramGroupMembershipCommand(_) => BlockedRefusal::Operation(
            Some(&counters.semantic_program_group_membership_handled),
        ),
        BusCommandPayload::DaliProgramSceneCommand(_) => {
            BlockedRefusal::Operation(Some(&counters.semantic_program_scene_handled))
        }
        payload if is_untallied_operation(payload) => BlockedRefusal::Operation(None),
        payload if is_confirmed_request(payload) => BlockedRefusal::Confirmation,
        _ => BlockedRefusal::NotGated,
    }
}

fn is_confirmed_request(payload: &BusCommandPayload) -> bool {
    matches!(
        payload,
        BusCommandPayload::DaliCommissioningStepCommand(_)
            | BusCommandPayload::DaliCommandPayload(_)
            | BusCommandPayload::DaliSetTargetStateCommand(_)
            | BusCommandPayload::DaliRecallSceneCommand(_)
            | BusCommandPayload::DaliRecallLastActiveLevelCommand(_)
            | BusCommandPayload::DaliStopFadeCommand(_)
            | BusCommandPayload::DaliRecallLevelPresetCommand(_)
            | BusCommandPayload::DaliDimCommand(_)
            | BusCommandPayload::Dali103FeedbackDriveCommand(_)
            | BusCommandPayload::Dali103InstanceActionCommand(_)
            | BusCommandPayload::Dali103HandoverCommand(_)
            | BusCommandPayload::Dali103ArbitrationProbeCommand(_)
            | BusCommandPayload::Dali103TestFrameCommand(_)
            | BusCommandPayload::Dali103PowerNotificationCommand(_)
            | BusCommandPayload::DaliBusHealthProbeCommand(_)
    )
}

fn is_untallied_operation(payload: &BusCommandPayload) -> bool {
    matches!(
        payload,
        BusCommandPayload::DaliIdentifyDeviceCommand(_)
            | BusCommandPayload::DaliWriteMemoryLuminaireCommand(_)
            | BusCommandPayload::DaliReplaceDeviceCommand(_)
            | BusCommandPayload::DaliAddressingCommand(_)
            | BusCommandPayload::Dali103ScanCommand(_)
            | BusCommandPayload::Dali103CommissionCommand(_)
            | BusCommandPayload::Dali103InstanceConfigureCommand(_)
            | BusCommandPayload::Dali103IdentifyCommand(_)
            | BusCommandPayload::Dali103FeedbackConfigureCommand(_)
    )
}

fn fail_blocked_semantic_operation(
    publisher: &BusPublisher,
    bus_id: BusId,
    correlation_id: u64,
    origin: Origin,
    handled: Option<&AtomicU32>,
    counters: &DaliWorkerCounters,
) -> bool {
    publish_operation_worker_signal(
        publisher,
        bus_id,
        correlation_id,
        OperationWorkerSignal::WorkerFailed,
        Some((ErrorCode::Conflict, "adapter_disabled")),
        origin,
        counters,
    );
    if let Some(handled) = handled {
        handled.fetch_add(1, Ordering::Relaxed);
    }
    true
}

fn command_registry_adapter_id(ce: &CommandEnvelope, bus_id: BusId) -> Option<u8> {
    match &ce.payload {
        BusCommandPayload::DaliCommandPayload(_) => u8::try_from(bus_id.0.saturating_sub(1)).ok(),
        payload => control_gear_adapter_id(payload).or_else(|| bus_unit_adapter_id(payload)),
    }
}

fn control_gear_adapter_id(payload: &BusCommandPayload) -> Option<u8> {
    match payload {
        BusCommandPayload::DaliSetTargetStateCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliRecallSceneCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliRecallLastActiveLevelCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliStopFadeCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliRecallLevelPresetCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliDimCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliWriteAttributesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliDiscoverDevicesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReadAttributesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReadMemoryBankCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliWriteMemoryLuminaireCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliProgramGroupMembershipCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliProgramSceneCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliCommissioningStepCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliIdentifyDeviceCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReplaceDeviceCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliAddressingCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliBusHealthProbeCommand(cmd) => Some(cmd.registry_adapter_id),
        _ => None,
    }
}

fn bus_unit_adapter_id(payload: &BusCommandPayload) -> Option<u8> {
    match payload {
        BusCommandPayload::Dali103ScanCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103CommissionCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103InstanceConfigureCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103IdentifyCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103FeedbackConfigureCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103FeedbackDriveCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103InstanceActionCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103ArbitrationProbeCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103HandoverCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::Dali103TestFrameCommand(Dali103TestFrameCommand { registry_adapter_id, .. })
        | BusCommandPayload::Dali103PowerNotificationCommand(Dali103PowerNotificationCommand {
            registry_adapter_id,
            ..
        }) => Some(*registry_adapter_id),
        _ => None,
    }
}
