use super::*;

pub(super) fn run_command_loop(
    cmd_rx: &BusSubscriberRx,
    controller: impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    interactive: &Arc<dali2rust_platform::dali::WireActivity>,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    let mut ctrl = controller;
    while let Ok(frame) = cmd_rx.recv() {
        let mut batch = vec![frame];
        while let Some(frame) = take_next_command(&mut batch, cmd_rx, publisher, adapter_id, counters)
        {
            let scope = frame_scope(&frame, interactive, adapter_id);
            let run = |c: &mut _| {
                process_one(
                    frame,
                    c,
                    runtime_config,
                    read_port,
                    publisher,
                    interactive.as_ref(),
                    adapter_id,
                    counters,
                    correlation,
                )
            };
            crate::runtime::required_publish::with_budget(scope.publish_priority, || {
                ctrl.with_wire_class(scope.class, |c| match scope.lease {
                    Some(lease) => c.with_wire_lease(lease, run),
                    None => run(c),
                });
            });
        }
    }
}

struct FrameScope {
    lease: Option<dali2rust_platform::dali::WireLease>,
    class: Option<dali2rust_domain::dali::ses::TransactionPriority>,
    publish_priority: Option<dali2rust_platform::dali::WirePriority>,
}

fn frame_scope(
    frame: &BusFrame,
    interactive: &Arc<dali2rust_platform::dali::WireActivity>,
    adapter_id: BusId,
) -> FrameScope {
    let BusFrame::Command(ce) = frame else {
        return FrameScope {
            lease: None,
            class: None,
            publish_priority: None,
        };
    };
    FrameScope {
        lease: frame_lease(ce, interactive, adapter_id),
        class: crate::runtime::priority::command_transaction_priority(
            &ce.meta,
            &ce.payload,
            adapter_id,
        ),
        publish_priority: crate::runtime::priority::command_wire_priority(
            &ce.meta,
            &ce.payload,
            adapter_id,
        ),
    }
}

fn frame_lease(
    ce: &dali2rust_contracts::msg::CommandEnvelope,
    interactive: &Arc<dali2rust_platform::dali::WireActivity>,
    adapter_id: BusId,
) -> Option<dali2rust_platform::dali::WireLease> {
    let priority = crate::runtime::priority::command_wire_priority(&ce.meta, &ce.payload, adapter_id)?;
    if priority == WirePriority::Interactive {
        return None;
    }
    let granularity = crate::runtime::priority::yield_granularity(ce.payload.variant_name())?;
    if granularity == YieldGranularity::Never {
        return None;
    }
    Some(dali2rust_platform::dali::WireLease::new(
        Arc::clone(interactive),
        priority,
        granularity,
    ))
}

fn take_next_command(
    batch: &mut Vec<BusFrame>,
    cmd_rx: &BusSubscriberRx,
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
) -> Option<BusFrame> {
    while let Ok(extra) = cmd_rx.try_recv() {
        batch.push(extra);
    }
    supersede_stale_target_state(batch, publisher, adapter_id, counters);
    if batch.is_empty() {
        return None;
    }
    let index = highest_priority_index(batch, adapter_id);
    Some(batch.remove(index))
}

fn highest_priority_index(batch: &[BusFrame], adapter_id: BusId) -> usize {
    let winner = batch
        .iter()
        .enumerate()
        .min_by_key(|(_, frame)| frame_priority_key(frame, adapter_id))
        .map_or(0, |(index, _)| index);
    if !writes_lamp_state(&batch[winner], adapter_id) {
        return winner;
    }
    batch
        .iter()
        .position(|frame| writes_lamp_state(frame, adapter_id))
        .unwrap_or(winner)
}

fn writes_lamp_state(frame: &BusFrame, adapter_id: BusId) -> bool {
    let BusFrame::Command(ce) = frame else {
        return false;
    };
    if BusId(ce.meta.target_adapter_id) != adapter_id {
        return false;
    }
    crate::runtime::priority::drives_lamp_state(ce.payload.variant_name()).unwrap_or(false)
}

fn frame_priority_key(frame: &BusFrame, adapter_id: BusId) -> u8 {
    const NOT_OURS: u8 = u8::MAX;
    let BusFrame::Command(ce) = frame else {
        return NOT_OURS;
    };
    crate::runtime::priority::command_wire_priority(&ce.meta, &ce.payload, adapter_id)
        .map_or(NOT_OURS, |priority| priority as u8)
}

fn supersede_coalesce_key(frame: &BusFrame, adapter_id: BusId) -> Option<(u8, u8, u8, u8, u8)> {
    let BusFrame::Command(ce) = frame else {
        return None;
    };
    if BusId(ce.meta.target_adapter_id) != adapter_id {
        return None;
    }
    match &ce.payload {
        BusCommandPayload::DaliSetTargetStateCommand(ts) => {
            let identity = match ts.scope {
                DaliTargetScope::Short => ts.short_address,
                DaliTargetScope::VirtualLamp => ts.virtual_lamp_id,
                DaliTargetScope::Group => ts.group_id,
                _ => return None,
            };
            Some((0, ts.registry_adapter_id, ts.scope as u8, identity, 0))
        }
        BusCommandPayload::DaliRecallSceneCommand(rc) => Some((
            1,
            rc.registry_adapter_id,
            rc.scope as u8,
            rc.scene_id,
            rc.group_id,
        )),
        _ => None,
    }
}

fn target_state_setpoint(frame: &BusFrame) -> Option<&LightSetpoint> {
    match frame {
        BusFrame::Command(ce) => match &ce.payload {
            BusCommandPayload::DaliSetTargetStateCommand(ts) => Some(&ts.setpoint),
            _ => None,
        },
        _ => None,
    }
}

fn frame_with_setpoint(frame: &BusFrame, setpoint: LightSetpoint) -> Option<BusFrame> {
    let BusFrame::Command(ce) = frame else {
        return None;
    };
    let BusCommandPayload::DaliSetTargetStateCommand(_) = &ce.payload else {
        return None;
    };
    let mut envelope = (**ce).clone();
    let BusCommandPayload::DaliSetTargetStateCommand(ts) = &mut envelope.payload else {
        return None;
    };
    ts.setpoint = setpoint;
    Some(BusFrame::command(envelope))
}

fn is_displaced(
    key: Option<(u8, u8, u8, u8, u8)>,
    index: usize,
    last_by_key: &std::collections::HashMap<(u8, u8, u8, u8, u8), usize>,
) -> bool {
    key.is_some_and(|key| last_by_key.get(&key) != Some(&index))
}

fn same_origin(a: &BusFrame, b: &BusFrame) -> bool {
    match (a, b) {
        (BusFrame::Command(a), BusFrame::Command(b)) => a.meta.origin == b.meta.origin,
        _ => false,
    }
}

fn fold_displaced_setpoints(
    batch: &mut [BusFrame],
    last_by_key: &std::collections::HashMap<(u8, u8, u8, u8, u8), usize>,
    adapter_id: BusId,
) -> Vec<usize> {
    let mut carried = Vec::new();
    for (&key, &survivor) in last_by_key {
        if key.0 != 0 {
            continue;
        }
        let Some(original) = target_state_setpoint(&batch[survivor]).cloned() else {
            continue;
        };
        let acc = fold_one_key(
            batch,
            last_by_key,
            adapter_id,
            key,
            survivor,
            original.clone(),
            &mut carried,
        );
        if acc != original {
            if let Some(folded) = frame_with_setpoint(&batch[survivor], acc) {
                batch[survivor] = folded;
            }
        }
    }
    carried
}

fn fold_one_key(
    batch: &[BusFrame],
    last_by_key: &std::collections::HashMap<(u8, u8, u8, u8, u8), usize>,
    adapter_id: BusId,
    key: (u8, u8, u8, u8, u8),
    survivor: usize,
    survivor_setpoint: LightSetpoint,
    carried: &mut Vec<usize>,
) -> LightSetpoint {
    let mut acc = survivor_setpoint;
    for index in (0..survivor).rev() {
        let frame = &batch[index];
        let frame_key = supersede_coalesce_key(frame, adapter_id);
        if frame_key != Some(key) {
            if writes_lamp_state(frame, adapter_id) && !is_displaced(frame_key, index, last_by_key)
            {
                break;
            }
            continue;
        }
        if !same_origin(frame, &batch[survivor]) {
            continue;
        }
        let Some(earlier) = target_state_setpoint(frame) else {
            continue;
        };
        let mut candidate = earlier.clone();
        candidate.merge_from(&acc);
        if candidate != acc {
            acc = candidate;
            carried.push(index);
        }
    }
    acc
}

fn supersede_stale_target_state(
    batch: &mut Vec<BusFrame>,
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
) {
    if batch.len() < 2 {
        return;
    }
    let mut last_by_key: std::collections::HashMap<(u8, u8, u8, u8, u8), usize> =
        std::collections::HashMap::new();
    for (index, frame) in batch.iter().enumerate() {
        if let Some(key) = supersede_coalesce_key(frame, adapter_id) {
            last_by_key.insert(key, index);
        }
    }
    let carried = fold_displaced_setpoints(batch, &last_by_key, adapter_id);
    let mut kept = Vec::with_capacity(batch.len());
    for (index, frame) in batch.drain(..).enumerate() {
        let displaced = is_displaced(
            supersede_coalesce_key(&frame, adapter_id),
            index,
            &last_by_key,
        );
        if !displaced {
            kept.push(frame);
            continue;
        }
        if let BusFrame::Command(ce) = &frame {
            counters
                .target_state_superseded
                .fetch_add(1, Ordering::Relaxed);
            if carried.contains(&index) {
                publish_confirmation_ok(publisher, ce.meta.correlation_id, counters);
            } else {
                emit_execution_failed_with_product_error(
                    publisher,
                    ce.meta.correlation_id,
                    counters,
                    ErrorCode::Superseded,
                    "superseded",
                );
            }
        }
    }
    *batch = kept;
}
