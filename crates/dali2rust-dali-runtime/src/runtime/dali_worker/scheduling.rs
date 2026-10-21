use super::*;

pub(super) fn run_command_loop(inputs: &mut WorkerInputs<impl DaliApplicationController>) {
    let WorkerInputs { cmd_rx, controller: ctrl, runtime_config, read_port, publisher, adapter_id,
        counters, interactive, correlation } = inputs;
    let (runtime_config, adapter_id) = (*runtime_config, *adapter_id);
    while let Ok(frame) = cmd_rx.recv() {
        let mut batch = vec![Queued::new(frame)];
        while let Some(Queued { frame, riders }) =
            take_next_command(&mut batch, cmd_rx, publisher, adapter_id, counters)
        {
            let scope = frame_scope(&frame, interactive, adapter_id);
            let run = |c: &mut _| {
                process_one(
                    frame,
                    c,
                    runtime_config,
                    read_port.as_ref(),
                    publisher,
                    interactive.as_ref(),
                    adapter_id,
                    counters,
                    correlation.as_ref(),
                    &riders,
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

pub(super) struct Queued {
    frame: BusFrame,
    riders: Vec<u64>,
}

impl Queued {
    fn new(frame: BusFrame) -> Self {
        Self { frame, riders: Vec::new() }
    }
}

fn take_next_command(
    batch: &mut Vec<Queued>,
    cmd_rx: &BusSubscriberRx,
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
) -> Option<Queued> {
    while let Ok(extra) = cmd_rx.try_recv() {
        batch.push(Queued::new(extra));
    }
    supersede_stale_target_state(batch, publisher, adapter_id, counters);
    if batch.is_empty() {
        return None;
    }
    let index = highest_priority_index(batch, adapter_id);
    Some(batch.remove(index))
}

fn highest_priority_index(batch: &[Queued], adapter_id: BusId) -> usize {
    let winner = batch
        .iter()
        .enumerate()
        .min_by_key(|(_, queued)| frame_priority_key(&queued.frame, adapter_id))
        .map_or(0, |(index, _)| index);
    if !writes_lamp_state(&batch[winner].frame, adapter_id) {
        return winner;
    }
    batch
        .iter()
        .position(|queued| writes_lamp_state(&queued.frame, adapter_id))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SupersedeKind {
    TargetState,
    RecallScene { scene_id: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SupersedeKey {
    kind: SupersedeKind,
    adapter: u8,
    scope: u8,
    identity: u8,
    hold_hcl: bool,
}

fn supersede_coalesce_key(frame: &BusFrame, adapter_id: BusId) -> Option<SupersedeKey> {
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
            Some(SupersedeKey {
                kind: SupersedeKind::TargetState,
                adapter: ts.registry_adapter_id,
                scope: ts.scope as u8,
                identity,
                hold_hcl: ts.hold_hcl,
            })
        }
        BusCommandPayload::DaliRecallSceneCommand(rc) => Some(SupersedeKey {
            kind: SupersedeKind::RecallScene { scene_id: rc.scene_id },
            adapter: rc.registry_adapter_id,
            scope: rc.scope as u8,
            identity: recall_identity(rc),
            hold_hcl: rc.hold_hcl,
        }),
        _ => None,
    }
}

fn recall_identity(rc: &dali2rust_contracts::msg::DaliRecallSceneCommand) -> u8 {
    match rc.scope {
        DaliTargetScope::Group => rc.group_id,
        DaliTargetScope::Short => rc.short_address,
        DaliTargetScope::VirtualLamp => rc.virtual_lamp_id,
        DaliTargetScope::Broadcast | DaliTargetScope::AddressRange => 0,
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
    key: Option<SupersedeKey>,
    index: usize,
    last_by_key: &std::collections::HashMap<SupersedeKey, usize>,
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
    batch: &mut [Queued],
    last_by_key: &std::collections::HashMap<SupersedeKey, usize>,
    adapter_id: BusId,
) -> Vec<(usize, usize)> {
    let mut carried = Vec::new();
    for (&key, &survivor) in last_by_key {
        if key.kind != SupersedeKind::TargetState {
            continue;
        }
        let Some(original) = target_state_setpoint(&batch[survivor].frame).cloned() else {
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
            if let Some(folded) = frame_with_setpoint(&batch[survivor].frame, acc) {
                batch[survivor].frame = folded;
            }
        }
    }
    carried
}

fn fold_one_key(
    batch: &[Queued],
    last_by_key: &std::collections::HashMap<SupersedeKey, usize>,
    adapter_id: BusId,
    key: SupersedeKey,
    survivor: usize,
    survivor_setpoint: LightSetpoint,
    carried: &mut Vec<(usize, usize)>,
) -> LightSetpoint {
    let mut acc = survivor_setpoint;
    for index in (0..survivor).rev() {
        let frame = &batch[index].frame;
        let frame_key = supersede_coalesce_key(frame, adapter_id);
        if frame_key != Some(key) {
            if writes_lamp_state(frame, adapter_id) && !is_displaced(frame_key, index, last_by_key)
            {
                break;
            }
            continue;
        }
        if !same_origin(frame, &batch[survivor].frame) {
            continue;
        }
        let Some(earlier) = target_state_setpoint(frame) else {
            continue;
        };
        let mut candidate = earlier.clone();
        candidate.merge_from(&acc);
        if candidate != acc {
            acc = candidate;
            carried.push((index, survivor));
        }
    }
    acc
}

fn supersede_stale_target_state(
    batch: &mut Vec<Queued>,
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
) {
    if batch.len() < 2 {
        return;
    }
    let mut last_by_key: std::collections::HashMap<SupersedeKey, usize> =
        std::collections::HashMap::new();
    for (index, queued) in batch.iter().enumerate() {
        if let Some(key) = supersede_coalesce_key(&queued.frame, adapter_id) {
            last_by_key.insert(key, index);
        }
    }
    let carried = fold_displaced_setpoints(batch, &last_by_key, adapter_id);
    let displaced: Vec<bool> = batch
        .iter()
        .enumerate()
        .map(|(index, queued)| {
            is_displaced(supersede_coalesce_key(&queued.frame, adapter_id), index, &last_by_key)
        })
        .collect();
    let superseded_frames = displaced.iter().filter(|displaced| **displaced).count();
    counters
        .target_state_superseded
        .fetch_add(u32::try_from(superseded_frames).unwrap_or(u32::MAX), Ordering::Relaxed);
    let (kept, superseded) = settle_batch(std::mem::take(batch), &displaced, &carried);
    answer_each_refused(publisher, &superseded, counters, (ErrorCode::Superseded, "superseded"));
    *batch = kept;
}

fn command_correlation(frame: &BusFrame) -> Option<u64> {
    match frame {
        BusFrame::Command(ce) => Some(ce.meta.correlation_id),
        _ => None,
    }
}

fn settle_batch(
    batch: Vec<Queued>,
    displaced: &[bool],
    carried: &[(usize, usize)],
) -> (Vec<Queued>, Vec<u64>) {
    let mut moved = vec![Vec::new(); batch.len()];
    let mut superseded = Vec::new();
    let mut survivors = Vec::with_capacity(batch.len());
    for (index, mut queued) in batch.into_iter().enumerate() {
        if !displaced[index] {
            survivors.push((index, queued));
            continue;
        }
        let Some(correlation_id) = command_correlation(&queued.frame) else {
            continue;
        };
        queued.riders.push(correlation_id);
        match carried.iter().find(|(rider, _)| *rider == index) {
            Some(&(_, survivor)) => moved[survivor].append(&mut queued.riders),
            None => superseded.append(&mut queued.riders),
        }
    }
    let kept = survivors
        .into_iter()
        .map(|(index, mut queued)| {
            queued.riders.append(&mut moved[index]);
            queued
        })
        .collect();
    (kept, superseded)
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::PowerState;

    use super::*;

    const RIDER: u64 = 70;
    const FIRST: u64 = 71;
    const SECOND: u64 = 72;

    fn queued(correlation_id: u64, riders: &[u64]) -> Queued {
        let setpoint = LightSetpoint { power: PowerState::On, level: Some(1), color: None };
        Queued {
            frame: BusFrame::command(dali2rust_contracts::bus::command_envelope(
                SOURCE_ID_UNSPECIFIED,
                correlation_id,
                BusId::default().0,
                None,
                dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(0, 1, &setpoint),
            )),
            riders: riders.to_vec(),
        }
    }

    fn shape(kept: &[Queued]) -> Vec<(Option<u64>, Vec<u64>)> {
        kept.iter()
            .map(|queued| (command_correlation(&queued.frame), queued.riders.clone()))
            .collect()
    }

    #[test]
    fn a_carried_request_rides_with_its_carrier_and_brings_its_own_riders() {
        let batch = vec![queued(FIRST, &[RIDER]), queued(SECOND, &[])];
        let (kept, superseded) = settle_batch(batch, &[true, false], &[(0, 1)]);
        assert_eq!(shape(&kept), vec![(Some(SECOND), vec![RIDER, FIRST])]);
        assert!(superseded.is_empty(), "nothing is answered before the carrier runs");
    }

    #[test]
    fn a_request_superseded_outright_is_answered_with_its_riders() {
        let batch = vec![queued(FIRST, &[RIDER]), queued(SECOND, &[])];
        let (kept, superseded) = settle_batch(batch, &[true, false], &[]);
        assert_eq!(shape(&kept), vec![(Some(SECOND), vec![])]);
        assert_eq!(superseded, vec![RIDER, FIRST]);
    }

    #[test]
    fn a_frame_that_survives_keeps_the_riders_it_already_carries() {
        let batch = vec![queued(FIRST, &[RIDER]), queued(SECOND, &[])];
        let (kept, superseded) = settle_batch(batch, &[false, false], &[]);
        assert_eq!(shape(&kept), vec![(Some(FIRST), vec![RIDER]), (Some(SECOND), vec![])]);
        assert!(superseded.is_empty());
    }
}
