use super::*;

pub(super) fn handle_write_attributes(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    w: &dali2rust_contracts::msg::DaliWriteAttributesCommand,
    counters: &DaliWorkerCounters,
) {
    let workflow = correlation_id;
    if w.signals_operation {
        publish_worker_started(publisher, adapter_id, workflow, Origin::Internal, counters);
    }
    let execution = write_short_attributes(
        controller,
        w.short_address,
        w.fade_time_ms,
        w.fade_rate,
        w.power_on_level,
        w.system_failure_level,
        w.extended_fade_time_ms,
        (w.tc_coolest_mirek, w.tc_warmest_mirek),
        (w.min_level, w.max_level),
        w.dimming_curve,
    );
    let recorded = publish_attributes_written(publisher, adapter_id, correlation_id, w, &execution.confirmed, counters);
    publish_phm_readback(publisher, adapter_id, correlation_id, w, &execution.confirmed, counters);
    if recorded || execution.error.is_some() {
        if w.signals_operation {
            publish_write_attributes_outcome(publisher, adapter_id, workflow, execution.error, counters);
        }
    } else {
        report_lost_evidence(publisher, adapter_id, workflow, w.signals_operation, counters);
    }
    counters
        .semantic_write_attributes_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn report_lost_evidence(
    publisher: &BusPublisher,
    adapter_id: BusId,
    workflow: u64,
    signals_operation: bool,
    counters: &DaliWorkerCounters,
) {
    counters.evidence_publish_failed.fetch_add(1, Ordering::Relaxed);
    counters.execution_failed.fetch_add(1, Ordering::Relaxed);
    if !signals_operation {
        return;
    }
    publish_worker_failed(
        publisher,
        adapter_id,
        workflow,
        Origin::Internal,
        ErrorCode::CommandsIngressOverload,
        "attributes_written_event_lost",
        counters,
    );
    emit_execution_failed(publisher, workflow, counters);
}

fn publish_phm_readback(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    w: &dali2rust_contracts::msg::DaliWriteAttributesCommand,
    confirmed: &crate::runtime::executor::ConfirmedWritableAttributes,
    counters: &DaliWorkerCounters,
) {
    let Some(phm) = confirmed.physical_minimum_readback else {
        return;
    };
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesReadEvent { registry_adapter_id: w.registry_adapter_id, short_address: w.short_address, last_chunk: true, chunk: dali2rust_contracts::msg::DaliAttributeReadChunk::Common102 { version: None, device_type: None, physical_minimum: Some(phm), min_level: None, max_level: None, power_on_level: None, system_failure_level: None, fade_time_ms: None, fade_rate: None, supported_device_types: None, light_source_type: None, light_source_types: None } });
    let _ = publish_event_required(publisher, ev, counters, PublishKind::Series, "phm-readback");
}

fn publish_attributes_written(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    w: &dali2rust_contracts::msg::DaliWriteAttributesCommand,
    confirmed: &crate::runtime::executor::ConfirmedWritableAttributes,
    counters: &DaliWorkerCounters,
) -> bool {
    if confirmed.is_empty() {
        return true;
    }
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliAttributesWrittenEvent { short_address: w.short_address, fade_time_ms: confirmed.fade_time_ms, fade_rate: confirmed.fade_rate, power_on_level: confirmed.power_on_level, system_failure_level: confirmed.system_failure_level, extended_fade_time_ms: confirmed.extended_fade_time_ms, registry_adapter_id: w.registry_adapter_id, tc_coolest_mirek: confirmed.tc_coolest_mirek, tc_warmest_mirek: confirmed.tc_warmest_mirek, min_level: confirmed.min_level, max_level: confirmed.max_level, dimming_curve: confirmed.dimming_curve });
    publish_event_required(publisher, ev, counters, PublishKind::Series, "attributes-written")
}

fn publish_write_attributes_outcome(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    error: Option<SemanticDaliError>,
    counters: &DaliWorkerCounters,
) {
    if let Some(error) = error {
        if error.is_preempted() {
            counters
                .write_attributes_preempted
                .fetch_add(1, Ordering::Relaxed);
        } else {
            counters.execution_failed.fetch_add(1, Ordering::Relaxed);
        }
        publish_worker_failed(
            publisher,
            adapter_id,
            correlation_id,
            Origin::Internal,
            error.code(),
            error.message(),
            counters,
        );
        emit_execution_failed(publisher, correlation_id, counters);
    } else {
        publish_confirmation_ok(publisher, correlation_id, counters);
        publish_worker_succeeded(publisher, adapter_id, correlation_id, Origin::Internal, counters);
    }
}
