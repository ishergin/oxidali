use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{
    publish_or_drop, BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx,
    CorrelationIdAllocator, PublishResult,
};
use dali2rust_contracts::bus::{build_confirmation_envelope, command_envelope};
use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, DaliRecallLastActiveLevelCommand,
    DaliSetTargetStateCommand, DeliveryStatus, HclOverrideClearCommand, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{HclScheduleView, HclSchedulerReadPort};
use dali2rust_platform::small_sort::insertion_sort_by;
use dali2rust_platform::wall_clock::{LocalCivilTime, WallClock};

use super::astronomy::Location;
use super::curve::{effective_points, evaluate, DesiredState};
use super::overrides::{commit_hits_target, driven_dimensions, OverrideLedger, RuntimeCommit};
use super::plan::{
    coalesce, expand_target, plan, DesiredEntry, PlannedCommand, TargetKey,
};

pub const DEFAULT_TICK_PERIOD_MS: u64 = 60_000;
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 2_000;
const MAX_IDLE_WAIT: Duration = Duration::from_secs(1);
const CONFIRMATION_POLL_SLICE: Duration = Duration::from_millis(50);
const MAX_DEFERRED_FRAMES: usize = 64;
const DAYS_PER_WEEK: u8 = 7;

#[derive(Clone, Copy, Debug)]
pub struct HclConfig {
    pub tick_period_ms: u64,
    pub command_timeout_ms: u64,
}

impl Default for HclConfig {
    fn default() -> Self {
        Self {
            tick_period_ms: DEFAULT_TICK_PERIOD_MS,
            command_timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Default)]
pub struct HclSchedulerCounters {
    pub ticks: AtomicU32,
    pub ticks_time_unsynced: AtomicU32,
    pub commands_published: AtomicU32,
    pub commands_dropped_cap: AtomicU32,
    pub deferred_dropped_cap: AtomicU32,
    pub command_timeouts: AtomicU32,
    pub command_failures: AtomicU32,
    pub ingress_rejections: AtomicU32,
    pub overrides_started: AtomicU32,
    pub overrides_cleared: AtomicU32,
    pub overrides_reset: AtomicU32,
    pub ignored_events: AtomicU32,
    pub ignored_commands: AtomicU32,
}

pub const HCL_SCHEDULER_HANDLED_EVENTS: &[&str] =
    &["HclScheduleChangedEvent", "RuntimeStateChangedEvent"];

pub type SharedOverrideLedger = Arc<Mutex<OverrideLedger>>;

pub(crate) fn lock_ledger(ledger: &SharedOverrideLedger) -> MutexGuard<'_, OverrideLedger> {
    ledger.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct PublishRecord {
    state: DesiredState,
    confirmed: bool,
}

#[derive(Default)]
struct PublishOutcomes {
    entries: Vec<(TargetKey, bool)>,
}

impl PublishOutcomes {
    fn merge(&mut self, key: TargetKey, confirmed: bool) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, all)) => *all &= confirmed,
            None => self.entries.push((key, confirmed)),
        }
    }

    fn demote(&mut self, key: TargetKey) {
        if let Some((_, confirmed)) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            *confirmed = false;
        }
    }

    fn get(&self, key: &TargetKey) -> Option<bool> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, c)| *c)
    }
}

struct SchedulerState {
    last_published: HashMap<TargetKey, PublishRecord>,
    published_from: Option<Vec<HclScheduleView>>,
}

impl SchedulerState {
    fn new() -> Self {
        Self {
            last_published: HashMap::new(),
            published_from: None,
        }
    }

    fn forget_edited(&mut self, current: &[HclScheduleView]) {
        if self.published_from.as_deref() == Some(current) {
            for record in self.last_published.values_mut() {
                record.confirmed = false;
            }
            return;
        }
        self.last_published.clear();
        self.published_from = None;
    }
}

struct SchedulerDeps {
    publisher: BusPublisher,
    bus_id: BusId,
    read_port: Arc<dyn HclSchedulerReadPort>,
    clock: Arc<dyn WallClock>,
    correlation: Arc<CorrelationIdAllocator>,
    config: HclConfig,
    counters: Arc<HclSchedulerCounters>,
    overrides: SharedOverrideLedger,
    role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
}

#[allow(clippy::too_many_arguments, reason = "one spawn site; the deps are grouped inside")]
pub fn spawn_hcl_scheduler_worker(
    rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    read_port: Arc<dyn HclSchedulerReadPort>,
    clock: Arc<dyn WallClock>,
    correlation: Arc<CorrelationIdAllocator>,
    config: HclConfig,
    counters: Arc<HclSchedulerCounters>,
    overrides: SharedOverrideLedger,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
    role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
) -> std::thread::JoinHandle<()> {
    let deps = SchedulerDeps {
        publisher,
        bus_id,
        read_port,
        clock,
        correlation,
        config,
        counters,
        overrides,
        role,
        hold,
        liveness,
    };
    esp_thread::spawn_named_stack_in(
        c"hcl-scheduler",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || run_loop(rx, conf_rx, deps),
    )
}

fn drain_stale_confirmations(conf_rx: &BusSubscriberRx) {
    while conf_rx.try_recv().is_ok() {}
}

static WORKER_STACK: StackLowWater =
    StackLowWater::new("hcl-scheduler", std_thread_stack::EVENT_WORKER_STACK);

fn run_loop(rx: BusSubscriberRx, conf_rx: BusSubscriberRx, deps: SchedulerDeps) {
    let mut state = SchedulerState::new();
    let period = Duration::from_millis(deps.config.tick_period_ms);
    let mut next_tick = Instant::now();
    loop {
        if Instant::now() >= next_tick {
            run_tick(&mut state, &rx, &conf_rx, &deps);
            WORKER_STACK.note("tick");
            next_tick = Instant::now() + period;
        }
        let wait = next_tick
            .saturating_duration_since(Instant::now())
            .min(MAX_IDLE_WAIT);
        match deps.liveness.while_turning(|| rx.recv_timeout(wait)) {
            Ok(frame) => {
                apply_frame(&mut state, &deps, &frame);
                WORKER_STACK.note(frame_tag(&frame));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => WORKER_STACK.note("idle"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        drain_stale_confirmations(&conf_rx);
    }
}

fn frame_tag(frame: &BusFrame) -> &'static str {
    match frame {
        BusFrame::Event(envelope) => match &envelope.payload {
            BusEventPayload::HclScheduleChangedEvent(_) => "event:HclScheduleChanged",
            BusEventPayload::RuntimeStateChangedEvent(_) => "event:RuntimeStateChanged",
            _ => "event:other",
        },
        BusFrame::Command(_) => "command",
        BusFrame::Confirmation(_) => "confirmation",
    }
}

fn apply_frame(state: &mut SchedulerState, deps: &SchedulerDeps, frame: &BusFrame) {
    match frame {
        BusFrame::Event(envelope) => apply_event(state, deps, &envelope.payload),
        BusFrame::Command(envelope) => dispatch_scheduler_command(
            &envelope.payload,
            &deps.publisher,
            &deps.overrides,
            &deps.counters,
            envelope.meta.correlation_id,
        ),
        BusFrame::Confirmation(_) => {
            deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const HCL_SCHEDULER_HANDLED_COMMANDS;
    fn dispatch_scheduler_command(
        payload: &BusCommandPayload,
        publisher: &BusPublisher,
        overrides: &SharedOverrideLedger,
        counters: &Arc<HclSchedulerCounters>,
        correlation_id: u64,
    );
    payload = payload;
    ignored = { counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    HclOverrideClearCommand(body) =>
        handle_override_clear(publisher, overrides, counters, correlation_id, body),
}

fn handle_override_clear(
    publisher: &BusPublisher,
    overrides: &SharedOverrideLedger,
    counters: &Arc<HclSchedulerCounters>,
    correlation_id: u64,
    body: &HclOverrideClearCommand,
) {
    let lifted = lock_ledger(overrides).clear_schedule(body.schedule_id.as_str());
    if lifted > 0 {
        counters
            .overrides_reset
            .fetch_add(lifted as u32, Ordering::Relaxed);
    }
    let confirmation = BusFrame::confirmation(build_confirmation_envelope(
        correlation_id,
        DeliveryStatus::Ok,
        0,
        SOURCE_ID_UNSPECIFIED,
    ));
    publish_or_drop(
        publisher,
        BusChannel::Confirmations,
        confirmation,
        "hcl-override-clear",
    );
}

fn apply_event(state: &mut SchedulerState, deps: &SchedulerDeps, payload: &BusEventPayload) {
    match payload {
        BusEventPayload::HclScheduleChangedEvent(body) => {
            state.forget_edited(&sorted_schedules(deps));
            if body.removed {
                let lifted = lock_ledger(&deps.overrides).clear_schedule(body.schedule_id.as_str());
                if lifted > 0 {
                    deps.counters
                        .overrides_reset
                        .fetch_add(lifted as u32, Ordering::Relaxed);
                }
            }
        }
        BusEventPayload::RuntimeStateChangedEvent(body) => {
            let commit = RuntimeCommit {
                adapter_id: body.adapter_id,
                virtual_lamp_id: body.virtual_lamp_id,
                value_source: Some(body.commit_source),
                states: body.commit_dimensions,
            };
            suspend_hit_targets(state, deps, &commit);
        }
        _ => {
            deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn suspend_hit_targets(state: &mut SchedulerState, deps: &SchedulerDeps, commit: &RuntimeCommit) {
    if !commit.is_foreign() {
        return;
    }
    let Some(local) = deps.clock.local() else {
        return;
    };
    roll_over_ledger(state, deps, local.year_day);
    let mut hit: Vec<TargetKey> = Vec::new();
    let mut spared: Vec<TargetKey> = Vec::new();
    for schedule in deps.read_port.list_hcl_schedule_views() {
        if !runs_today(&schedule, local) {
            continue;
        }
        let driven = driven_dimensions(&schedule.points);
        for target in schedule.targets.iter().flat_map(expand_target) {
            if !state.last_published.contains_key(&target) {
                continue;
            }
            if !commit_hits_target(deps.read_port.as_ref(), commit, target, driven) {
                spared.push(target);
                continue;
            }
            let started = lock_ledger(&deps.overrides).suspend(
                &schedule.schedule_id,
                target,
                local.minutes_since_midnight,
            );
            if started {
                deps.counters.overrides_started.fetch_add(1, Ordering::Relaxed);
            }
            hit.push(target);
        }
    }
    for target in hit {
        if spared.contains(&target) {
            continue;
        }
        state.last_published.remove(&target);
    }
}

fn roll_over_ledger(state: &mut SchedulerState, deps: &SchedulerDeps, year_day: u16) {
    let Some(cleared) = lock_ledger(&deps.overrides).roll_over_to(year_day) else {
        return;
    };
    if cleared > 0 {
        deps.counters
            .overrides_cleared
            .fetch_add(cleared as u32, Ordering::Relaxed);
    }
    state.last_published.clear();
}

fn tick_is_due(deps: &SchedulerDeps) -> Option<LocalCivilTime> {
    if !deps.role.dali_settings_view().application_active {
        return None;
    }
    let local = deps.clock.local();
    if local.is_none() {
        deps.counters
            .ticks_time_unsynced
            .fetch_add(1, Ordering::Relaxed);
    }
    local
}

fn held_for_maintenance(deps: &SchedulerDeps) -> bool {
    deps.hold.engaged()
}

fn run_tick(
    state: &mut SchedulerState,
    rx: &BusSubscriberRx,
    conf_rx: &BusSubscriberRx,
    deps: &SchedulerDeps,
) {
    deps.counters.ticks.fetch_add(1, Ordering::Relaxed);
    if held_for_maintenance(deps) {
        return;
    }
    let Some(local) = tick_is_due(deps) else {
        return;
    };
    roll_over_ledger(state, deps, local.year_day);
    state.published_from = Some(sorted_schedules(deps));
    let schedules = state.published_from.as_deref().unwrap_or_default();
    let entries = coalesce(desired_entries(deps, schedules, local));
    WORKER_STACK.note("tick:desired");
    let fresh = unconfirmed_entries(state, entries);
    let tick_plan = plan(&fresh);
    if tick_plan.dropped > 0 {
        deps.counters
            .commands_dropped_cap
            .fetch_add(tick_plan.dropped as u32, Ordering::Relaxed);
    }
    WORKER_STACK.note("tick:plan");
    let mut deferred = Vec::new();
    let outcomes = publish_plan(rx, conf_rx, deps, &tick_plan.commands, &mut deferred);
    WORKER_STACK.note("tick:publish");
    for entry in fresh {
        if let Some(confirmed) = outcomes.get(&entry.key) {
            state.last_published.insert(
                entry.key,
                PublishRecord {
                    state: entry.state,
                    confirmed,
                },
            );
        }
    }
    for frame in deferred {
        apply_frame(state, deps, &frame);
    }
}

fn unconfirmed_entries(state: &SchedulerState, entries: Vec<DesiredEntry>) -> Vec<DesiredEntry> {
    entries
        .into_iter()
        .filter(|entry| {
            !state
                .last_published
                .get(&entry.key)
                .is_some_and(|rec| rec.confirmed && rec.state.wire_equal(&entry.state))
        })
        .collect()
}

fn sorted_schedules(deps: &SchedulerDeps) -> Vec<HclScheduleView> {
    let mut schedules = deps.read_port.list_hcl_schedule_views();
    insertion_sort_by(&mut schedules, |a, b| a.schedule_id > b.schedule_id);
    schedules
}

fn desired_entries(
    deps: &SchedulerDeps,
    schedules: &[HclScheduleView],
    local: LocalCivilTime,
) -> Vec<DesiredEntry> {
    let ledger = lock_ledger(&deps.overrides);
    schedules
        .iter()
        .filter(|schedule| runs_today(schedule, local))
        .filter_map(|schedule| Some((schedule, desired_state(schedule, local)?)))
        .flat_map(|(schedule, desired)| {
            schedule
                .targets
                .iter()
                .flat_map(expand_target)
                .filter(|target| !ledger.is_suspended(&schedule.schedule_id, *target))
                .map(move |key| DesiredEntry { key, state: desired })
        })
        .collect()
}

fn runs_today(schedule: &HclScheduleView, local: LocalCivilTime) -> bool {
    local.weekday < DAYS_PER_WEEK
        && schedule.enabled
        && schedule.active_days_mask & (1u8 << local.weekday) != 0
}

fn desired_state(schedule: &HclScheduleView, local: LocalCivilTime) -> Option<DesiredState> {
    let location =
        Location::from_microdeg(schedule.latitude_microdeg, schedule.longitude_microdeg);
    let points = effective_points(
        &schedule.points,
        location,
        local.year_day,
        local.utc_offset_minutes,
    );
    evaluate(
        &points,
        i32::from(local.minutes_since_midnight),
        schedule.algorithm,
    )
}

fn publish_plan(
    rx: &BusSubscriberRx,
    conf_rx: &BusSubscriberRx,
    deps: &SchedulerDeps,
    commands: &[PlannedCommand],
    deferred: &mut Vec<BusFrame>,
) -> PublishOutcomes {
    let mut outcomes = PublishOutcomes::default();
    for (index, command) in commands.iter().enumerate() {
        let correlation_id = deps.correlation.next_id();
        let frame = BusFrame::command(command_envelope_for(deps, command, correlation_id));
        if deps.publisher.try_publish(BusChannel::Commands, frame) != PublishResult::Queued {
            deps.counters
                .ingress_rejections
                .fetch_add(1, Ordering::Relaxed);
            mark_unpublished_keys(&mut outcomes, &commands[index..]);
            break;
        }
        deps.counters
            .commands_published
            .fetch_add(1, Ordering::Relaxed);
        let confirmed = wait_for_confirmation(
            rx,
            conf_rx,
            deps,
            correlation_id,
            deps.config.command_timeout_ms,
            deferred,
        );
        outcomes.merge(command.key(), confirmed);
    }
    outcomes
}

fn mark_unpublished_keys(outcomes: &mut PublishOutcomes, unpublished: &[PlannedCommand]) {
    for command in unpublished {
        outcomes.demote(command.key());
    }
}

fn service_commands(rx: &BusSubscriberRx, deps: &SchedulerDeps, deferred: &mut Vec<BusFrame>) {
    while let Ok(frame) = rx.try_recv() {
        match frame {
            BusFrame::Command(envelope) => dispatch_scheduler_command(
                &envelope.payload,
                &deps.publisher,
                &deps.overrides,
                &deps.counters,
                envelope.meta.correlation_id,
            ),
            other => defer_frame(&deps.counters, deferred, other),
        }
    }
}

fn defer_frame(counters: &HclSchedulerCounters, deferred: &mut Vec<BusFrame>, frame: BusFrame) {
    if deferred.len() >= MAX_DEFERRED_FRAMES {
        counters
            .deferred_dropped_cap
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    deferred.push(frame);
}

fn command_envelope_for(
    deps: &SchedulerDeps,
    command: &PlannedCommand,
    correlation_id: u64,
) -> dali2rust_contracts::msg::CommandEnvelope {
    let key = command.key();
    match command {
        PlannedCommand::TargetState { setpoint, .. } => command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            deps.bus_id.0,
            Some(Origin::Hcl),
            DaliSetTargetStateCommand {
                scope: command.dali_scope(),
                virtual_lamp_id: 0,
                short_address: 0,
                group_id: key.group_id,
                setpoint: setpoint.clone(),
                registry_adapter_id: key.adapter_id,
            },
        ),
        PlannedCommand::RecallLastActive { .. } => command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            deps.bus_id.0,
            Some(Origin::Hcl),
            DaliRecallLastActiveLevelCommand {
                registry_adapter_id: key.adapter_id,
                scope: key.scope,
                group_id: match key.scope {
                    dali2rust_contracts::msg::HclTargetScope::Group => Some(key.group_id),
                    dali2rust_contracts::msg::HclTargetScope::Broadcast => None,
                },
            },
        ),
    }
}

fn wait_for_confirmation(
    rx: &BusSubscriberRx,
    conf_rx: &BusSubscriberRx,
    deps: &SchedulerDeps,
    correlation_id: u64,
    timeout_ms: u64,
    deferred: &mut Vec<BusFrame>,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            deps.counters.command_timeouts.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match deps
            .liveness
            .while_turning(|| conf_rx.recv_timeout(remaining.min(CONFIRMATION_POLL_SLICE)))
        {
            Ok(BusFrame::Confirmation(envelope)) => {
                if envelope.meta.correlation_id == correlation_id {
                    if envelope.status == dali2rust_contracts::msg::DeliveryStatus::Ok {
                        return true;
                    }
                    deps.counters.command_failures.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                service_commands(rx, deps, deferred);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                deps.counters.command_timeouts.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::bus::event_envelope;
    use dali2rust_contracts::msg::{
        HclScheduleChangedEvent, fixed_text_32,
    };

    fn any_event_frame() -> BusFrame {
        BusFrame::event(event_envelope(
            SOURCE_ID_UNSPECIFIED,
            0,
            BusId::default().0,
            Some(Origin::Registry),
            HclScheduleChangedEvent {
                schedule_id: fixed_text_32("morning"),
                removed: false,
                enabled: true,
            },
        ))
    }

    #[test]
    fn an_ingress_break_between_a_pairs_halves_unconfirms_the_key_but_keeps_it_driven() {
        let pair_key = TargetKey {
            adapter_id: 0,
            scope: dali2rust_contracts::msg::HclTargetScope::Group,
            group_id: 3,
        };
        let undriven_key = TargetKey {
            adapter_id: 0,
            scope: dali2rust_contracts::msg::HclTargetScope::Group,
            group_id: 5,
        };
        let mut outcomes = PublishOutcomes::default();
        outcomes.merge(pair_key, true);
        mark_unpublished_keys(
            &mut outcomes,
            &[
                PlannedCommand::RecallLastActive { key: pair_key },
                PlannedCommand::RecallLastActive { key: undriven_key },
            ],
        );
        assert_eq!(
            outcomes.get(&pair_key),
            Some(false),
            "a partially-published entry is driven but not confirmed"
        );
        assert!(
            outcomes.get(&undriven_key).is_none(),
            "an entry nothing was published for must not arm the override gate"
        );
    }

    #[test]
    fn the_deferred_buffer_holds_its_cap_and_counts_the_rest() {
        let counters = HclSchedulerCounters::default();
        let mut deferred = Vec::new();
        let overflow = 5;
        for _ in 0..MAX_DEFERRED_FRAMES + overflow {
            defer_frame(&counters, &mut deferred, any_event_frame());
        }
        assert_eq!(deferred.len(), MAX_DEFERRED_FRAMES, "the cap holds");
        assert_eq!(
            counters.deferred_dropped_cap.load(Ordering::Relaxed),
            overflow as u32,
            "every dropped frame is counted, so the loss is visible"
        );
    }
}
