use super::*;

// IEC 62386-103 §9.9.1
pub(super) fn controller_passive_blocked(
    ce: &CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    bus_id: BusId,
    correlation_id: u64,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    if read_port.application_controller_active() || passive_allowed(&ce.payload) {
        return false;
    }
    counters.tx_suppressed_passive.fetch_add(1, Ordering::Relaxed);
    match blocked_refusal(&ce.payload, counters) {
        BlockedRefusal::Operation(handled) => fail_passive_semantic_operation(
            publisher,
            bus_id,
            correlation_id,
            ce.meta.origin,
            handled,
            counters,
        ),
        BlockedRefusal::Confirmation | BlockedRefusal::NotGated => {
            counters.execution_failed.fetch_add(1, Ordering::Relaxed);
            emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                ErrorCode::Conflict,
                "controller_passive",
            );
            true
        }
    }
}

// DiiA 351 §7
fn passive_allowed(payload: &BusCommandPayload) -> bool {
    matches!(payload, BusCommandPayload::Dali103ArbitrationProbeCommand(_))
}

fn fail_passive_semantic_operation(
    publisher: &BusPublisher,
    bus_id: BusId,
    correlation_id: u64,
    origin: Origin,
    handled: Option<&AtomicU32>,
    counters: &DaliWorkerCounters,
) -> bool {
    if let Some(counter) = handled {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    publish_worker_failed(
        publisher,
        bus_id,
        correlation_id,
        origin,
        ErrorCode::Conflict,
        "controller_passive",
        counters,
    );
    true
}

pub(super) fn adapter_command_blocked(
    ce: &CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    bus_id: BusId,
    correlation_id: u64,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    let Some(registry_adapter_id) = command_registry_adapter_id(ce, bus_id) else {
        return false;
    };
    if read_port.adapter_snapshot(registry_adapter_id).enabled {
        return false;
    }

    match blocked_refusal(&ce.payload, counters) {
        BlockedRefusal::Operation(handled) => fail_blocked_semantic_operation(
            publisher,
            bus_id,
            correlation_id,
            ce.meta.origin,
            handled,
            counters,
        ),
        BlockedRefusal::Confirmation => {
            counters.execution_failed.fetch_add(1, Ordering::Relaxed);
            emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                ErrorCode::Conflict,
                "adapter_disabled",
            );
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

fn blocked_refusal<'c>(
    payload: &BusCommandPayload,
    counters: &'c DaliWorkerCounters,
) -> BlockedRefusal<'c> {
    match payload {
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
        BusCommandPayload::DaliIdentifyDeviceCommand(_)
        | BusCommandPayload::DaliReplaceDeviceCommand(_)
        | BusCommandPayload::DaliAddressingCommand(_)
        | BusCommandPayload::Dali103ScanCommand(_)
        | BusCommandPayload::Dali103CommissionCommand(_)
        | BusCommandPayload::Dali103InstanceConfigureCommand(_)
        | BusCommandPayload::Dali103IdentifyCommand(_)
        | BusCommandPayload::Dali103FeedbackConfigureCommand(_) => BlockedRefusal::Operation(None),
        BusCommandPayload::DaliCommissioningStepCommand(_)
        | BusCommandPayload::DaliCommandPayload(_)
        | BusCommandPayload::DaliSetTargetStateCommand(_)
        | BusCommandPayload::DaliRecallSceneCommand(_)
        | BusCommandPayload::DaliRecallLastActiveLevelCommand(_)
        | BusCommandPayload::DaliStopFadeCommand(_)
        | BusCommandPayload::Dali103FeedbackDriveCommand(_) => BlockedRefusal::Confirmation,
        _ => BlockedRefusal::NotGated,
    }
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
        Some(ErrorCode::Conflict),
        "adapter_disabled",
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
        BusCommandPayload::DaliSetTargetStateCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliRecallSceneCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliRecallLastActiveLevelCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliStopFadeCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliWriteAttributesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliDiscoverDevicesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReadAttributesCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReadMemoryBankCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliCommissioningStepCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliIdentifyDeviceCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliReplaceDeviceCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliAddressingCommand(cmd) => Some(cmd.registry_adapter_id),
        BusCommandPayload::DaliCommandPayload(_) => u8::try_from(bus_id.0.saturating_sub(1)).ok(),
        _ => None,
    }
}
