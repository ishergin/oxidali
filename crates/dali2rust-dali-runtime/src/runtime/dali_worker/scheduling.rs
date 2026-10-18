use super::*;

pub(super) fn run_command_loop(inputs: &mut WorkerInputs<impl DaliApplicationController>) {
    let WorkerInputs { cmd_rx, controller: ctrl, runtime_config, read_port, publisher, adapter_id,
        counters, interactive, correlation } = inputs;
    let (runtime_config, adapter_id) = (*runtime_config, *adapter_id);
    let mut riders = Riders::default();
    while let Ok(frame) = cmd_rx.recv() {
        let mut batch = vec![frame];
        while let Some(frame) =
            take_next_command(&mut batch, cmd_rx, publisher, adapter_id, counters, &mut riders)
        {
            let scope = frame_scope(&frame, interactive, adapter_id);
            let carried = riders.take(&frame);
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
                    &carried,
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
    riders: &mut Riders,
) -> Option<BusFrame> {
    while let Ok(extra) = cmd_rx.try_recv() {
        batch.push(extra);
    }
    supersede_stale_target_state(batch, publisher, adapter_id, counters, riders);
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
    batch: &mut [BusFrame],
    last_by_key: &std::collections::HashMap<SupersedeKey, usize>,
    adapter_id: BusId,
) -> Vec<(usize, usize)> {
    let mut carried = Vec::new();
    for (&key, &survivor) in last_by_key {
        if key.kind != SupersedeKind::TargetState {
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
    last_by_key: &std::collections::HashMap<SupersedeKey, usize>,
    adapter_id: BusId,
    key: SupersedeKey,
    survivor: usize,
    survivor_setpoint: LightSetpoint,
    carried: &mut Vec<(usize, usize)>,
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
            carried.push((index, survivor));
        }
    }
    acc
}

fn supersede_stale_target_state(
    batch: &mut Vec<BusFrame>,
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
    riders: &mut Riders,
) {
    if batch.len() < 2 {
        return;
    }
    let mut last_by_key: std::collections::HashMap<SupersedeKey, usize> =
        std::collections::HashMap::new();
    for (index, frame) in batch.iter().enumerate() {
        if let Some(key) = supersede_coalesce_key(frame, adapter_id) {
            last_by_key.insert(key, index);
        }
    }
    let carried = fold_displaced_setpoints(batch, &last_by_key, adapter_id);
    let correlations: Vec<Option<u64>> = batch.iter().map(command_correlation).collect();
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
        let Some(correlation_id) = correlations[index] else {
            continue;
        };
        counters
            .target_state_superseded
            .fetch_add(1, Ordering::Relaxed);
        let carrier = carried
            .iter()
            .find(|(rider, _)| *rider == index)
            .and_then(|(_, survivor)| correlations[*survivor]);
        settle_displaced(publisher, counters, riders, correlation_id, carrier);
    }
    *batch = kept;
}

fn command_correlation(frame: &BusFrame) -> Option<u64> {
    match frame {
        BusFrame::Command(ce) => Some(ce.meta.correlation_id),
        _ => None,
    }
}

fn settle_displaced(
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
    riders: &mut Riders,
    displaced: u64,
    carrier: Option<u64>,
) {
    match carrier {
        Some(carrier) => riders.carry(carrier, displaced),
        None => answer_each_refused(
            publisher,
            &riders.release(displaced),
            counters,
            (ErrorCode::Superseded, "superseded"),
        ),
    }
}

#[derive(Default)]
pub(super) struct Riders(std::collections::HashMap<u64, Vec<u64>>);

impl Riders {
    fn take(&mut self, frame: &BusFrame) -> Vec<u64> {
        command_correlation(frame)
            .and_then(|correlation_id| self.0.remove(&correlation_id))
            .unwrap_or_default()
    }

    fn carry(&mut self, carrier: u64, rider: u64) {
        let moved = self.release(rider);
        self.0.entry(carrier).or_default().extend(moved);
    }

    fn release(&mut self, displaced: u64) -> Vec<u64> {
        let mut answered = self.0.remove(&displaced).unwrap_or_default();
        answered.push(displaced);
        answered
    }
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::PowerState;

    use super::*;

    const FIRST: u64 = 71;
    const SECOND: u64 = 72;
    const THIRD: u64 = 73;

    fn frame(correlation_id: u64) -> BusFrame {
        let setpoint = LightSetpoint { power: PowerState::On, level: Some(1), color: None };
        BusFrame::command(dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            BusId::default().0,
            None,
            dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(0, 1, &setpoint),
        ))
    }

    #[test]
    fn a_carried_request_rides_with_its_carrier_until_the_carrier_runs() {
        let mut riders = Riders::default();
        riders.carry(SECOND, FIRST);
        assert_eq!(riders.take(&frame(SECOND)), vec![FIRST]);
        assert!(riders.take(&frame(SECOND)).is_empty(), "a carrier's riders are answered once");
    }

    #[test]
    fn a_carrier_carried_in_a_later_drain_hands_its_riders_on() {
        let mut riders = Riders::default();
        riders.carry(SECOND, FIRST);
        riders.carry(THIRD, SECOND);
        assert!(riders.take(&frame(SECOND)).is_empty());
        assert_eq!(riders.take(&frame(THIRD)), vec![FIRST, SECOND]);
    }

    #[test]
    fn a_carrier_superseded_outright_releases_its_riders_with_it() {
        let mut riders = Riders::default();
        riders.carry(SECOND, FIRST);
        assert_eq!(riders.release(SECOND), vec![FIRST, SECOND]);
        assert!(riders.take(&frame(SECOND)).is_empty());
    }
}
