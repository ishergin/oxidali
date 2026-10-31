use super::*;

#[derive(Clone, Copy)]
pub(super) struct Provenance {
    pub(super) source: RuntimeSource,
    pub(super) hold_hcl: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Reply<'a> {
    pub(super) publisher: &'a BusPublisher,
    pub(super) adapter_id: BusId,
    pub(super) answer: Answer<'a>,
    pub(super) provenance: Provenance,
    pub(super) counters: &'a DaliWorkerCounters,
}

impl<'a> Reply<'a> {
    fn of(env: &WorkerEnv<'a>, answer: Answer<'a>, origin: Origin, hold_hcl: bool) -> Self {
        let source = RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api);
        Self {
            publisher: env.publisher,
            adapter_id: env.adapter_id,
            answer,
            provenance: Provenance { source, hold_hcl },
            counters: env.counters,
        }
    }
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

pub(super) fn handle_set_target_state(
    controller: &mut impl DaliApplicationController,
    env: &WorkerEnv<'_>,
    answer: Answer<'_>,
    origin: Origin,
    ts: &dali2rust_contracts::msg::DaliSetTargetStateCommand,
) {
    let sequence_retries = env.runtime_config.target_state_sequence_retries;
    let reply = Reply::of(env, answer, origin, ts.hold_hcl);
    let (registry_adapter_id, sp) = (ts.registry_adapter_id, &ts.setpoint);
    match ts.scope {
        DaliTargetScope::VirtualLamp => handle_set_target_state_vl(
            controller,
            sequence_retries,
            env.read_port,
            &reply,
            registry_adapter_id,
            ts.virtual_lamp_id,
            sp,
        ),
        _ => {
            let policy = if ts.scope == DaliTargetScope::Short {
                resolve_color_write_policy(env.read_port, registry_adapter_id, ts.short_address)
            } else {
                ColorWritePolicy::NONE
            };
            run_set_target_state_physical(controller, sequence_retries, &reply, ts, policy)
        }
    }
}

fn run_set_target_state_physical(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    reply: &Reply<'_>,
    ts: &dali2rust_contracts::msg::DaliSetTargetStateCommand,
    policy: ColorWritePolicy,
) {
    let (registry_adapter_id, sp) = (ts.registry_adapter_id, &ts.setpoint);
    match ts.scope {
        DaliTargetScope::Short => handle_set_target_state_short(
            controller,
            sequence_retries,
            reply,
            registry_adapter_id,
            ts.short_address,
            sp,
            policy,
        ),
        DaliTargetScope::Group => handle_set_target_state_group(
            controller,
            sequence_retries,
            reply,
            registry_adapter_id,
            ts.group_id,
            sp,
        ),
        DaliTargetScope::Broadcast => {
            handle_set_target_state_broadcast(controller, sequence_retries, reply, registry_adapter_id, sp)
        }
        _ => reply.answer.failed(reply.publisher, reply.counters, EXECUTION_FAILED),
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

fn close_target_state_failure(reply: &Reply<'_>, target: FailedTarget, error: &SemanticDaliError) {
    let Reply { publisher, adapter_id, answer, counters, .. } = *reply;
    publish_target_state_failed(publisher, adapter_id, answer.correlation_id, target, error);
    answer.ended_by(publisher, counters, *error);
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

fn handle_set_target_state_short(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    reply: &Reply<'_>,
    registry_adapter_id: u8,
    short_address: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
    policy: ColorWritePolicy,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_short_target_state(controller, short_address, sp, policy)
    }) {
        let target = FailedTarget {
            short_address: Some(short_address),
            ..FailedTarget::new(registry_adapter_id, DaliTargetScope::Short)
        };
        close_target_state_failure(reply, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Short,
        None,
        Some(short_address),
        None,
        sp,
        reply.provenance,
    );
    close_target_state_applied(reply, applied);
}

pub(super) fn close_target_state_applied(
    reply: &Reply<'_>,
    applied: dali2rust_contracts::msg::DaliTargetStateAppliedEvent,
) -> bool {
    let Reply { publisher, adapter_id, answer, counters, .. } = *reply;
    let env = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, answer.correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), applied);
    if !publish_closing_fact_or_fail(publisher, env, counters, PublishKind::Singleton, "target-state-applied") {
        answer_each_failed(publisher, answer.riders, counters, EXECUTION_FAILED);
        return false;
    }
    answer_each_ok(publisher, answer.riders, counters);
    counters
        .semantic_set_target_state_handled
        .fetch_add(1, Ordering::Relaxed);
    true
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

fn handle_set_target_state_broadcast(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    reply: &Reply<'_>,
    registry_adapter_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_broadcast_target_state(controller, sp)
    }) {
        let target = FailedTarget::new(registry_adapter_id, DaliTargetScope::Broadcast);
        close_target_state_failure(reply, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Broadcast,
        None,
        None,
        None,
        sp,
        reply.provenance,
    );
    if close_target_state_applied(reply, applied) {
        publish_confirmation_ok(reply.publisher, reply.answer.correlation_id, reply.counters);
    }
}

fn handle_set_target_state_group(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    reply: &Reply<'_>,
    registry_adapter_id: u8,
    group_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
) {
    if let Err(error) = apply_with_sequence_retry(sequence_retries, || {
        apply_group_target_state(controller, group_id, sp)
    }) {
        let target = FailedTarget {
            group_id: Some(group_id),
            ..FailedTarget::new(registry_adapter_id, DaliTargetScope::Group)
        };
        close_target_state_failure(reply, target, &error);
        return;
    }

    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::Group,
        None,
        None,
        Some(group_id),
        sp,
        reply.provenance,
    );
    if close_target_state_applied(reply, applied) {
        publish_confirmation_ok(reply.publisher, reply.answer.correlation_id, reply.counters);
    }
}

fn handle_set_target_state_vl(
    controller: &mut impl DaliApplicationController,
    sequence_retries: u8,
    read_port: &dyn RegistryReadPort,
    reply: &Reply<'_>,
    registry_adapter_id: u8,
    vl_id: u8,
    sp: &dali2rust_contracts::msg::LightSetpoint,
) {
    let Some(short_sa) = read_port.virtual_lamp_binding_short(registry_adapter_id, vl_id) else {
        fail_target_state_vl_unbound(reply, registry_adapter_id, vl_id);
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
        close_target_state_failure(reply, target, &error);
        return;
    }
    let applied = target_state_applied_event(
        registry_adapter_id,
        dali2rust_contracts::msg::DaliTargetScope::VirtualLamp,
        Some(vl_id),
        Some(short_sa),
        None,
        sp,
        reply.provenance,
    );
    close_target_state_applied(reply, applied);
}

fn fail_target_state_vl_unbound(reply: &Reply<'_>, registry_adapter_id: u8, vl_id: u8) {
    let Reply { publisher, adapter_id, answer, counters, .. } = *reply;
    let ev = dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, answer.correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliTargetStateFailedEvent { adapter_id: registry_adapter_id, short_address: 0, error: dali2rust_contracts::msg::CompactErrorPayload::new(ErrorCode::VlUnbound, "vl_unbound"), scope: dali2rust_contracts::msg::DaliTargetScope::VirtualLamp, group_id: None, virtual_lamp_id: Some(vl_id) });
    publish_event_typed(publisher, ev);
    answer.refused(publisher, counters, (ErrorCode::VlUnbound, "vl_unbound"));
}

fn setpoint_dapc_applied(sp: &LightSetpoint) -> bool {
    sp.dapc_level()
        .is_some_and(|level| level > dali2rust_contracts::msg::ARC_POWER_OFF)
}
