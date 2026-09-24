use super::*;

fn resolve_color_write_policy(
    read_port: &dyn RegistryReadPort,
    registry_adapter_id: u8,
    short_address: u8,
) -> ColorWritePolicy {
    let observed = read_port.physical_dt8_gear_features(registry_adapter_id, short_address);
    let clear = observed.is_some_and(|byte| !gear_features_automatic_activation(byte));
    let auto_activation = if clear
        && read_port.physical_dt8_auto_activation_repair_allowed(registry_adapter_id, short_address)
    {
        RepairAutoActivation::Yes
    } else {
        RepairAutoActivation::No
    };
    let rgbwaf_control =
        if read_port.physical_dt8_rgbwaf_control_assert_allowed(registry_adapter_id, short_address)
        {
            AssertRgbwafControl::Yes
        } else {
            AssertRgbwafControl::No
        };
    ColorWritePolicy { auto_activation, rgbwaf_control }
}

#[allow(clippy::too_many_arguments, reason = "origin threads provenance through one extra param")]
pub(super) fn handle_set_target_state(
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    ts: &dali2rust_contracts::msg::DaliSetTargetStateCommand,
    counters: &DaliWorkerCounters,
) {
    let sequence_retries = runtime_config.target_state_sequence_retries;
    let source = RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api);

    match ts.scope {
        DaliTargetScope::VirtualLamp => handle_set_target_state_vl(
            controller,
            sequence_retries,
            read_port,
            publisher,
            adapter_id,
            correlation_id,
            source,
            ts.registry_adapter_id,
            ts.virtual_lamp_id,
            &ts.setpoint,
            counters,
        ),
        _ => {
            let policy = if ts.scope == DaliTargetScope::Short {
                resolve_color_write_policy(read_port, ts.registry_adapter_id, ts.short_address)
            } else {
                ColorWritePolicy::NONE
            };
            run_set_target_state_physical(
                controller,
                sequence_retries,
                publisher,
                adapter_id,
                correlation_id,
                source,
                ts,
                policy,
                counters,
            )
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "source threads provenance through one extra param")]
fn run_set_target_state_physical(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    source: RuntimeSource,
    ts: &dali2rust_contracts::msg::DaliSetTargetStateCommand,
    policy: ColorWritePolicy,
    counters: &DaliWorkerCounters,
) {
    match ts.scope {
        DaliTargetScope::Short => handle_set_target_state_short(
            controller,
            sequence_retries,
            publisher,
            adapter_id,
            correlation_id,
            source,
            ts.registry_adapter_id,
            ts.short_address,
            &ts.setpoint,
            policy,
            counters,
        ),
        DaliTargetScope::Group => handle_set_target_state_group(
            controller,
            sequence_retries,
            publisher,
            adapter_id,
            correlation_id,
            source,
            ts.registry_adapter_id,
            ts.group_id,
            &ts.setpoint,
            counters,
        ),
        DaliTargetScope::Broadcast => handle_set_target_state_broadcast(
            controller,
            sequence_retries,
            publisher,
            adapter_id,
            correlation_id,
            source,
            ts.registry_adapter_id,
            &ts.setpoint,
            counters,
        ),
        _ => fail_execution(publisher, correlation_id, counters),
    }
}

#[allow(clippy::too_many_arguments, reason = "one param per addressing field the fact carries")]
fn close_target_state_failure(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
    registry_adapter_id: u8,
    scope: dali2rust_contracts::msg::DaliTargetScope,
    short_address: Option<u8>,
    error: &SemanticDaliError,
) {
    publish_target_state_failed(
        publisher, adapter_id, correlation_id, counters, registry_adapter_id, scope,
        short_address, error,
    );
    counters.execution_failed.fetch_add(1, Ordering::Relaxed);
    emit_execution_failed_with_product_error(
        publisher,
        correlation_id,
        counters,
        error.code(),
        error.message(),
    );
}

fn fail_execution(publisher: &BusPublisher, correlation_id: u64, counters: &DaliWorkerCounters) {
    counters.execution_failed.fetch_add(1, Ordering::Relaxed);
    emit_execution_failed(publisher, correlation_id, counters);
}

fn publish_target_state_failed(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
    registry_adapter_id: u8,
    scope: dali2rust_contracts::msg::DaliTargetScope,
    short_address: Option<u8>,
    error: &SemanticDaliError,
) {
    let event = dali2rust_contracts::msg::DaliTargetStateFailedEvent {
        adapter_id: registry_adapter_id,
        short_address: short_address.unwrap_or(0),
        error: dali2rust_contracts::msg::CompactErrorPayload::new(error.code(), error.message()),
        scope,
        group_id: None,
        virtual_lamp_id: None,
    };
    publish_event_typed(
        publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            adapter_id.0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            event,
        ),
        counters,
    );
}

#[allow(clippy::too_many_arguments, reason = "source threads provenance through one extra param")]
fn handle_set_target_state_short(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    source: RuntimeSource,
    registry_adapter_id: u8,
    short_address: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    policy: ColorWritePolicy,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_short_target_state(controller, short_address, sp, policy)
    }) {
        publish_target_state_failed(
            publisher, adapter_id, correlation_id, counters, registry_adapter_id,
            dali2rust_contracts::msg::DaliTargetScope::Short, Some(short_address), &error,
        );
        counters.execution_failed.fetch_add(1, Ordering::Relaxed);
        emit_execution_failed(publisher, correlation_id, counters);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Short,
        None,
        Some(short_address),
        None,
        sp,
        source,
    );
    if !publish_target_state_applied_event(publisher, correlation_id, adapter_id, counters, applied)
    {
        emit_execution_failed(publisher, correlation_id, counters);
        return;
    }
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
}

pub(super) fn target_state_applied_event(
    registry_adapter_id: u8,
    scope: dali2rust_contracts::msg::DaliTargetScope,
    virtual_lamp_id: Option<u8>,
    short_address: Option<u8>,
    group_id: Option<u8>,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    source: RuntimeSource,
) -> dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
    dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
        registry_adapter_id,
        scope,
        virtual_lamp_id,
        short_address,
        group_id,
        setpoint: sp.clone(),
        dapc_applied: setpoint_dapc_applied(sp),
        source,
        applied_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
    }
}

#[allow(clippy::too_many_arguments, reason = "source threads provenance through one extra param")]
fn handle_set_target_state_broadcast(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    source: RuntimeSource,
    registry_adapter_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_broadcast_target_state(controller, sp)
    }) {
        close_target_state_failure(
            publisher, adapter_id, correlation_id, counters, registry_adapter_id,
            dali2rust_contracts::msg::DaliTargetScope::Broadcast, None, &error,
        );
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Broadcast,
        None,
        None,
        None,
        sp,
        source,
    );
    publish_event_required(
        publisher,
        dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            adapter_id.0,
            Some(dali2rust_contracts::msg::Origin::Internal),
            applied,
        ),
        counters,
        PublishKind::Singleton,
        "target-state-applied-broadcast",
    );
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
    publish_confirmation_ok(publisher, correlation_id, counters);
}

#[allow(clippy::too_many_arguments, reason = "source threads provenance through one extra param")]
fn handle_set_target_state_group(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    source: RuntimeSource,
    registry_adapter_id: u8,
    group_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_group_target_state(controller, group_id, sp)
    }) {
        let event = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliTargetStateFailedEvent { adapter_id: registry_adapter_id, short_address: 0, error: dali2rust_contracts::msg::CompactErrorPayload::new(error.code(), error.message()), scope: dali2rust_contracts::msg::DaliTargetScope::Group, group_id: Some(group_id), virtual_lamp_id: None });
        publish_event_typed(publisher, event, counters);
        counters.execution_failed.fetch_add(1, Ordering::Relaxed);
        emit_execution_failed_with_product_error(
            publisher,
            correlation_id,
            counters,
            error.code(),
            error.message(),
        );
        return;
    }

    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Group,
        None,
        None,
        Some(group_id),
        sp,
        source,
    );
    let applied_event =
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), applied);
    publish_event_required(publisher, applied_event, counters, PublishKind::Singleton, "target-state-applied-group");
    publish_confirmation_ok(publisher, correlation_id, counters);
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "source threads provenance through one extra param")]
fn handle_set_target_state_vl(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    source: RuntimeSource,
    registry_adapter_id: u8,
    vl_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    counters: &DaliWorkerCounters,
) {
    let Some(short_sa) = read_port.virtual_lamp_binding_short(registry_adapter_id, vl_id) else {
        fail_target_state_vl_unbound(
            publisher,
            correlation_id,
            adapter_id,
            registry_adapter_id,
            vl_id,
            counters,
        );
        return;
    };
    let policy = resolve_color_write_policy(read_port, registry_adapter_id, short_sa);
    if apply_with_sequence_retry(sequence_retries, || {
        apply_short_target_state(controller, short_sa, sp, policy)
    })
    .is_err()
    {
        counters.execution_failed.fetch_add(1, Ordering::Relaxed);
        emit_execution_failed(publisher, correlation_id, counters);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::VirtualLamp,
        Some(vl_id),
        Some(short_sa),
        None,
        sp,
        source,
    );
    if !publish_target_state_applied_event(publisher, correlation_id, adapter_id, counters, applied)
    {
        emit_execution_failed(publisher, correlation_id, counters);
        return;
    }
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn fail_target_state_vl_unbound(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    registry_adapter_id: u8,
    vl_id: u8,
    counters: &DaliWorkerCounters,
) {
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliTargetStateFailedEvent { adapter_id: registry_adapter_id, short_address: 0, error: dali2rust_contracts::msg::CompactErrorPayload::new(ErrorCode::VlUnbound, "vl_unbound"), scope: dali2rust_contracts::msg::DaliTargetScope::VirtualLamp, group_id: None, virtual_lamp_id: Some(vl_id) });
    publish_event_typed(publisher, ev, counters);
    emit_execution_failed_with_product_error(
        publisher,
        correlation_id,
        counters,
        ErrorCode::VlUnbound,
        "vl_unbound",
    );
}

fn setpoint_dapc_applied(sp: &LightSetpoint) -> bool {
    sp.power != dali2rust_contracts::msg::PowerState::Off && sp.level > 0
}

fn publish_target_state_applied_event(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
    body: dali2rust_contracts::msg::DaliTargetStateAppliedEvent,
) -> bool {
    let env = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), body);
    publish_event_required(publisher, env, counters, PublishKind::Singleton, "target-state-applied")
}
