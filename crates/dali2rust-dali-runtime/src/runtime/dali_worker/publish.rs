use super::*;

pub const DALI_WORKER_REQUIRED_EVENTS: &[&str] = &[
    "DaliDiscoveryProgressEvent",
    "DaliDiscoveryScanReconciledEvent",
    "DaliAttributeReadOutcomesEvent",
    "DaliAttributesReadEvent",
    "DaliMemoryBankReadEvent",
    "DaliMemoryBankReadAbortedEvent",
    "DaliSceneRecalledEvent",
    "DaliSceneProgrammedEvent",
    "DaliGroupMembershipProgrammedEvent",
    "DaliTargetStateAppliedEvent",
    "DaliAddressingCompletedEvent",
    "DaliDeviceIdentifiedEvent",
    "DaliDeviceReplacedEvent",
    "DaliAttributesWrittenEvent",
    "OperationWorkerSignalEvent",
    "Dali103ScanProgressEvent",
    "Dali103ScanStartedEvent",
    "Dali103InstanceConfiguredEvent",
];

pub(super) fn publish_event_typed(publisher: &BusPublisher, env: EventEnvelope) {
    let result = publisher.try_publish(BusChannel::Events, BusFrame::event(env));
    if result != PublishResult::Queued {
        log::warn!("DALI worker: event publish failed: {result:?}");
    }
}

pub(super) fn publish_event_required(
    publisher: &BusPublisher,
    env: EventEnvelope,
    counters: &DaliWorkerCounters,
    kind: PublishKind,
    label: &str,
) -> bool {
    debug_assert!(
        DALI_WORKER_REQUIRED_EVENTS.contains(&env.payload.variant_name()),
        "{} published on the required path is missing from DALI_WORKER_REQUIRED_EVENTS",
        env.payload.variant_name(),
    );
    let outcome = dali2rust_bus::publish_required(
        publisher,
        BusChannel::Events,
        BusFrame::event(env),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        crate::runtime::required_publish::budget_ms(kind),
        label,
    );
    crate::runtime::required_publish::charge_ms(kind, outcome.slept_ms);
    if outcome.retries > 0 {
        counters
            .event_publish_retried
            .fetch_add(outcome.retries, Ordering::Relaxed);
    }
    if outcome.slept_ms > 0 {
        counters
            .event_publish_backoff_ms
            .fetch_add(outcome.slept_ms, Ordering::Relaxed);
    }
    if !outcome.queued {
        counters
            .event_publish_failed
            .fetch_add(1, Ordering::Relaxed);
    }
    outcome.queued
}

pub(super) fn publish_confirmation_typed(
    publisher: &BusPublisher,
    env: ConfirmationEnvelope,
    counters: &DaliWorkerCounters,
) {
    let result = publisher.try_publish(BusChannel::Confirmations, BusFrame::confirmation(env));
    if result != dali2rust_bus::PublishResult::Queued {
        counters
            .confirmation_publish_failed
            .fetch_add(1, Ordering::Relaxed);
        log::warn!("DALI worker: confirmation publish failed: {result:?}");
    }
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
pub(super) fn publish_operation_worker_signal(
    publisher: &BusPublisher,
    adapter_id: BusId,
    workflow_correlation_id: u64,
    signal: OperationWorkerSignal,
    error_code: Option<ErrorCode>,
    error_message: &str,
    origin: Origin,
    counters: &DaliWorkerCounters,
) {
    let body = match signal {
        OperationWorkerSignal::WorkerStarted => {
            dali2rust_contracts::msg::OperationWorkerSignalEvent::started(workflow_correlation_id)
        }
        OperationWorkerSignal::WorkerSucceeded => {
            dali2rust_contracts::msg::OperationWorkerSignalEvent::succeeded(workflow_correlation_id)
        }
        OperationWorkerSignal::WorkerFailed => {
            dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
                workflow_correlation_id,
                error_code.unwrap_or(ErrorCode::ExecutionFailed),
                error_message,
            )
        }
    };
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow_correlation_id,
        adapter_id.0,
        Some(origin),
        body,
    );
    let kind = match signal {
        OperationWorkerSignal::WorkerStarted => PublishKind::Series,
        OperationWorkerSignal::WorkerSucceeded | OperationWorkerSignal::WorkerFailed => {
            PublishKind::Singleton
        }
    };
    publish_event_required(publisher, ev, counters, kind, "operation-worker-signal");
}

pub(super) fn publish_worker_started(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    origin: Origin,
    counters: &DaliWorkerCounters,
) {
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerStarted,
        None,
        "",
        origin,
        counters,
    );
}

pub(super) fn publish_worker_succeeded(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    origin: Origin,
    counters: &DaliWorkerCounters,
) {
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerSucceeded,
        None,
        "",
        origin,
        counters,
    );
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
pub(super) fn publish_worker_failed(
    publisher: &BusPublisher,
    adapter_id: BusId,
    w: u64,
    origin: Origin,
    code: ErrorCode,
    message: &str,
    counters: &DaliWorkerCounters,
) {
    publish_operation_worker_signal(
        publisher,
        adapter_id,
        w,
        OperationWorkerSignal::WorkerFailed,
        Some(code),
        message,
        origin,
        counters,
    );
}

pub(super) fn publish_confirmation_ok(
    publisher: &BusPublisher,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
) {
    let env =
        build_confirmation_envelope(correlation_id, DeliveryStatus::Ok, 0, SOURCE_ID_UNSPECIFIED);
    publish_confirmation_typed(publisher, env, counters);
}

pub(super) fn emit_execution_failed(
    publisher: &BusPublisher,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
) {
    let ce = build_confirmation_envelope_with_product_error(
        correlation_id,
        DeliveryStatus::ExecutionFailed,
        0,
        SOURCE_ID_UNSPECIFIED,
        Some((ErrorCode::ExecutionFailed, "execution_failed")),
    );
    publish_confirmation_typed(publisher, ce, counters);
}

pub(super) fn emit_execution_failed_with_product_error(
    publisher: &BusPublisher,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
    code: ErrorCode,
    message: &str,
) {
    let ce = build_confirmation_envelope_with_product_error(
        correlation_id,
        DeliveryStatus::ExecutionFailed,
        0,
        SOURCE_ID_UNSPECIFIED,
        Some((code, message)),
    );
    publish_confirmation_typed(publisher, ce, counters);
}
