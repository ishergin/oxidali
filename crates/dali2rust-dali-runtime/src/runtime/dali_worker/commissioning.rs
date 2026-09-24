use super::*;

pub(super) fn handle_replace_device(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliReplaceDeviceCommand,
    counters: &DaliWorkerCounters,
) {
    let workflow = correlation_id;
    publish_worker_started(publisher, adapter_id, workflow, Origin::Internal, counters);

    let outcome = replace_device(
        controller,
        cmd.failed_short_address,
        cmd.replacement_short_address,
    );
    let error_code = outcome.err().map(SemanticDaliError::code);
    let handover_ok = error_code.is_none();

    publish_event_required(
        publisher,
        replaced_event(adapter_id, correlation_id, cmd, handover_ok, error_code),
        counters,
        PublishKind::Series,
        "device-replaced",
    );

    match error_code {
        None => publish_worker_succeeded(publisher, adapter_id, workflow, Origin::Internal, counters),
        Some(code) => {
            publish_worker_failed(publisher, adapter_id, workflow, Origin::Internal, code, "replace failed", counters)
        }
    }
}

fn replaced_event(
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliReplaceDeviceCommand,
    handover_ok: bool,
    error_code: Option<dali2rust_contracts::msg::ErrorCode>,
) -> dali2rust_contracts::msg::EventEnvelope {
    dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDeviceReplacedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            failed_short_address: cmd.failed_short_address,
            replacement_short_address: cmd.replacement_short_address,
            restored_metadata_and_overrides: handover_ok && cmd.restore_metadata_and_overrides,
            restored_attributes: handover_ok && cmd.restore_attributes,
            restored_groups: handover_ok && cmd.restore_groups,
            restored_scenes: handover_ok && cmd.restore_scenes,
            operation_key: cmd.operation_key.clone(),
            error: error_code.map(|code| {
                dali2rust_contracts::msg::CompactErrorPayload::new(code, "replace failed")
            }),
        },
    )
}

pub(super) fn handle_commissioning_step(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliCommissioningStepCommand,
    counters: &DaliWorkerCounters,
) {
    match run_commissioning_step(
        controller,
        cmd.step,
        cmd.scope,
        cmd.short_address,
        cmd.search_address,
    ) {
        Ok(outcome) => {
            let env = if outcome.backward_violation {
                build_confirmation_envelope_violation(
                    correlation_id,
                    DeliveryStatus::Ok,
                    SOURCE_ID_UNSPECIFIED,
                )
            } else {
                build_confirmation_envelope(
                    correlation_id,
                    DeliveryStatus::Ok,
                    outcome.backward_frame.unwrap_or(0),
                    SOURCE_ID_UNSPECIFIED,
                )
            };
            publish_confirmation_typed(publisher, env, counters);
        }
        Err(error) => {
            emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                error.code(),
                error.message(),
            );
        }
    }
}

pub(super) fn handle_addressing(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliAddressingCommand,
    counters: &DaliWorkerCounters,
) {
    let workflow = correlation_id;
    publish_worker_started(publisher, adapter_id, workflow, Origin::Internal, counters);

    let outcome = change_short_address(
        controller,
        cmd.short_address,
        cmd.new_short_address,
        cmd.verify_after_program,
    );
    let error = outcome.err();
    let error_code = error.map(SemanticDaliError::code);

    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliAddressingCompletedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            old_short_address: cmd.short_address,
            new_short_address: cmd.new_short_address,
            operation_key: cmd.operation_key.clone(),
            error: error_code.map(|code| {
                dali2rust_contracts::msg::CompactErrorPayload::new(code, "address change failed")
            }),
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "addressing-completed");

    match error_code {
        None => publish_worker_succeeded(publisher, adapter_id, workflow, Origin::Internal, counters),
        Some(code) => {
            publish_worker_failed(publisher, adapter_id, workflow, Origin::Internal, code, "address change failed", counters)
        }
    }
}

pub(super) fn handle_identify_device(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliIdentifyDeviceCommand,
    counters: &DaliWorkerCounters,
) {
    let workflow = correlation_id;
    publish_worker_started(publisher, adapter_id, workflow, Origin::Internal, counters);

    let error = identify_device(controller, cmd.short_address).err();

    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliDeviceIdentifiedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            short_address: cmd.short_address,
            mechanism: dali2rust_contracts::msg::IdentifyMechanism::IdentifyDevice,
            operation_key: cmd.operation_key.clone(),
            error: error.as_ref().map(|e: &SemanticDaliError| {
                dali2rust_contracts::msg::CompactErrorPayload::new(e.code(), e.message())
            }),
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "device-identified");

    match error {
        None => publish_worker_succeeded(publisher, adapter_id, workflow, Origin::Internal, counters),
        Some(e) => {
            publish_worker_failed(publisher, adapter_id, workflow, Origin::Internal, e.code(), e.message(), counters)
        }
    }
}
