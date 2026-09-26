use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{
    recv_then_drain, BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, WorkerTurn,
};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    BusEventPayload, DaliAttributeReadChunk,
    DaliAttributeReadOutcomesEvent, DaliObservedFrameEvent,
    DaliSceneRecalledEvent, DaliSceneTargetState, DaliTargetScope, DaliTargetStateAppliedEvent,
    LastDapcSource, LightSetpoint, ObservedKind, Origin, PowerState,
    RegistryLevelTransitionCommand, RegistryRuntimeUpdateCommand,
    RuntimeObservation, RuntimeRegistryUpdateEntry, RuntimeSource,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{
    capability_accepts_color_mode, CapabilityFlagsView, ProjectorReadPort,
};

const IDLE_RECV_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug, Default)]
pub struct ProjectorCounters {
    pub runtime_updates_published: AtomicU32,
    pub runtime_updates_retried: AtomicU32,
    pub group_expansions: AtomicU32,
    pub scene_expansions: AtomicU32,
    pub broadcast_expansions: AtomicU32,
    pub coalesced_observed: AtomicU32,
    pub skipped_unknown_observed: AtomicU32,
    pub skipped_unbound: AtomicU32,
    pub transitions_expanded: AtomicU32,
    pub publish_failed: AtomicU32,
    pub ignored_events: AtomicU32,
}

#[derive(Default)]
struct ObservedCoalescer {
    pending: Option<DaliObservedFrameEvent>,
}

impl ObservedCoalescer {
    fn coalesce_key(body: &DaliObservedFrameEvent) -> (u8, DaliTargetScope, u8) {
        let id = body
            .short_address
            .or(body.group_id)
            .unwrap_or(u8::MAX);
        (body.registry_adapter_id, body.scope, id)
    }

    fn matches(&self, body: &DaliObservedFrameEvent) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|prev| Self::coalesce_key(prev) == Self::coalesce_key(body))
    }

    fn set(&mut self, body: &DaliObservedFrameEvent) {
        self.pending = Some(body.clone());
    }

    fn take(&mut self) -> Option<DaliObservedFrameEvent> {
        self.pending.take()
    }
}

pub fn spawn_projector_worker(
    ev_rx: BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    read_port: Arc<dyn ProjectorReadPort>,
    counters: Arc<ProjectorCounters>,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"fanout_projector_worker",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || {
            let ctx = Ctx {
                publisher: &publisher,
                bus_id,
                read_port: read_port.as_ref(),
                counters: &counters,
            };
            let mut coalescer = ObservedCoalescer::default();
            loop {
                let turn = recv_then_drain(&ev_rx, IDLE_RECV_TIMEOUT, |frame| {
                    handle_event_frame(&frame, &ctx, &mut coalescer);
                });
                match turn {
                    WorkerTurn::Handled => flush_coalesced(&ctx, &mut coalescer),
                    WorkerTurn::Idle => {}
                    WorkerTurn::Disconnected => break,
                }
            }
        },
    )
}

struct Ctx<'a> {
    publisher: &'a BusPublisher,
    bus_id: BusId,
    read_port: &'a dyn ProjectorReadPort,
    counters: &'a Arc<ProjectorCounters>,
}

#[inline(never)]
fn flush_coalesced(ctx: &Ctx<'_>, coalescer: &mut ObservedCoalescer) {
    if let Some(body) = coalescer.take() {
        handle_observed_frame(ctx.publisher, ctx.bus_id, ctx.read_port, ctx.counters, &body);
    }
}

fn handle_event_frame(frame: &BusFrame, ctx: &Ctx<'_>, coalescer: &mut ObservedCoalescer) {
    let Some(ev) = frame.event_for(ctx.bus_id) else {
        return;
    };
    let meta = &ev.meta;
    if let BusEventPayload::DaliObservedFrameEvent(body) = &ev.payload {
        if body.observed_kind == ObservedKind::TargetStateObserved {
            if coalescer.matches(body) {
                ctx.counters
                    .coalesced_observed
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                flush_coalesced(ctx, coalescer);
            }
            coalescer.set(body);
            return;
        }
    }
    flush_coalesced(ctx, coalescer);
    dispatch_projector_event(
        &ev.payload,
        ctx.publisher,
        ctx.bus_id,
        ctx.read_port,
        ctx.counters,
        meta.correlation_id,
    );
}

dali2rust_contracts::dispatch_bus_events! {
    pub const PROJECTOR_HANDLED_EVENTS;
    fn dispatch_projector_event(
        payload: &BusEventPayload,
        publisher: &BusPublisher,
        bus_id: BusId,
        read_port: &dyn ProjectorReadPort,
        counters: &Arc<ProjectorCounters>,
        corr: u64,
    );
    payload = payload;
    ignored = { counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    DaliTargetStateAppliedEvent(body) =>
        handle_target_state_applied(publisher, bus_id, read_port, counters, corr, body),
    DaliSceneRecalledEvent(body) =>
        handle_scene_recalled(publisher, bus_id, read_port, counters, body),
    DaliObservedFrameEvent(body) =>
        handle_observed_frame(publisher, bus_id, read_port, counters, body),
    DaliAttributeReadOutcomesEvent(body) =>
        handle_read_outcomes(publisher, bus_id, counters, body),
    DaliAttributesReadEvent(body) => {
        if let DaliAttributeReadChunk::RuntimeStatus {
            setpoint,
            observation,
            read_started_mono_ms,
        } = &body.chunk
        {
            handle_runtime_status_chunk(
                publisher,
                bus_id,
                counters,
                body.registry_adapter_id,
                body.short_address,
                setpoint.as_ref(),
                observation,
                *read_started_mono_ms,
            );
        }
    },
}

struct RuntimeFact {
    registry_adapter_id: u8,
    virtual_lamp_id: Option<u8>,
    short_address: Option<u8>,
    setpoint: LightSetpoint,
    observation: RuntimeObservation,
    last_dapc_source: Option<LastDapcSource>,
    source: RuntimeSource,
    correlation_id: u64,
    observed_at_mono_ms: Option<u32>,
}

const EXPANSION_BATCH: usize = 8;

const EXPANSION_PAUSE: Duration = Duration::from_millis(15);

const EXPANSION_BUDGET_MS: u32 = 1_525;

struct ExpansionPace {
    remaining_ms: u32,
    since_pause: usize,
}

impl ExpansionPace {
    fn new() -> Self {
        Self {
            remaining_ms: EXPANSION_BUDGET_MS,
            since_pause: 0,
        }
    }

    fn publish(
        &mut self,
        publisher: &BusPublisher,
        bus_id: BusId,
        counters: &Arc<ProjectorCounters>,
        fact: RuntimeFact,
    ) {
        if self.since_pause >= EXPANSION_BATCH {
            self.since_pause = 0;
            // sleep-ok: bounded inter-batch pacing, ADR-007 mass-command policy
            std::thread::sleep(EXPANSION_PAUSE);
        }
        self.since_pause += 1;
        let outcome = publish_required_fact(publisher, bus_id, counters, fact, self.remaining_ms);
        self.remaining_ms = self.remaining_ms.saturating_sub(outcome);
    }

    fn publish_transition(
        &mut self,
        publisher: &BusPublisher,
        bus_id: BusId,
        counters: &Arc<ProjectorCounters>,
        command: RegistryLevelTransitionCommand,
    ) {
        if self.since_pause >= EXPANSION_BATCH {
            self.since_pause = 0;
            // sleep-ok: bounded inter-batch pacing, ADR-007 mass-command policy
            std::thread::sleep(EXPANSION_PAUSE);
        }
        self.since_pause += 1;
        let outcome = dali2rust_bus::publish_required(
            publisher,
            BusChannel::Commands,
            BusFrame::command(command_envelope(
                SOURCE_ID_UNSPECIFIED,
                CORRELATION_NONE,
                bus_id.0,
                Some(Origin::Internal),
                command,
            )),
            &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
            self.remaining_ms,
            "projector-level-transition",
        );
        if outcome.retries > 0 {
            counters
                .runtime_updates_retried
                .fetch_add(outcome.retries, Ordering::Relaxed);
        }
        if outcome.queued {
            counters
                .runtime_updates_published
                .fetch_add(1, Ordering::Relaxed);
        } else {
            counters.publish_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(outcome.slept_ms);
    }
}

fn publish_fact(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<ProjectorCounters>,
    fact: RuntimeFact,
) {
    publish_required_fact(publisher, bus_id, counters, fact, EXPANSION_BUDGET_MS);
}

#[inline(never)]
fn publish_required_fact(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<ProjectorCounters>,
    fact: RuntimeFact,
    budget_ms: u32,
) -> u32 {
    let outcome = dali2rust_bus::publish_required(
        publisher,
        BusChannel::Commands,
        BusFrame::command(command_envelope(
            SOURCE_ID_UNSPECIFIED,
            fact.correlation_id,
            bus_id.0,
            Some(Origin::Internal),
            RegistryRuntimeUpdateCommand::internal(
                fact.registry_adapter_id,
                RuntimeRegistryUpdateEntry {
                    virtual_lamp_id: fact.virtual_lamp_id,
                    short_address: fact.short_address,
                    setpoint: Some(fact.setpoint),
                    observation: Some(fact.observation),
                    last_dapc_source: fact.last_dapc_source,
                    source: fact.source,
                    observed_at_mono_ms: fact.observed_at_mono_ms,
                },
            ),
        )),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        budget_ms,
        "state-fanout",
    );
    if outcome.retries > 0 {
        counters
            .runtime_updates_retried
            .fetch_add(outcome.retries, Ordering::Relaxed);
    }
    if outcome.queued {
        counters
            .runtime_updates_published
            .fetch_add(1, Ordering::Relaxed);
    } else {
        counters.publish_failed.fetch_add(1, Ordering::Relaxed);
    }
    outcome.slept_ms
}

fn filter_setpoint_for_capabilities(
    setpoint: &LightSetpoint,
    caps: CapabilityFlagsView,
) -> LightSetpoint {
    let color = setpoint
        .color
        .as_ref()
        .filter(|c| capability_accepts_color_mode(caps, c.mode))
        .cloned();
    LightSetpoint {
        color,
        ..setpoint.clone()
    }
}

fn product_last_dapc(dapc: bool, mass: bool) -> Option<LastDapcSource> {
    match (dapc, mass) {
        (false, _) => None,
        (true, false) => Some(LastDapcSource::Unknown),
        (true, true) => Some(LastDapcSource::Group),
    }
}

fn handle_target_state_applied(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    corr: u64,
    body: &DaliTargetStateAppliedEvent,
) {
    let now = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
    match body.scope {
        DaliTargetScope::Short | DaliTargetScope::VirtualLamp => {
            publish_applied_direct(publisher, bus_id, counters, corr, body, now);
        }
        DaliTargetScope::Group => match body.group_id {
            Some(group_id) => expand_group(publisher, bus_id, read_port, counters, GroupExpansion {
                registry_adapter_id: body.registry_adapter_id,
                group_id,
                setpoint: &body.setpoint,
                source: body.source,
                last_dapc_source: product_last_dapc(body.dapc_applied, true),
                last_seen_ms: now,
                observed_at_mono_ms: Some(body.applied_at_mono_ms),
            }),
            None => {
                counters.ignored_events.fetch_add(1, Ordering::Relaxed);
            }
        },
        DaliTargetScope::Broadcast => {
            expand_broadcast(publisher, bus_id, read_port, counters, BroadcastExpansion {
                registry_adapter_id: body.registry_adapter_id,
                setpoint: &body.setpoint,
                source: body.source,
                last_dapc_source: product_last_dapc(body.dapc_applied, true),
                last_seen_ms: now,
                observed_at_mono_ms: Some(body.applied_at_mono_ms),
            });
        }
        DaliTargetScope::AddressRange => {
            counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn publish_applied_direct(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<ProjectorCounters>,
    corr: u64,
    body: &DaliTargetStateAppliedEvent,
    now: u64,
) {
    publish_fact(
        publisher,
        bus_id,
        counters,
        RuntimeFact {
            registry_adapter_id: body.registry_adapter_id,
            virtual_lamp_id: body.virtual_lamp_id,
            short_address: body.short_address,
            setpoint: body.setpoint.clone(),
            observation: RuntimeObservation::timestamped(body.source, now),
            last_dapc_source: product_last_dapc(body.dapc_applied, false),
            source: body.source,
            correlation_id: corr,
            observed_at_mono_ms: Some(body.applied_at_mono_ms),
        },
    );
}

struct GroupExpansion<'a> {
    registry_adapter_id: u8,
    group_id: u8,
    setpoint: &'a LightSetpoint,
    source: RuntimeSource,
    last_dapc_source: Option<LastDapcSource>,
    last_seen_ms: u64,
    observed_at_mono_ms: Option<u32>,
}

#[inline(never)]
fn expand_group(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    exp: GroupExpansion<'_>,
) {
    let Some(snapshot) = read_port.group_apply_snapshot(exp.registry_adapter_id) else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(bit) = group_mask_bit(exp.group_id) else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut pace = ExpansionPace::new();
    for row in snapshot
        .rows
        .iter()
        .filter(|row| row.applied_groups_mask & bit != 0)
    {
        let caps = read_port
            .virtual_lamp_capability_view(exp.registry_adapter_id, row.virtual_lamp_id)
            .capabilities;
        pace.publish(
            publisher,
            bus_id,
            counters,
            RuntimeFact {
                registry_adapter_id: exp.registry_adapter_id,
                virtual_lamp_id: Some(row.virtual_lamp_id),
                short_address: None,
                setpoint: filter_setpoint_for_capabilities(exp.setpoint, caps),
                observation: RuntimeObservation::timestamped(exp.source, exp.last_seen_ms),
                last_dapc_source: exp.last_dapc_source,
                source: exp.source,
                correlation_id: CORRELATION_NONE,
                observed_at_mono_ms: exp.observed_at_mono_ms,
            },
        );
    }
    counters.group_expansions.fetch_add(1, Ordering::Relaxed);
}

struct BroadcastExpansion<'a> {
    registry_adapter_id: u8,
    setpoint: &'a LightSetpoint,
    source: RuntimeSource,
    last_dapc_source: Option<LastDapcSource>,
    last_seen_ms: u64,
    observed_at_mono_ms: Option<u32>,
}

#[inline(never)]
fn expand_broadcast(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    exp: BroadcastExpansion<'_>,
) {
    let mut pace = ExpansionPace::new();
    for view in read_port.list_virtual_lamp_capability_views(exp.registry_adapter_id) {
        if view.binding_short.is_none() {
            counters.skipped_unbound.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        pace.publish(
            publisher,
            bus_id,
            counters,
            RuntimeFact {
                registry_adapter_id: exp.registry_adapter_id,
                virtual_lamp_id: Some(view.virtual_lamp_id),
                short_address: None,
                setpoint: filter_setpoint_for_capabilities(exp.setpoint, view.capabilities),
                observation: RuntimeObservation::timestamped(exp.source, exp.last_seen_ms),
                last_dapc_source: exp.last_dapc_source,
                source: exp.source,
                correlation_id: CORRELATION_NONE,
                observed_at_mono_ms: exp.observed_at_mono_ms,
            },
        );
    }
    counters.broadcast_expansions.fetch_add(1, Ordering::Relaxed);
}

struct SceneRecallExpansion {
    registry_adapter_id: u8,
    scene_id: u8,
    group_id: Option<u8>,
    short_address: Option<u8>,
    source: RuntimeSource,
    last_seen_ms: u64,
    observed_at_mono_ms: Option<u32>,
}

#[inline(never)]
fn expand_scene_recall(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    exp: SceneRecallExpansion,
) {
    let Some(snapshot) = read_port.scene_apply_snapshot(exp.registry_adapter_id, exp.scene_id)
    else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Ok(member_mask) =
        resolve_member_mask(read_port, counters, exp.registry_adapter_id, exp.group_id)
    else {
        return;
    };
    let mut pace = ExpansionPace::new();
    for row in snapshot.rows.iter().filter(|row| row.applied_included) {
        if !recall_reaches_row(member_mask, exp.short_address, row) {
            continue;
        }
        publish_scene_row(publisher, bus_id, read_port, counters, &exp, row, &mut pace);
    }
    counters.scene_expansions.fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "the expansion's context, threaded whole")]
fn publish_scene_row(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    exp: &SceneRecallExpansion,
    row: &dali2rust_domain::registry::SceneApplyRowView,
    pace: &mut ExpansionPace,
) {
    let Some(target) = row.applied_target.as_ref() else {
        return;
    };
    let caps = read_port
        .virtual_lamp_capability_view(exp.registry_adapter_id, row.virtual_lamp_id)
        .capabilities;
    let setpoint = filter_setpoint_for_capabilities(&scene_target_to_setpoint(target), caps);
    pace.publish(
        publisher,
        bus_id,
        counters,
        RuntimeFact {
            registry_adapter_id: exp.registry_adapter_id,
            virtual_lamp_id: Some(row.virtual_lamp_id),
            short_address: None,
            setpoint,
            observation: RuntimeObservation::timestamped(exp.source, exp.last_seen_ms),
            last_dapc_source: Some(LastDapcSource::Scene),
            source: exp.source,
            correlation_id: CORRELATION_NONE,
            observed_at_mono_ms: exp.observed_at_mono_ms,
        },
    );
}

fn resolve_member_mask(
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    registry_adapter_id: u8,
    group_id: Option<u8>,
) -> Result<Option<u64>, ()> {
    match group_id {
        None => Ok(None),
        Some(group_id) => {
            match applied_group_members(read_port, registry_adapter_id, group_id) {
                Some(members) => Ok(Some(members)),
                None => {
                    counters.ignored_events.fetch_add(1, Ordering::Relaxed);
                    Err(())
                }
            }
        }
    }
}

fn recall_reaches_row(
    member_mask: Option<u64>,
    short_address: Option<u8>,
    row: &dali2rust_domain::registry::SceneApplyRowView,
) -> bool {
    if let Some(members) = member_mask {
        if members & (1u64 << row.virtual_lamp_id) == 0 {
            return false;
        }
    }
    match short_address {
        Some(short) => row.binding_short == Some(short),
        None => true,
    }
}

fn applied_group_members(
    read_port: &dyn ProjectorReadPort,
    registry_adapter_id: u8,
    group_id: u8,
) -> Option<u64> {
    read_port.applied_group_member_mask(registry_adapter_id, group_id)
}

fn group_mask_bit(group_id: u8) -> Option<u16> {
    (group_id <= 15).then(|| 1u16 << group_id)
}

fn handle_scene_recalled(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliSceneRecalledEvent,
) {
    if body.error.is_some() {
        return;
    }
    expand_scene_recall(
        publisher,
        bus_id,
        read_port,
        counters,
        SceneRecallExpansion {
            registry_adapter_id: body.registry_adapter_id,
            scene_id: body.scene_id,
            group_id: (body.scope == DaliTargetScope::Group).then_some(body.group_id),
            short_address: None,
            source: RuntimeSource::Api,
            last_seen_ms: dali2rust_bsp::unix_clock::unix_wall_clock_millis(),
            observed_at_mono_ms: Some(body.recalled_at_mono_ms),
        },
    );
}

fn scene_target_to_setpoint(target: &DaliSceneTargetState) -> LightSetpoint {
    let level = target.level.unwrap_or(0);
    LightSetpoint {
        power: target.power.unwrap_or(PowerState::for_level(level)),
        level,
        color: target.color,
    }
}

#[inline(never)]
fn handle_observed_frame(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
) {
    match body.observed_kind {
        ObservedKind::UnknownObserved => {
            counters
                .skipped_unknown_observed
                .fetch_add(1, Ordering::Relaxed);
        }
        ObservedKind::TargetStateObserved => handle_observed_target_state(
            publisher, bus_id, read_port, counters, body,
        ),
        ObservedKind::SceneRecallObserved => {
            handle_observed_scene_recall(publisher, bus_id, read_port, counters, body)
        }
        ObservedKind::LevelTransitionObserved => {
            handle_observed_level_transition(publisher, bus_id, read_port, counters, body)
        }
        ObservedKind::SceneWriteObserved | ObservedKind::SceneRemovalObserved => {}
    }
}

fn transition_members(
    read_port: &dyn ProjectorReadPort,
    body: &DaliObservedFrameEvent,
) -> Vec<(Option<u8>, Option<u8>)> {
    let aid = body.registry_adapter_id;
    match body.scope {
        DaliTargetScope::Short => match body.short_address {
            Some(short) => vec![(bound_lamp_of(read_port, aid, short), Some(short))],
            None => Vec::new(),
        },
        DaliTargetScope::Group => match body.group_id.and_then(group_mask_bit) {
            Some(bit) => read_port
                .group_apply_snapshot(aid)
                .map(|snapshot| {
                    snapshot
                        .rows
                        .iter()
                        .filter(|row| row.applied_groups_mask & bit != 0)
                        .map(|row| (Some(row.virtual_lamp_id), None))
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        },
        DaliTargetScope::Broadcast => read_port
            .list_virtual_lamp_capability_views(aid)
            .into_iter()
            .filter(|view| view.binding_short.is_some())
            .map(|view| (Some(view.virtual_lamp_id), None))
            .collect(),
        DaliTargetScope::VirtualLamp | DaliTargetScope::AddressRange => Vec::new(),
    }
}

fn bound_lamp_of(read_port: &dyn ProjectorReadPort, aid: u8, short: u8) -> Option<u8> {
    read_port
        .list_virtual_lamp_capability_views(aid)
        .into_iter()
        .find(|view| view.binding_short == Some(short))
        .map(|view| view.virtual_lamp_id)
}

#[inline(never)]
fn handle_observed_level_transition(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
) {
    let Some(transition) = body.level_transition else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let aid = body.registry_adapter_id;
    let members = transition_members(read_port, body);
    if members.is_empty() {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut pace = ExpansionPace::new();
    for (virtual_lamp_id, short_address) in members {
        pace.publish_transition(publisher, bus_id, counters, RegistryLevelTransitionCommand {
            adapter_id: aid,
            virtual_lamp_id,
            short_address,
            transition,
            observation: Some(RuntimeObservation::sniffer_timestamped(body.observed_at_ms)),
            source: RuntimeSource::Sniffer,
            observed_at_mono_ms: Some(body.observed_at_mono_ms),
        });
    }
    counters
        .transitions_expanded
        .fetch_add(1, Ordering::Relaxed);
}

fn handle_observed_scene_recall(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
) {
    let Some(scene_id) = body.scene_id else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let (group_id, short_address) = match (body.scope, body.group_id, body.short_address) {
        (DaliTargetScope::Group, Some(group_id), _) => (Some(group_id), None),
        (DaliTargetScope::Short, _, Some(short)) => (None, Some(short)),
        (DaliTargetScope::Group, None, _) | (DaliTargetScope::Short, _, None) => {
            counters.ignored_events.fetch_add(1, Ordering::Relaxed);
            return;
        }
        _ => (None, None),
    };
    expand_scene_recall(
        publisher,
        bus_id,
        read_port,
        counters,
        SceneRecallExpansion {
            registry_adapter_id: body.registry_adapter_id,
            scene_id,
            group_id,
            short_address,
            source: RuntimeSource::Sniffer,
            last_seen_ms: body.observed_at_ms,
            observed_at_mono_ms: Some(body.observed_at_mono_ms),
        },
    );
}

fn handle_observed_target_state(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
) {
    let Some(setpoint) = body.setpoint.as_ref() else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    match body.scope {
        DaliTargetScope::Short => match body.short_address {
            Some(short) => {
                observed_short_fact(publisher, bus_id, read_port, counters, body, short, setpoint)
            }
            None => {
                counters.ignored_events.fetch_add(1, Ordering::Relaxed);
            }
        },
        DaliTargetScope::Group | DaliTargetScope::Broadcast => {
            observed_mass_fact(publisher, bus_id, read_port, counters, body, setpoint);
        }
        DaliTargetScope::VirtualLamp | DaliTargetScope::AddressRange => {
            counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn observed_mass_fact(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
    setpoint: &LightSetpoint,
) {
    let last_dapc_source = body.dapc_observed.then_some(LastDapcSource::Group);
    if body.scope == DaliTargetScope::Group {
        let Some(group_id) = body.group_id else {
            counters.ignored_events.fetch_add(1, Ordering::Relaxed);
            return;
        };
        expand_group(publisher, bus_id, read_port, counters, GroupExpansion {
            registry_adapter_id: body.registry_adapter_id,
            group_id,
            setpoint,
            source: RuntimeSource::Sniffer,
            last_dapc_source,
            last_seen_ms: body.observed_at_ms,
            observed_at_mono_ms: Some(body.observed_at_mono_ms),
        });
    } else {
        expand_broadcast(publisher, bus_id, read_port, counters, BroadcastExpansion {
            registry_adapter_id: body.registry_adapter_id,
            setpoint,
            source: RuntimeSource::Sniffer,
            last_dapc_source,
            last_seen_ms: body.observed_at_ms,
            observed_at_mono_ms: Some(body.observed_at_mono_ms),
        });
    }
}

fn observed_short_fact(
    publisher: &BusPublisher,
    bus_id: BusId,
    read_port: &dyn ProjectorReadPort,
    counters: &Arc<ProjectorCounters>,
    body: &DaliObservedFrameEvent,
    short: u8,
    setpoint: &LightSetpoint,
) {
    let aid = body.registry_adapter_id;
    let bound_vl = read_port
        .list_virtual_lamp_capability_views(aid)
        .into_iter()
        .find(|view| view.binding_short == Some(short))
        .map(|view| view.virtual_lamp_id);
    publish_fact(
        publisher,
        bus_id,
        counters,
        RuntimeFact {
            registry_adapter_id: aid,
            virtual_lamp_id: bound_vl,
            short_address: Some(short),
            setpoint: setpoint.clone(),
            observation: RuntimeObservation::sniffer_timestamped(body.observed_at_ms),
            last_dapc_source: body.dapc_observed.then_some(LastDapcSource::Sniffer),
            source: RuntimeSource::Sniffer,
            correlation_id: CORRELATION_NONE,
            observed_at_mono_ms: Some(body.observed_at_mono_ms),
        },
    );
}

fn handle_read_outcomes(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<ProjectorCounters>,
    body: &DaliAttributeReadOutcomesEvent,
) {
    if !body.any_section_absent() {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    }
    publish_fact(
        publisher,
        bus_id,
        counters,
        RuntimeFact {
            registry_adapter_id: body.registry_adapter_id,
            virtual_lamp_id: None,
            short_address: Some(body.short_address),
            setpoint: LightSetpoint::default(),
            observation: RuntimeObservation::device_absent(RuntimeSource::Poller),
            last_dapc_source: None,
            source: RuntimeSource::Poller,
            correlation_id: CORRELATION_NONE,
            observed_at_mono_ms: None,
        },
    );
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-arm signature")]
fn handle_runtime_status_chunk(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<ProjectorCounters>,
    registry_adapter_id: u8,
    short_address: u8,
    setpoint: Option<&LightSetpoint>,
    observation: &RuntimeObservation,
    read_started_mono_ms: u32,
) {
    let Some(setpoint) = setpoint else {
        counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    publish_fact(
        publisher,
        bus_id,
        counters,
        RuntimeFact {
            registry_adapter_id,
            virtual_lamp_id: None,
            short_address: Some(short_address),
            setpoint: setpoint.clone(),
            observation: observation.clone(),
            last_dapc_source: None,
            source: RuntimeSource::Readback,
            correlation_id: CORRELATION_NONE,
            observed_at_mono_ms: Some(read_started_mono_ms),
        },
    );
}
