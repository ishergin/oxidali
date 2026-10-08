use super::*;

#[derive(Clone, Copy)]
pub(super) struct Provenance {
    pub(super) source: RuntimeSource,
    pub(super) hold_hcl: bool,
}

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
    let provenance = Provenance {
        source: RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api),
        hold_hcl: ts.hold_hcl,
    };

    match ts.scope {
        DaliTargetScope::VirtualLamp => handle_set_target_state_vl(
            controller,
            sequence_retries,
            read_port,
            publisher,
            adapter_id,
            correlation_id,
            provenance,
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
                provenance,
                ts,
                policy,
                counters,
            )
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "provenance threads through one extra param")]
fn run_set_target_state_physical(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    provenance: Provenance,
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
            provenance,
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
            provenance,
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
            provenance,
            ts.registry_adapter_id,
            &ts.setpoint,
            counters,
        ),
        _ => fail_execution(publisher, correlation_id, counters),
    }
}

#[derive(Clone, Copy)]
struct FailedTarget {
    registry_adapter_id: u8,
    scope: DaliTargetScope,
    short_address: Option<u8>,
    group_id: Option<u8>,
    virtual_lamp_id: Option<u8>,
}

impl FailedTarget {
    const fn new(registry_adapter_id: u8, scope: DaliTargetScope) -> Self {
        Self {
            registry_adapter_id,
            scope,
            short_address: None,
            group_id: None,
            virtual_lamp_id: None,
        }
    }
}

fn close_target_state_failure(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
    target: FailedTarget,
    error: &SemanticDaliError,
) {
    publish_target_state_failed(publisher, adapter_id, correlation_id, target, error);
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
    target: FailedTarget,
    error: &SemanticDaliError,
) {
    let event = dali2rust_contracts::msg::DaliTargetStateFailedEvent {
        adapter_id: target.registry_adapter_id,
        short_address: target.short_address.unwrap_or(0),
        error: dali2rust_contracts::msg::CompactErrorPayload::new(error.code(), error.message()),
        scope: target.scope,
        group_id: target.group_id,
        virtual_lamp_id: target.virtual_lamp_id,
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
    );
}

#[allow(clippy::too_many_arguments, reason = "provenance threads through one extra param")]
fn handle_set_target_state_short(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    provenance: Provenance,
    registry_adapter_id: u8,
    short_address: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    policy: ColorWritePolicy,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_short_target_state(controller, short_address, sp, policy)
    }) {
        let target = FailedTarget {
            short_address: Some(short_address),
            ..FailedTarget::new(registry_adapter_id, DaliTargetScope::Short)
        };
        close_target_state_failure(publisher, adapter_id, correlation_id, counters, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Short,
        None,
        Some(short_address),
        None,
        sp,
        provenance,
    );
    close_target_state_applied(publisher, correlation_id, adapter_id, counters, applied);
}

fn close_target_state_applied(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
    applied: dali2rust_contracts::msg::DaliTargetStateAppliedEvent,
) {
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
    provenance: Provenance,
) -> dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
    dali2rust_contracts::msg::DaliTargetStateAppliedEvent {
        registry_adapter_id,
        scope,
        virtual_lamp_id,
        short_address,
        group_id,
        setpoint: sp.clone(),
        dapc_applied: setpoint_dapc_applied(sp),
        source: provenance.source,
        applied_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
        hold_hcl: provenance.hold_hcl,
    }
}

#[allow(clippy::too_many_arguments, reason = "provenance threads through one extra param")]
fn handle_set_target_state_broadcast(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    provenance: Provenance,
    registry_adapter_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_broadcast_target_state(controller, sp)
    }) {
        let target = FailedTarget::new(registry_adapter_id, DaliTargetScope::Broadcast);
        close_target_state_failure(publisher, adapter_id, correlation_id, counters, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Broadcast,
        None,
        None,
        None,
        sp,
        provenance,
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

#[allow(clippy::too_many_arguments, reason = "provenance threads through one extra param")]
fn handle_set_target_state_group(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    provenance: Provenance,
    registry_adapter_id: u8,
    group_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_group_target_state(controller, group_id, sp)
    }) {
        let target = FailedTarget {
            group_id: Some(group_id),
            ..FailedTarget::new(registry_adapter_id, DaliTargetScope::Group)
        };
        close_target_state_failure(publisher, adapter_id, correlation_id, counters, target, &error);
        return;
    }

    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Group,
        None,
        None,
        Some(group_id),
        sp,
        provenance,
    );
    let applied_event =
        dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), applied);
    publish_event_required(publisher, applied_event, counters, PublishKind::Singleton, "target-state-applied-group");
    publish_confirmation_ok(publisher, correlation_id, counters);
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "provenance threads through one extra param")]
fn handle_set_target_state_vl(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    provenance: Provenance,
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
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_short_target_state(controller, short_sa, sp, policy)
    }) {
        let target = FailedTarget {
            short_address: Some(short_sa),
            virtual_lamp_id: Some(vl_id),
            ..FailedTarget::new(registry_adapter_id, DaliTargetScope::VirtualLamp)
        };
        close_target_state_failure(publisher, adapter_id, correlation_id, counters, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::VirtualLamp,
        Some(vl_id),
        Some(short_sa),
        None,
        sp,
        provenance,
    );
    close_target_state_applied(publisher, correlation_id, adapter_id, counters, applied);
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
    publish_event_typed(publisher, ev);
    emit_execution_failed_with_product_error(
        publisher,
        correlation_id,
        counters,
        ErrorCode::VlUnbound,
        "vl_unbound",
    );
}

fn setpoint_dapc_applied(sp: &LightSetpoint) -> bool {
    sp.dapc_level()
        .is_some_and(|level| level > dali2rust_contracts::msg::ARC_POWER_OFF)
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
