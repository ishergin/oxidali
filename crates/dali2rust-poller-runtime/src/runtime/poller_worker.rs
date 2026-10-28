use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, CorrelationIdAllocator, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusEventPayload, DaliAttributeGroup,
    BusHealthVerdict, ConfirmationEnvelope, DaliAttributeReadOutcomesEvent, DaliBusHealthProbeCommand,
    DaliBusHealthProbedEvent, DaliReadAttributesCommand, DeliveryStatus, ErrorCode,
    MemoryBankReadPreset, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{PollTargetView, PollerReadPort, PollerSettingsView};

use super::power_cycle::{short_bit, PollerConfig, PowerCycleWatch};

const MAX_IDLE_WAIT: Duration = Duration::from_secs(1);
const OUTSTANDING_ABANDON_TTL: Duration = Duration::from_secs(16 * 60);
const MAX_CHARGEABLE_WAIT: Duration = Duration::from_secs(30);
const CORRELATION_TOMBSTONES_MAX: usize = 8;
const MIN_INTERVAL_MS: u32 = 200;
const COOLDOWN_MULTIPLE: u32 = 3;

const ABSENT_COOLDOWN_MULTIPLES: [u32; 3] = [3, 6, 12];

fn absent_cooldown_multiple(strikes: u8) -> u32 {
    let idx = usize::from(strikes.saturating_sub(1)).min(ABSENT_COOLDOWN_MULTIPLES.len() - 1);
    ABSENT_COOLDOWN_MULTIPLES[idx]
}

#[derive(Debug, Default)]
pub struct PollerCounters {
    pub cycles_total: AtomicU32,
    pub reads_published: AtomicU32,
    pub health_probes_published: AtomicU32,
    pub health_probes_invalid: AtomicU32,
    pub health_probes_clear: AtomicU32,
    pub health_probes_one_failure: AtomicU32,
    pub health_probes_several_failures: AtomicU32,
    pub health_probes_expired: AtomicU32,
    pub health_probes_power_failure: AtomicU32,
    pub sweeps: AtomicU32,
    pub sweep_reads: AtomicU32,
    pub reads_completed: AtomicU32,
    pub reads_failed: AtomicU32,
    pub reads_preempted: AtomicU32,
    pub outstanding_expired: AtomicU32,
    pub skipped_inbox_full: AtomicU32,
    pub duty_deferred: AtomicU32,
    pub interactive_deferred: AtomicU32,
    pub reads_absent: AtomicU32,
    pub targets_excluded: AtomicU32,
    pub window_deferred: AtomicU32,
    pub device_cooldowns: AtomicU32,
    pub ignored_events: AtomicU32,
}

#[derive(Clone, Copy, Debug)]
struct DeviceCooldown {
    until: Instant,
    absent_strikes: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadPurpose {
    Cycle,
    PowerCycleSweep,
}

#[derive(Clone, Copy, Debug)]
struct PollTarget {
    adapter_id: u8,
    short_address: u8,
    groups_mask: u8,
    memory_banks: MemoryBankReadPreset,
    purpose: ReadPurpose,
}

#[derive(Clone, Copy, Debug)]
struct Outstanding {
    correlation_id: u64,
    target: PollTarget,
    published_at: Instant,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug)]
struct PendingProbe {
    correlation_id: u64,
    published_at: Instant,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug)]
struct CorrelationTombstone {
    correlation_id: u64,
    published_at: Instant,
}

struct PollerState {
    settings: PollerSettingsView,
    queue: VecDeque<PollTarget>,
    sweep_queue: VecDeque<PollTarget>,
    outstanding: Option<Outstanding>,
    probe_due: VecDeque<u8>,
    pending_probe: Option<PendingProbe>,
    probe_cursor: u8,
    last_probe: Instant,
    power_cycles: PowerCycleWatch,
    tombstones: VecDeque<CorrelationTombstone>,
    cooldown: HashMap<(u8, u8), DeviceCooldown>,
    bank_reads: HashMap<(u8, u8, u8), Instant>,
    rest_until: Instant,
    duty_deferred_for: Option<Instant>,
    interactive_deferred_open: bool,
    cursor: Option<(u8, u8)>,
    last_cycle: Instant,
    next_cycle: Instant,
}

impl PollerState {
    fn boot(settings: PollerSettingsView) -> Self {
        let now = Instant::now();
        Self {
            settings,
            queue: VecDeque::new(),
            sweep_queue: VecDeque::new(),
            outstanding: None,
            probe_due: VecDeque::new(),
            pending_probe: None,
            probe_cursor: 0,
            last_probe: now,
            power_cycles: PowerCycleWatch::default(),
            tombstones: VecDeque::new(),
            cooldown: HashMap::new(),
            bank_reads: HashMap::new(),
            rest_until: now,
            duty_deferred_for: None,
            interactive_deferred_open: false,
            cursor: None,
            last_cycle: now,
            next_cycle: now,
        }
    }
}

pub struct PollerWiring {
    pub publisher: BusPublisher,
    pub read_port: Arc<dyn PollerReadPort>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub bus_id: BusId,
    pub adapter_count: u8,
    pub counters: Arc<PollerCounters>,
    pub role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub interactive: Arc<dali2rust_platform::dali::WireActivity>,
    pub hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
    pub config: PollerConfig,
}

pub fn spawn_poller_worker(
    ev_rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    deps: PollerWiring,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"poller",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || run_loop(ev_rx, conf_rx, deps),
    )
}

fn run_loop(ev_rx: BusSubscriberRx, conf_rx: BusSubscriberRx, deps: PollerWiring) {
    let mut state = PollerState::boot(deps.read_port.poller_settings_view());
    loop {
        if Instant::now() >= state.next_cycle {
            start_cycle(&mut state, &deps);
        }
        let next_probe_round = watch_probe_round_at(&state, &deps);
        if next_probe_round.is_some_and(|at| Instant::now() >= at) {
            start_probe_round(&mut state, &deps);
        }
        let wait = next_probe_round
            .map_or(state.next_cycle, |at| state.next_cycle.min(at))
            .saturating_duration_since(Instant::now())
            .min(MAX_IDLE_WAIT);
        match ev_rx.recv_timeout(wait) {
            Ok(frame) => apply_event(&mut state, &deps, &frame),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        drain_confirmations(&mut state, &deps, &conf_rx);
        sweep_stale(&mut state, &deps);
        pump_queue(&mut state, &deps);
    }
}

fn watch_probe_round_at(state: &PollerState, deps: &PollerWiring) -> Option<Instant> {
    state
        .settings
        .power_cycle_watch
        .then(|| state.last_probe + deps.config.probe_period)
}

fn cycle_period(settings: &PollerSettingsView) -> Duration {
    Duration::from_millis(u64::from(settings.interval_ms.max(MIN_INTERVAL_MS)))
}

fn probe_rota(cursor: u8, enabled_adapters: &[u8]) -> VecDeque<u8> {
    let start = enabled_adapters
        .iter()
        .position(|adapter_id| *adapter_id >= cursor)
        .unwrap_or(0);
    enabled_adapters[start..]
        .iter()
        .chain(&enabled_adapters[..start])
        .copied()
        .collect()
}

fn start_cycle(state: &mut PollerState, deps: &PollerWiring) {
    deps.counters.cycles_total.fetch_add(1, Ordering::Relaxed);
    state.last_cycle = Instant::now();
    state.next_cycle = state.last_cycle + cycle_period(&state.settings);
    if !state.queue.is_empty() {
        deps.counters.window_deferred.fetch_add(1, Ordering::Relaxed);
    }
    let application_active = deps.role.dali_settings_view().application_active;
    state.queue = if state.settings.enabled && application_active {
        let (queue, enabled_adapters) = select_targets(state, deps);
        arm_probe_round(state, &enabled_adapters);
        queue
    } else {
        deps.counters.targets_excluded.store(0, Ordering::Relaxed);
        if !state.settings.power_cycle_watch {
            state.probe_due.clear();
        }
        VecDeque::new()
    };
    if !application_active {
        stand_down_power_cycle_watch(state);
    }
}

fn start_probe_round(state: &mut PollerState, deps: &PollerWiring) {
    if !deps.role.dali_settings_view().application_active {
        state.last_probe = Instant::now();
        stand_down_power_cycle_watch(state);
        return;
    }
    let enabled_adapters: Vec<u8> = (0..deps.adapter_count)
        .filter(|adapter_id| deps.read_port.list_poll_targets(*adapter_id).adapter_enabled)
        .collect();
    arm_probe_round(state, &enabled_adapters);
}

fn arm_probe_round(state: &mut PollerState, enabled_adapters: &[u8]) {
    state.last_probe = Instant::now();
    state.probe_due = probe_rota(state.probe_cursor, enabled_adapters);
    state.sweep_queue.retain(|target| enabled_adapters.contains(&target.adapter_id));
    state.power_cycles.keep_only(enabled_adapters);
}

fn stand_down_power_cycle_watch(state: &mut PollerState) {
    state.probe_due.clear();
    state.sweep_queue.clear();
    state.power_cycles = PowerCycleWatch::default();
}

fn publish_health_probe(state: &mut PollerState, deps: &PollerWiring) {
    let Some(adapter_id) = state.probe_due.pop_front() else {
        return;
    };
    let correlation_id = deps.correlation.next_id();
    let cmd = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        deps.bus_id.0,
        Some(Origin::Poller),
        DaliBusHealthProbeCommand {
            registry_adapter_id: adapter_id,
            query_power_failure: state.settings.power_cycle_watch,
        },
    );
    if deps.publisher.try_publish(BusChannel::Commands, BusFrame::command(cmd))
        == PublishResult::Queued
    {
        let now = Instant::now();
        state.pending_probe = Some(PendingProbe {
            correlation_id,
            published_at: now,
            deadline: now + OUTSTANDING_ABANDON_TTL,
        });
        state.probe_cursor = adapter_id.wrapping_add(1);
        deps.counters.health_probes_published.fetch_add(1, Ordering::Relaxed);
    } else {
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
    }
}

fn select_targets(state: &mut PollerState, deps: &PollerWiring) -> (VecDeque<PollTarget>, Vec<u8>) {
    let mut queue = VecDeque::new();
    let mut listed: std::collections::HashSet<(u8, u8)> = std::collections::HashSet::new();
    let mut excluded: u32 = 0;
    let mut enabled_adapters = Vec::new();
    for adapter_id in 0..deps.adapter_count {
        let selection = deps.read_port.list_poll_targets(adapter_id);
        if selection.adapter_enabled {
            enabled_adapters.push(adapter_id);
        }
        excluded += u32::from(selection.excluded_unbound);
        for target in selection.targets {
            listed.insert((adapter_id, target.short_address));
            if state.cooldown.get(&(adapter_id, target.short_address)).is_some_and(|c| Instant::now() < c.until) {
                continue;
            }
            if already_outstanding(state, adapter_id, target.short_address) {
                continue;
            }
            let groups_mask = groups_mask_for(&state.settings, &target);
            let memory_banks = due_bank_preset(state, adapter_id, &target);
            queue.push_back(PollTarget {
                adapter_id,
                short_address: target.short_address,
                groups_mask,
                memory_banks,
                purpose: ReadPurpose::Cycle,
            });
        }
    }
    deps.counters.targets_excluded.store(excluded, Ordering::Relaxed);
    prune_delisted_device_state(state, &listed);
    (rotate_after_cursor(queue, state.cursor), enabled_adapters)
}

fn prune_delisted_device_state(state: &mut PollerState, listed: &std::collections::HashSet<(u8, u8)>) {
    state.cooldown.retain(|key, _| listed.contains(key));
    state
        .bank_reads
        .retain(|(adapter, short, _), _| listed.contains(&(*adapter, *short)));
}

fn rotate_after_cursor(
    mut queue: VecDeque<PollTarget>,
    cursor: Option<(u8, u8)>,
) -> VecDeque<PollTarget> {
    let Some(cursor) = cursor else {
        return queue;
    };
    let resume = queue
        .iter()
        .position(|t| (t.adapter_id, t.short_address) > cursor)
        .unwrap_or(0);
    queue.rotate_left(resume);
    queue
}

fn already_outstanding(state: &PollerState, adapter_id: u8, short_address: u8) -> bool {
    state.outstanding.is_some_and(|o| {
        o.target.adapter_id == adapter_id && o.target.short_address == short_address
    })
}

fn groups_mask_for(settings: &PollerSettingsView, target: &PollTargetView) -> u8 {
    let mut mask = DaliAttributeGroup::polled_mask(settings.attribute_groups_mask);
    if settings.include_dt8_color && target.is_dt8 {
        mask |= DaliAttributeGroup::Dt8Color.mask_bit();
    }
    if !target.is_dt6 {
        mask &= !DaliAttributeGroup::Dt6Led.mask_bit();
    }
    mask
}

// DiiA 252 §9.2.7
// DiiA 253 §9.2.9
const POWER_MIN_INTERVAL: Duration = Duration::from_secs(30);
const ENERGY_MIN_INTERVAL: Duration = Duration::from_secs(600);
const DIAGNOSTICS_MIN_INTERVAL: Duration = Duration::from_secs(300);

fn due_bank_preset(
    state: &PollerState,
    adapter_id: u8,
    target: &PollTargetView,
) -> MemoryBankReadPreset {
    let now = Instant::now();
    let mut best: Option<(Duration, MemoryBankReadPreset)> = None;
    for (preset, interval) in bank_series_for(&state.settings, target) {
        let key = (adapter_id, target.short_address, preset as u8);
        let overdue = match state.bank_reads.get(&key) {
            None => Duration::MAX,
            Some(last) => {
                let Some(interval) = interval else { continue };
                let elapsed = now.saturating_duration_since(*last);
                match elapsed.checked_sub(interval) {
                    Some(over) => over,
                    None => continue,
                }
            }
        };
        if best.is_none_or(|(best_over, _)| overdue > best_over) {
            best = Some((overdue, preset));
        }
    }
    best.map_or(MemoryBankReadPreset::None, |(_, preset)| preset)
}

fn bank_series_for(
    settings: &PollerSettingsView,
    target: &PollTargetView,
) -> Vec<(MemoryBankReadPreset, Option<Duration>)> {
    let mut series = Vec::new();
    if settings.include_energy && target.declares_energy {
        series.push((MemoryBankReadPreset::Power, Some(POWER_MIN_INTERVAL)));
        series.push((MemoryBankReadPreset::Energy, Some(ENERGY_MIN_INTERVAL)));
    }
    if settings.include_diagnostics && target.declares_diagnostics {
        series.push((
            MemoryBankReadPreset::Diagnostics,
            Some(DIAGNOSTICS_MIN_INTERVAL),
        ));
        series.push((MemoryBankReadPreset::LuminaireData, None));
    }
    series
}

const INTERACTIVE_QUIET: Duration = Duration::from_millis(400);

const BACKGROUND_WIRE_REST_MULTIPLE: u32 = 3;

fn pump_queue(state: &mut PollerState, deps: &PollerWiring) {
    if !state.settings.enabled {
        state.queue.clear();
        if !state.settings.power_cycle_watch {
            state.probe_due.clear();
            return;
        }
    }
    if deps.hold.engaged() {
        return;
    }
    let probe_ready = state.pending_probe.is_none() && !state.probe_due.is_empty();
    let read_ready = state.outstanding.is_none()
        && !(state.sweep_queue.is_empty() && state.queue.is_empty());
    if !probe_ready && !read_ready {
        return;
    }
    if !deps.interactive.quiet_for(INTERACTIVE_QUIET) {
        charge_interactive_deferral(state, deps);
        return;
    }
    state.interactive_deferred_open = false;
    if Instant::now() < state.rest_until {
        charge_duty_deferral(state, deps);
        return;
    }
    if probe_ready {
        publish_health_probe(state, deps);
    }
    if let Some(target) = read_ready
        .then(|| state.sweep_queue.pop_front().or_else(|| state.queue.pop_front()))
        .flatten()
    {
        publish_read(state, deps, target);
    }
}

fn charge_duty_deferral(state: &mut PollerState, deps: &PollerWiring) {
    if state.duty_deferred_for == Some(state.rest_until) {
        return;
    }
    state.duty_deferred_for = Some(state.rest_until);
    deps.counters.duty_deferred.fetch_add(1, Ordering::Relaxed);
}

fn charge_interactive_deferral(state: &mut PollerState, deps: &PollerWiring) {
    if state.interactive_deferred_open {
        return;
    }
    state.interactive_deferred_open = true;
    deps.counters.interactive_deferred.fetch_add(1, Ordering::Relaxed);
}

fn charge_wire_time(state: &mut PollerState, cost: Duration) {
    let charged = cost.min(MAX_CHARGEABLE_WAIT);
    state.rest_until = state
        .rest_until
        .max(Instant::now() + charged * BACKGROUND_WIRE_REST_MULTIPLE);
}

fn publish_read(state: &mut PollerState, deps: &PollerWiring, target: PollTarget) {
    let correlation_id = deps.correlation.next_id();
    let cmd = command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        deps.bus_id.0,
        Some(Origin::Poller),
        DaliReadAttributesCommand {
            registry_adapter_id: target.adapter_id,
            short_address: target.short_address,
            attribute_groups_mask: target.groups_mask,
            memory_banks: target.memory_banks,
        },
    );
    let result = deps.publisher.try_publish(BusChannel::Commands, BusFrame::command(cmd));
    if result == PublishResult::Queued {
        hold_published_read(state, deps, correlation_id, target);
    } else {
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
        requeue_target(state, target);
    }
}

fn hold_published_read(state: &mut PollerState, deps: &PollerWiring, correlation_id: u64, target: PollTarget) {
    let now = Instant::now();
    state.outstanding = Some(Outstanding {
        correlation_id,
        target,
        published_at: now,
        deadline: now + OUTSTANDING_ABANDON_TTL,
    });
    match target.purpose {
        ReadPurpose::Cycle => state.cursor = Some((target.adapter_id, target.short_address)),
        ReadPurpose::PowerCycleSweep => {
            deps.counters.sweep_reads.fetch_add(1, Ordering::Relaxed);
        }
    }
    deps.counters.reads_published.fetch_add(1, Ordering::Relaxed);
}

fn drain_confirmations(state: &mut PollerState, deps: &PollerWiring, conf_rx: &BusSubscriberRx) {
    while let Ok(frame) = conf_rx.try_recv() {
        let BusFrame::Confirmation(conf) = frame else {
            continue;
        };
        if conf.status == DeliveryStatus::Ok {
            continue;
        }
        if let Some(probe) = take_pending_probe(state, conf.meta.correlation_id) {
            end_probe_without_verdict(state, deps, &conf, probe);
            continue;
        }
        if conf.status != DeliveryStatus::DeliveryRejected {
            continue;
        }
        let Some(outstanding) = take_outstanding(state, conf.meta.correlation_id) else {
            continue;
        };
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
        requeue_target(state, outstanding.target);
    }
}

fn end_probe_without_verdict(state: &mut PollerState, deps: &PollerWiring, ended: &ConfirmationEnvelope, probe: PendingProbe) {
    if ended.status == DeliveryStatus::DeliveryRejected {
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
    } else if !refused_before_the_wire(ended) {
        charge_wire_time(state, probe.published_at.elapsed());
    }
}

fn refused_before_the_wire(ended: &ConfirmationEnvelope) -> bool {
    ended.confirmation.error.as_ref().is_some_and(|error| error.code == ErrorCode::Conflict)
}

fn sweep_stale(state: &mut PollerState, deps: &PollerWiring) {
    sweep_stale_probe(state, deps);
    let Some(o) = state.outstanding.filter(|o| o.deadline <= Instant::now()) else {
        return;
    };
    state.outstanding = None;
    deps.counters.outstanding_expired.fetch_add(1, Ordering::Relaxed);
    push_tombstone(state, o.correlation_id, o.published_at);
    close_sweep_read(state, o.target, &ReadVerdict::Failed);
}

fn sweep_stale_probe(state: &mut PollerState, deps: &PollerWiring) {
    if let Some(p) = state
        .pending_probe
        .filter(|p| p.deadline <= Instant::now())
    {
        state.pending_probe = None;
        deps.counters.health_probes_expired.fetch_add(1, Ordering::Relaxed);
        push_tombstone(state, p.correlation_id, p.published_at);
    }
}

fn push_tombstone(state: &mut PollerState, correlation_id: u64, published_at: Instant) {
    if state.tombstones.len() == CORRELATION_TOMBSTONES_MAX {
        state.tombstones.pop_front();
    }
    state.tombstones.push_back(CorrelationTombstone {
        correlation_id,
        published_at,
    });
}

fn take_tombstone(state: &mut PollerState, correlation_id: u64) -> Option<CorrelationTombstone> {
    let idx = state
        .tombstones
        .iter()
        .position(|t| t.correlation_id == correlation_id)?;
    state.tombstones.remove(idx)
}

fn take_outstanding(state: &mut PollerState, correlation_id: u64) -> Option<Outstanding> {
    let o = state
        .outstanding
        .filter(|o| o.correlation_id == correlation_id)?;
    state.outstanding = None;
    charge_wire_time(state, o.published_at.elapsed());
    Some(o)
}

fn take_pending_probe(state: &mut PollerState, correlation_id: u64) -> Option<PendingProbe> {
    let p = state
        .pending_probe
        .filter(|p| p.correlation_id == correlation_id)?;
    state.pending_probe = None;
    Some(p)
}

fn apply_event(state: &mut PollerState, deps: &PollerWiring, frame: &BusFrame) {
    let BusFrame::Event(envelope) = frame else {
        deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let corr = envelope.meta.correlation_id;
    dispatch_poller_event(&envelope.payload, state, deps, corr);
}

dali2rust_contracts::dispatch_bus_events! {
    pub const POLLER_HANDLED_EVENTS;
    fn dispatch_poller_event(
        payload: &BusEventPayload,
        state: &mut PollerState,
        deps: &PollerWiring,
        corr: u64,
    );
    payload = payload;
    ignored = { deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    PollerSettingsChangedEvent(_changed) => adopt_settings(state, deps.read_port.poller_settings_view()),
    RegistrySliceReloadedEvent(_reloaded) => adopt_settings(state, deps.read_port.poller_settings_view()),
    DaliAttributeReadOutcomesEvent(body) => complete_read(state, deps, corr, body),
    DaliBusHealthProbedEvent(body) => record_health_probe(state, deps, corr, body),
}

fn record_health_probe(
    state: &mut PollerState,
    deps: &PollerWiring,
    corr: u64,
    body: &DaliBusHealthProbedEvent,
) {
    if let Some(probe) = take_pending_probe(state, corr) {
        charge_wire_time(state, probe.published_at.elapsed());
    } else if let Some(t) = take_tombstone(state, corr) {
        charge_wire_time(state, t.published_at.elapsed());
        return;
    }
    if !body.control_answered {
        deps.counters.health_probes_invalid.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let counter = match body.lamp_failure {
        BusHealthVerdict::Clear => &deps.counters.health_probes_clear,
        BusHealthVerdict::One => &deps.counters.health_probes_one_failure,
        BusHealthVerdict::Several => &deps.counters.health_probes_several_failures,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    watch_power_cycles(state, deps, body);
}

fn watch_power_cycles(state: &mut PollerState, deps: &PollerWiring, body: &DaliBusHealthProbedEvent) {
    if !state.settings.power_cycle_watch {
        return;
    }
    if cycle_reads_status(&state.settings) {
        forget_absences_on_news(state, body);
    } else {
        sweep_after_power_cycles(state, deps, body);
    }
    if body.power_cycle_seen {
        deps.counters.health_probes_power_failure.fetch_add(1, Ordering::Relaxed);
    }
}

fn cycle_reads_status(settings: &PollerSettingsView) -> bool {
    let polled = DaliAttributeGroup::polled_mask(settings.attribute_groups_mask);
    settings.enabled && polled & DaliAttributeGroup::RuntimeStatus.mask_bit() != 0
}

fn forget_absences_on_news(state: &mut PollerState, body: &DaliBusHealthProbedEvent) {
    let adapter_id = body.registry_adapter_id;
    if state.power_cycles.news(adapter_id, body.power_cycle_seen) {
        state
            .cooldown
            .retain(|(adapter, _), cooldown| *adapter != adapter_id || cooldown.absent_strikes == 0);
    }
}

fn sweeps_power_cycles(settings: &PollerSettingsView) -> bool {
    settings.power_cycle_watch && !cycle_reads_status(settings)
}

fn sweep_after_power_cycles(state: &mut PollerState, deps: &PollerWiring, body: &DaliBusHealthProbedEvent) {
    let adapter_id = body.registry_adapter_id;
    let listed = if body.power_cycle_seen { listed_shorts(deps, adapter_id) } else { Vec::new() };
    let listed_set = listed.iter().fold(0, |set, short| set | short_bit(*short));
    if let Some(order) = state.power_cycles.answer(adapter_id, body.power_cycle_seen, listed_set) {
        let owed = listed.into_iter().filter(|short| order.owed & short_bit(*short) != 0);
        if queue_power_cycle_sweep(state, adapter_id, owed) && order.fresh {
            deps.counters.sweeps.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn listed_shorts(deps: &PollerWiring, adapter_id: u8) -> Vec<u8> {
    let targets = deps.read_port.list_poll_targets(adapter_id).targets;
    targets.iter().map(|target| target.short_address).collect()
}

fn queue_power_cycle_sweep(state: &mut PollerState, adapter_id: u8, owed: impl Iterator<Item = u8>) -> bool {
    let queued = state.sweep_queue.len();
    for short_address in owed {
        if sweep_holds(state, adapter_id, short_address) {
            continue;
        }
        state.sweep_queue.push_back(PollTarget {
            adapter_id,
            short_address,
            groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit(),
            memory_banks: MemoryBankReadPreset::None,
            purpose: ReadPurpose::PowerCycleSweep,
        });
    }
    state.sweep_queue.len() > queued
}

fn sweep_holds(state: &PollerState, adapter_id: u8, short_address: u8) -> bool {
    let is_it = |target: &PollTarget| target.adapter_id == adapter_id && target.short_address == short_address;
    state.sweep_queue.iter().any(is_it)
        || state
            .outstanding
            .is_some_and(|o| o.target.purpose == ReadPurpose::PowerCycleSweep && is_it(&o.target))
}

fn adopt_settings(state: &mut PollerState, settings: PollerSettingsView) {
    if !(sweeps_power_cycles(&state.settings) && sweeps_power_cycles(&settings)) {
        state.sweep_queue.clear();
        state.power_cycles = PowerCycleWatch::default();
    }
    state.settings = settings;
    state.next_cycle = state.last_cycle + cycle_period(&state.settings);
}

fn complete_read(state: &mut PollerState, deps: &PollerWiring, corr: u64, body: &DaliAttributeReadOutcomesEvent) {
    let Some(outstanding) = take_outstanding(state, corr) else {
        if let Some(t) = take_tombstone(state, corr) {
            charge_wire_time(state, t.published_at.elapsed());
        }
        return;
    };
    let target = outstanding.target;
    let verdict = read_verdict(body);
    match verdict {
        ReadVerdict::Completed => record_completed_read(state, &deps.counters, target),
        ReadVerdict::Preempted => {
            deps.counters.reads_preempted.fetch_add(1, Ordering::Relaxed);
            requeue_target(state, target);
        }
        ReadVerdict::Absent => cool_absent_device(state, &deps.counters, target.adapter_id, target.short_address),
        ReadVerdict::Contended | ReadVerdict::Failed => fail_device(state, &deps.counters, target.adapter_id, target.short_address),
    }
    close_sweep_read(state, target, &verdict);
}

fn close_sweep_read(state: &mut PollerState, target: PollTarget, verdict: &ReadVerdict) {
    if target.purpose != ReadPurpose::PowerCycleSweep {
        return;
    }
    match verdict {
        ReadVerdict::Completed | ReadVerdict::Absent => state.power_cycles.settle(target.adapter_id, target.short_address),
        ReadVerdict::Failed => state.power_cycles.fail(target.adapter_id, target.short_address),
        ReadVerdict::Contended | ReadVerdict::Preempted => {}
    }
}

fn record_completed_read(state: &mut PollerState, counters: &PollerCounters, target: PollTarget) {
    counters.reads_completed.fetch_add(1, Ordering::Relaxed);
    if target.memory_banks != MemoryBankReadPreset::None {
        state.bank_reads.insert(
            (target.adapter_id, target.short_address, target.memory_banks as u8),
            Instant::now(),
        );
    }
    state.cooldown.remove(&(target.adapter_id, target.short_address));
}

#[derive(Debug, PartialEq, Eq)]
enum ReadVerdict {
    Completed,
    Preempted,
    Absent,
    Contended,
    Failed,
}

fn read_verdict(body: &DaliAttributeReadOutcomesEvent) -> ReadVerdict {
    let sections = body.section_outcomes();
    if sections.contains(&AttributeGroupReadOutcome::Preempted) {
        ReadVerdict::Preempted
    } else if sections.contains(&AttributeGroupReadOutcome::DeviceAbsent) {
        ReadVerdict::Absent
    } else if sections
        .iter()
        .all(|o| matches!(o, AttributeGroupReadOutcome::NotRequested | AttributeGroupReadOutcome::Success))
    {
        ReadVerdict::Completed
    } else if lost_only_to_the_busy_wire(&sections) {
        ReadVerdict::Contended
    } else {
        ReadVerdict::Failed
    }
}

fn lost_only_to_the_busy_wire(sections: &[AttributeGroupReadOutcome]) -> bool {
    sections.contains(&AttributeGroupReadOutcome::ContendedAbort)
        && sections.iter().all(|o| {
            matches!(
                o,
                AttributeGroupReadOutcome::NotRequested
                    | AttributeGroupReadOutcome::Success
                    | AttributeGroupReadOutcome::ContendedAbort
                    | AttributeGroupReadOutcome::NotAttempted
            )
        })
}

fn requeue_target(state: &mut PollerState, target: PollTarget) {
    match target.purpose {
        ReadPurpose::Cycle => state.queue.push_back(target),
        ReadPurpose::PowerCycleSweep if state.power_cycles.owes(target.adapter_id, target.short_address) => {
            state.sweep_queue.push_back(target);
        }
        ReadPurpose::PowerCycleSweep => {}
    }
}

fn fail_device(state: &mut PollerState, counters: &PollerCounters, adapter_id: u8, short_address: u8) {
    let cooldown_ms = u64::from(state.settings.interval_ms) * u64::from(COOLDOWN_MULTIPLE);
    state.cooldown.insert(
        (adapter_id, short_address),
        DeviceCooldown {
            until: Instant::now() + Duration::from_millis(cooldown_ms),
            absent_strikes: 0,
        },
    );
    counters.reads_failed.fetch_add(1, Ordering::Relaxed);
    counters.device_cooldowns.fetch_add(1, Ordering::Relaxed);
}

fn cool_absent_device(state: &mut PollerState, counters: &PollerCounters, adapter_id: u8, short_address: u8) {
    let entry = state
        .cooldown
        .entry((adapter_id, short_address))
        .or_insert(DeviceCooldown {
            until: Instant::now(),
            absent_strikes: 0,
        });
    entry.absent_strikes = entry.absent_strikes.saturating_add(1);
    let cooldown_ms = u64::from(state.settings.interval_ms)
        * u64::from(absent_cooldown_multiple(entry.absent_strikes));
    entry.until = Instant::now() + Duration::from_millis(cooldown_ms);
    counters.reads_absent.fetch_add(1, Ordering::Relaxed);
    counters.device_cooldowns.fetch_add(1, Ordering::Relaxed);
}

pub struct ActiveRole;

impl dali2rust_domain::registry::DaliSettingsReadPort for ActiveRole {
    fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
        dali2rust_domain::registry::DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: true,
            device_short_address: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes(identity: AttributeGroupReadOutcome) -> DaliAttributeReadOutcomesEvent {
        let unrequested = AttributeGroupReadOutcome::NotRequested;
        DaliAttributeReadOutcomesEvent {
            registry_adapter_id: 0,
            short_address: 0,
            identity,
            runtime_status: unrequested,
            common_102: unrequested,
            dt8_color: unrequested,
            dt6_led: unrequested,
            groups: unrequested,
            scenes: unrequested,
            extended: unrequested,
            memory_banks: unrequested,
            scene_colours: unrequested,
        }
    }

    #[test]
    fn an_absent_device_is_its_own_verdict_not_a_generic_failure() {
        assert_eq!(
            read_verdict(&outcomes(AttributeGroupReadOutcome::DeviceAbsent)),
            ReadVerdict::Absent
        );
        assert_eq!(
            read_verdict(&outcomes(AttributeGroupReadOutcome::Success)),
            ReadVerdict::Completed
        );
        assert_eq!(
            read_verdict(&outcomes(AttributeGroupReadOutcome::Preempted)),
            ReadVerdict::Preempted,
            "a yield is still not a device fault"
        );
        assert_eq!(
            read_verdict(&outcomes(AttributeGroupReadOutcome::TransportAbort)),
            ReadVerdict::Failed,
            "a transport abort keeps the flat failure cooldown"
        );
        assert_eq!(read_verdict(&outcomes(AttributeGroupReadOutcome::ContendedAbort)), ReadVerdict::Contended);
    }

    #[test]
    fn a_read_is_lost_to_the_busy_wire_only_when_nothing_else_failed() {
        let mut contended = outcomes(AttributeGroupReadOutcome::ContendedAbort);
        contended.runtime_status = AttributeGroupReadOutcome::NotAttempted;
        assert_eq!(read_verdict(&contended), ReadVerdict::Contended, "what the abort left unread is the wire's too");
        let mut mixed = outcomes(AttributeGroupReadOutcome::ContendedAbort);
        mixed.runtime_status = AttributeGroupReadOutcome::TransportAbort;
        assert_eq!(read_verdict(&mixed), ReadVerdict::Failed);
        assert_eq!(
            read_verdict(&outcomes(AttributeGroupReadOutcome::NotAttempted)),
            ReadVerdict::Failed,
            "a read never started without a contended abort is no busy wire"
        );
    }

    #[test]
    fn scene_colours_are_never_polled_whatever_the_settings_hold() {
        let settings = PollerSettingsView {
            enabled: true,
            interval_ms: 5_000,
            attribute_groups_mask: DaliAttributeGroup::RuntimeStatus.mask_bit()
                | DaliAttributeGroup::SceneColours.mask_bit(),
            include_dt8_color: true,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: true,
            power_cycle_watch: false,
        };
        let target = PollTargetView {
            short_address: 3,
            is_dt8: true,
            is_dt6: false,
            declares_energy: false,
            declares_diagnostics: false,
        };
        let mask = groups_mask_for(&settings, &target);
        assert_eq!(mask & DaliAttributeGroup::SceneColours.mask_bit(), 0);
        assert_ne!(mask & DaliAttributeGroup::RuntimeStatus.mask_bit(), 0);
        assert_ne!(mask & DaliAttributeGroup::Dt8Color.mask_bit(), 0);
    }

    #[test]
    fn absent_cooldown_escalates_and_caps() {
        assert_eq!(absent_cooldown_multiple(1), 3);
        assert_eq!(absent_cooldown_multiple(2), 6);
        assert_eq!(absent_cooldown_multiple(3), 12);
        assert_eq!(absent_cooldown_multiple(4), 12, "capped, not unbounded");
        assert_eq!(absent_cooldown_multiple(u8::MAX), 12);
    }

    fn test_state() -> PollerState {
        PollerState::boot(PollerSettingsView {
            enabled: true,
            interval_ms: 100,
            attribute_groups_mask: 0,
            include_dt8_color: false,
            include_energy: false,
            include_diagnostics: false,
            skip_unbound_virtual_lamps: false,
            power_cycle_watch: false,
        })
    }

    #[test]
    fn success_removes_the_cooldown_entry_and_its_strikes() {
        let mut state = test_state();
        let counters = PollerCounters::default();
        cool_absent_device(&mut state, &counters, 0, 7);
        cool_absent_device(&mut state, &counters, 0, 7);
        assert_eq!(state.cooldown[&(0, 7)].absent_strikes, 2);

        state.cooldown.remove(&(0, 7));
        assert!(state.cooldown.is_empty());
        cool_absent_device(&mut state, &counters, 0, 7);
        assert_eq!(
            state.cooldown[&(0, 7)].absent_strikes, 1,
            "strikes restart from scratch after a success"
        );
        assert_eq!(counters.reads_absent.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn delisted_devices_lose_their_cooldown_but_listed_keep_strikes() {
        let mut state = test_state();
        let counters = PollerCounters::default();
        cool_absent_device(&mut state, &counters, 0, 7);
        cool_absent_device(&mut state, &counters, 0, 9);
        let listed: std::collections::HashSet<(u8, u8)> = [(0u8, 9u8)].into_iter().collect();
        prune_delisted_device_state(&mut state, &listed);
        assert!(!state.cooldown.contains_key(&(0, 7)), "delisted -> forgotten");
        assert_eq!(
            state.cooldown[&(0, 9)].absent_strikes, 1,
            "still listed -> strike memory survives (lapse-pruning would erase it)"
        );
    }

    use std::sync::atomic::AtomicBool;

    use crate::runtime::power_cycle::{RESWEEP_AFTER_YES, SWEEP_READ_ATTEMPTS};

    use dali2rust_bus::{BusConfig, BusHost};
    use dali2rust_domain::registry::{PollTargetReadPort, PollTargetSelection, PollerSettingsReadPort};

    struct NullPort;

    impl PollTargetReadPort for NullPort {
        fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
            PollTargetSelection { targets: Vec::new(), excluded_unbound: 0, adapter_enabled: true }
        }
    }

    impl PollerSettingsReadPort for NullPort {
        fn poller_settings_view(&self) -> PollerSettingsView {
            test_state().settings
        }
    }

    struct TestDeps {
        deps: PollerWiring,
        _host: BusHost,
    }

    fn test_deps() -> TestDeps {
        test_deps_on(Arc::new(NullPort))
    }

    fn test_deps_on(read_port: Arc<dyn PollerReadPort>) -> TestDeps {
        let (_host, publisher, ()) = BusHost::spawn(BusConfig::default(), |_| ());
        let deps = PollerWiring {
            role: Arc::new(ActiveRole),
            publisher,
            read_port,
            correlation: Arc::new(CorrelationIdAllocator::new()),
            bus_id: BusId::default(),
            adapter_count: 1,
            counters: Arc::new(PollerCounters::default()),
            interactive: Arc::new(dali2rust_platform::dali::WireActivity::new()),
            hold: Arc::new(dali2rust_platform::firmware::MaintenanceHold::new()),
            config: PollerConfig::default(),
        };
        TestDeps { deps, _host }
    }

    fn poll_target(short_address: u8) -> PollTarget {
        PollTarget {
            adapter_id: 0,
            short_address,
            groups_mask: 0,
            memory_banks: MemoryBankReadPreset::None,
            purpose: ReadPurpose::Cycle,
        }
    }

    #[test]
    fn an_armed_read_or_probe_outlives_a_commissioning_bracket() {
        let t = test_deps();
        let mut state = test_state();
        publish_read(&mut state, &t.deps, poll_target(9));
        let o = state.outstanding.expect("armed read");
        assert!(
            o.deadline.duration_since(o.published_at) >= Duration::from_secs(15 * 60),
            "the slot must outlive the longest legal queue wait (ADR-013: Never granularity, ~15 min)"
        );
        state.probe_due.push_back(0);
        publish_health_probe(&mut state, &t.deps);
        let p = state.pending_probe.expect("armed probe");
        assert!(
            p.deadline.duration_since(p.published_at) >= Duration::from_secs(15 * 60),
            "the probe rides the same queue and is parked by the same brackets"
        );
    }

    #[test]
    fn a_late_outcome_for_an_abandoned_read_still_charges_wire_time() {
        let t = test_deps();
        let mut state = test_state();
        publish_read(&mut state, &t.deps, poll_target(9));
        let corr = state.outstanding.expect("armed").correlation_id;
        {
            let o = state.outstanding.as_mut().expect("armed");
            o.published_at = Instant::now() - Duration::from_secs(10);
            o.deadline = Instant::now() - Duration::from_millis(1);
        }
        sweep_stale(&mut state, &t.deps);
        assert!(state.outstanding.is_none(), "the leak guard still frees the slot");
        assert_eq!(t.deps.counters.outstanding_expired.load(Ordering::Relaxed), 1);

        let before = Instant::now();
        complete_read(&mut state, &t.deps, corr, &outcomes(AttributeGroupReadOutcome::Success));
        assert!(
            state.rest_until >= before + Duration::from_secs(20),
            "a ~10 s read owes ~3x itself in rest even when it outlived the guard"
        );
        assert_eq!(
            t.deps.counters.reads_completed.load(Ordering::Relaxed),
            0,
            "already accounted as expired: the late outcome settles the bill, not the ledger"
        );
    }

    fn gear_view(short_address: u8) -> PollTargetView {
        PollTargetView {
            short_address,
            is_dt8: false,
            is_dt6: false,
            declares_energy: false,
            declares_diagnostics: false,
        }
    }

    struct ListsGears(&'static [u8]);

    impl PollTargetReadPort for ListsGears {
        fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
            let targets = self.0.iter().map(|short| gear_view(*short)).collect();
            PollTargetSelection { targets, excluded_unbound: 0, adapter_enabled: true }
        }
    }

    impl PollerSettingsReadPort for ListsGears {
        fn poller_settings_view(&self) -> PollerSettingsView {
            watch_state().settings
        }
    }

    fn watch_state() -> PollerState {
        let mut state = test_state();
        state.settings.power_cycle_watch = true;
        state
    }

    fn power_cycle_yes(state: &mut PollerState, deps: &PollerWiring) {
        let yes = DaliBusHealthProbedEvent {
            registry_adapter_id: 0,
            control_answered: true,
            lamp_failure: BusHealthVerdict::Clear,
            power_cycle_seen: true,
        };
        record_health_probe(state, deps, 0, &yes);
    }

    #[test]
    fn a_sweep_read_the_guard_expired_waits_for_the_next_yes_not_for_the_next_slot() {
        let t = test_deps_on(Arc::new(ListsGears(&[1])));
        let mut state = watch_state();
        power_cycle_yes(&mut state, &t.deps);
        let target = state.sweep_queue.pop_front().expect("the YES queues gear 1");
        publish_read(&mut state, &t.deps, target);
        state.outstanding.as_mut().expect("in flight").deadline = Instant::now() - Duration::from_millis(1);
        sweep_stale(&mut state, &t.deps);
        assert_eq!(t.deps.counters.outstanding_expired.load(Ordering::Relaxed), 1);
        assert!(
            state.sweep_queue.is_empty(),
            "a read nobody answered is not retried at once, where it could hold the slot for another guard"
        );
        power_cycle_yes(&mut state, &t.deps);
        let owed: Vec<u8> = state.sweep_queue.iter().map(|target| target.short_address).collect();
        assert_eq!(owed, vec![1], "the gear is still owed, so the next YES reads it");
        assert_eq!(t.deps.counters.sweeps.load(Ordering::Relaxed), 1, "the same sweep");
    }

    struct TwoAdapters {
        zero_enabled: AtomicBool,
    }

    impl PollTargetReadPort for TwoAdapters {
        fn list_poll_targets(&self, adapter_id: u8) -> PollTargetSelection {
            let enabled = adapter_id != 0 || self.zero_enabled.load(Ordering::Relaxed);
            let targets = if enabled { vec![gear_view(1)] } else { Vec::new() };
            PollTargetSelection { targets, excluded_unbound: 0, adapter_enabled: enabled }
        }
    }

    impl PollerSettingsReadPort for TwoAdapters {
        fn poller_settings_view(&self) -> PollerSettingsView {
            test_state().settings
        }
    }

    fn pump_after_rest(state: &mut PollerState, deps: &PollerWiring) -> Option<u8> {
        state.rest_until = Instant::now();
        pump_queue(state, deps);
        state.outstanding.map(|o| o.target.adapter_id)
    }

    #[test]
    fn a_cycle_read_the_guard_expired_waits_for_the_next_cycle_not_for_the_next_slot() {
        let port = Arc::new(TwoAdapters { zero_enabled: AtomicBool::new(true) });
        let mut t = test_deps_on(Arc::clone(&port) as Arc<dyn PollerReadPort>);
        t.deps.adapter_count = 2;
        let mut state = test_state();
        start_cycle(&mut state, &t.deps);
        assert_eq!(pump_after_rest(&mut state, &t.deps), Some(0));
        port.zero_enabled.store(false, Ordering::Relaxed);
        state.outstanding.as_mut().expect("in flight").deadline = Instant::now() - Duration::from_millis(1);
        sweep_stale(&mut state, &t.deps);
        assert_eq!(pump_after_rest(&mut state, &t.deps), Some(1));
        let corr = state.outstanding.expect("adapter 1 in flight").correlation_id;
        complete_read(&mut state, &t.deps, corr, &outcomes(AttributeGroupReadOutcome::Success));
        assert_eq!(
            pump_after_rest(&mut state, &t.deps),
            None,
            "a read nobody answered is not retried in this cycle, where a disabled adapter's read would take the slot back after every guard"
        );
        port.zero_enabled.store(true, Ordering::Relaxed);
        start_cycle(&mut state, &t.deps);
        let listed: Vec<u8> = state.queue.iter().map(|target| target.adapter_id).collect();
        assert_eq!(listed, vec![0, 1], "the next cycle lists it again, in its turn after the cursor");
    }

    fn preempt_the_sweep_read_in_flight(state: &mut PollerState, deps: &PollerWiring) {
        let target = state.sweep_queue.pop_front().expect("a sweep read to publish");
        publish_read(state, deps, target);
        let corr = state.outstanding.expect("in flight").correlation_id;
        complete_read(state, deps, corr, &outcomes(AttributeGroupReadOutcome::Preempted));
    }

    #[test]
    fn a_preempted_sweep_read_returns_only_while_the_watch_still_owes_its_target() {
        let port = Arc::new(TwoAdapters { zero_enabled: AtomicBool::new(true) });
        let t = test_deps_on(Arc::clone(&port) as Arc<dyn PollerReadPort>);
        let mut state = watch_state();
        power_cycle_yes(&mut state, &t.deps);
        preempt_the_sweep_read_in_flight(&mut state, &t.deps);
        assert_eq!(state.sweep_queue.len(), 1, "an owed target yields to the operator and goes back to the sweep");
        let target = state.sweep_queue.pop_front().expect("owed");
        publish_read(&mut state, &t.deps, target);
        let corr = state.outstanding.expect("in flight").correlation_id;
        port.zero_enabled.store(false, Ordering::Relaxed);
        start_probe_round(&mut state, &t.deps);
        complete_read(&mut state, &t.deps, corr, &outcomes(AttributeGroupReadOutcome::Preempted));
        assert!(
            state.sweep_queue.is_empty(),
            "the round dropped the disabled adapter, so its preempted read is not put back to hold the slot for a guard"
        );
        assert_eq!(pump_after_rest(&mut state, &t.deps), None);
    }

    #[test]
    fn a_preempted_sweep_read_after_a_no_is_not_requeued() {
        let t = test_deps_on(Arc::new(ListsGears(&[1])));
        let mut state = watch_state();
        power_cycle_yes(&mut state, &t.deps);
        let target = state.sweep_queue.pop_front().expect("the YES queues gear 1");
        publish_read(&mut state, &t.deps, target);
        let corr = state.outstanding.expect("in flight").correlation_id;
        let no = DaliBusHealthProbedEvent {
            registry_adapter_id: 0,
            control_answered: true,
            lamp_failure: BusHealthVerdict::Clear,
            power_cycle_seen: false,
        };
        record_health_probe(&mut state, &t.deps, 0, &no);
        complete_read(&mut state, &t.deps, corr, &outcomes(AttributeGroupReadOutcome::Preempted));
        assert!(state.sweep_queue.is_empty(), "a NO ended the sweep: no gear's bit is set to read");
    }

    fn read_the_next_sweep_target(state: &mut PollerState, deps: &PollerWiring, outcome: AttributeGroupReadOutcome) -> u8 {
        let target = state.sweep_queue.pop_front().expect("a sweep target");
        let corr = deps.correlation.next_id();
        hold_published_read(state, deps, corr, target);
        complete_read(state, deps, corr, &outcomes(outcome));
        target.short_address
    }

    fn sweeps(deps: &PollerWiring) -> u32 {
        deps.counters.sweeps.load(Ordering::Relaxed)
    }

    #[test]
    fn a_target_whose_sweep_reads_keep_failing_leaves_the_sweep_to_the_hold_off() {
        let t = test_deps_on(Arc::new(ListsGears(&[1, 2])));
        let mut state = watch_state();
        power_cycle_yes(&mut state, &t.deps);
        assert_eq!(read_the_next_sweep_target(&mut state, &t.deps, AttributeGroupReadOutcome::TransportAbort), 1);
        assert_eq!(read_the_next_sweep_target(&mut state, &t.deps, AttributeGroupReadOutcome::Success), 2);
        for _ in 0..RESWEEP_AFTER_YES * 2 {
            power_cycle_yes(&mut state, &t.deps);
            if sweeps(&t.deps) > 1 {
                break;
            }
            if !state.sweep_queue.is_empty() {
                assert_eq!(read_the_next_sweep_target(&mut state, &t.deps, AttributeGroupReadOutcome::TransportAbort), 1);
            }
        }
        assert_eq!(
            sweeps(&t.deps),
            2,
            "gear 1 left the sweep after its failed reads, so the hold-off ran out and a new cycle of gear 2 is swept"
        );
        let owed: Vec<u8> = state.sweep_queue.iter().map(|target| target.short_address).collect();
        assert_eq!(owed, vec![1, 2], "the fresh sweep asks gear 1 again too");
    }

    #[test]
    fn a_sweep_read_the_busy_wire_aborted_says_nothing_about_the_gear() {
        let t = test_deps_on(Arc::new(ListsGears(&[1])));
        let mut state = watch_state();
        for _ in 0..SWEEP_READ_ATTEMPTS {
            power_cycle_yes(&mut state, &t.deps);
            assert_eq!(read_the_next_sweep_target(&mut state, &t.deps, AttributeGroupReadOutcome::ContendedAbort), 1);
        }
        power_cycle_yes(&mut state, &t.deps);
        let owed: Vec<u8> = state.sweep_queue.iter().map(|target| target.short_address).collect();
        assert_eq!(owed, vec![1], "a contended abort is the wire's, like a preemption, so the gear stays owed");
        assert_eq!(
            t.deps.counters.reads_failed.load(Ordering::Relaxed),
            u32::from(SWEEP_READ_ATTEMPTS),
            "it is still a failed read with its cooldown"
        );
    }

    #[test]
    fn a_sweep_read_the_guard_expires_counts_as_a_failed_one() {
        let t = test_deps_on(Arc::new(ListsGears(&[1])));
        let mut state = watch_state();
        for _ in 0..SWEEP_READ_ATTEMPTS {
            power_cycle_yes(&mut state, &t.deps);
            let target = state.sweep_queue.pop_front().expect("gear 1 is still owed");
            hold_published_read(&mut state, &t.deps, t.deps.correlation.next_id(), target);
            state.outstanding.as_mut().expect("in flight").deadline = Instant::now() - Duration::from_millis(1);
            sweep_stale(&mut state, &t.deps);
        }
        power_cycle_yes(&mut state, &t.deps);
        assert!(state.sweep_queue.is_empty(), "a read that never returns leaves the sweep like one that keeps failing");
    }

    fn cycle_state() -> PollerState {
        let mut state = watch_state();
        state.settings.attribute_groups_mask = DaliAttributeGroup::RuntimeStatus.mask_bit();
        state.settings.interval_ms = 60_000;
        state
    }

    fn cycle_lists(state: &mut PollerState, deps: &PollerWiring) -> Vec<u8> {
        start_cycle(state, deps);
        state.queue.iter().map(|target| target.short_address).collect()
    }

    #[test]
    fn a_yes_that_is_news_lets_the_cycle_read_a_gear_it_saw_absent_through_the_outage() {
        let t = test_deps_on(Arc::new(ListsGears(&[1, 2])));
        let mut state = cycle_state();
        for _ in 0..ABSENT_COOLDOWN_MULTIPLES.len() {
            cool_absent_device(&mut state, &t.deps.counters, 0, 1);
        }
        fail_device(&mut state, &t.deps.counters, 0, 2);
        assert!(cycle_lists(&mut state, &t.deps).is_empty(), "gear 1 cools for twelve intervals, gear 2 for three");
        power_cycle_yes(&mut state, &t.deps);
        assert_eq!(
            cycle_lists(&mut state, &t.deps),
            vec![1],
            "the YES says the outage is over, so the absent cooldown is stale; a failed read's cooldown is not about absence"
        );
        assert!(state.sweep_queue.is_empty() && sweeps(&t.deps) == 0, "the cycle reads the status, so nothing is swept");
        cool_absent_device(&mut state, &t.deps.counters, 0, 1);
        for answer in 1..RESWEEP_AFTER_YES {
            power_cycle_yes(&mut state, &t.deps);
            assert!(
                cycle_lists(&mut state, &t.deps).is_empty(),
                "standing YES number {answer} is not news, so the absent cooldown holds"
            );
        }
        power_cycle_yes(&mut state, &t.deps);
        assert_eq!(cycle_lists(&mut state, &t.deps), vec![1], "news again where a sweep would sweep again");
    }

    #[test]
    fn a_poller_that_stops_reading_the_status_sweeps_at_the_first_yes() {
        let t = test_deps_on(Arc::new(ListsGears(&[1])));
        let mut state = cycle_state();
        power_cycle_yes(&mut state, &t.deps);
        let mut stopped = state.settings;
        stopped.enabled = false;
        adopt_settings(&mut state, stopped);
        power_cycle_yes(&mut state, &t.deps);
        assert_eq!(
            sweeps(&t.deps),
            1,
            "the hold-off the cycle began is not the sweep's: the cycle may not have read every gear yet"
        );
    }

    #[test]
    fn a_probe_refused_before_the_wire_owes_no_rest_and_one_that_reached_it_does() {
        let t = test_deps();
        for (code, reached_the_wire) in [
            (ErrorCode::Conflict, false),
            (ErrorCode::OperationFailed, true),
            (ErrorCode::Preempted, true),
        ] {
            let mut state = test_state();
            let probe = PendingProbe {
                correlation_id: 1,
                published_at: Instant::now() - Duration::from_secs(10),
                deadline: Instant::now() + OUTSTANDING_ABANDON_TTL,
            };
            let ended = dali2rust_contracts::bus::build_confirmation_envelope_with_product_error(
                1,
                DeliveryStatus::ExecutionFailed,
                0,
                SOURCE_ID_UNSPECIFIED,
                Some((code, "cause")),
            );
            let before = Instant::now();
            end_probe_without_verdict(&mut state, &t.deps, &ended, probe);
            assert_eq!(
                state.rest_until >= before + Duration::from_secs(20),
                reached_the_wire,
                "{code:?}: only a probe that reached the wire buys rest; a gate's refusal sent nothing"
            );
        }
    }

    #[test]
    fn a_late_probe_verdict_charges_once_and_counts_nowhere_twice() {
        let t = test_deps();
        let mut state = test_state();
        state.probe_due.push_back(0);
        publish_health_probe(&mut state, &t.deps);
        let corr = state.pending_probe.expect("armed").correlation_id;
        {
            let p = state.pending_probe.as_mut().expect("armed");
            p.published_at = Instant::now() - Duration::from_secs(10);
            p.deadline = Instant::now() - Duration::from_millis(1);
        }
        sweep_stale_probe(&mut state, &t.deps);
        assert_eq!(t.deps.counters.health_probes_expired.load(Ordering::Relaxed), 1);

        let before = Instant::now();
        let body = DaliBusHealthProbedEvent {
            registry_adapter_id: 0,
            control_answered: true,
            lamp_failure: BusHealthVerdict::Clear,
            power_cycle_seen: false,
        };
        record_health_probe(&mut state, &t.deps, corr, &body);
        assert!(
            state.rest_until >= before + Duration::from_secs(20),
            "the two frames were real wire time"
        );
        assert_eq!(
            t.deps.counters.health_probes_clear.load(Ordering::Relaxed),
            0,
            "expired + clear for one probe would double-count it"
        );
    }

    #[test]
    fn a_cheaper_later_charge_extends_rest_it_never_shortens_it() {
        let mut state = test_state();
        charge_wire_time(&mut state, Duration::from_secs(1));
        let standing = state.rest_until;
        charge_wire_time(&mut state, Duration::from_millis(1));
        assert!(
            state.rest_until >= standing,
            "a later, cheaper charge must not erase standing rest"
        );
    }

    #[test]
    fn a_charge_is_capped_at_the_guard_window_not_the_queue_wait() {
        let mut state = test_state();
        let now = Instant::now();
        charge_wire_time(&mut state, Duration::from_secs(600));
        assert!(
            state.rest_until <= now + Duration::from_secs(95),
            "3x the 30 s chargeable ceiling, never 3x a ten-minute park"
        );
    }

    #[test]
    fn a_delisted_device_loses_its_bank_read_memo_too() {
        let mut state = test_state();
        state
            .bank_reads
            .insert((0, 7, MemoryBankReadPreset::LuminaireData as u8), Instant::now());
        state
            .bank_reads
            .insert((0, 9, MemoryBankReadPreset::Power as u8), Instant::now());
        let listed: std::collections::HashSet<(u8, u8)> = [(0u8, 9u8)].into_iter().collect();
        prune_delisted_device_state(&mut state, &listed);
        assert!(
            !state
                .bank_reads
                .contains_key(&(0, 7, MemoryBankReadPreset::LuminaireData as u8)),
            "delisted -> the one-shot bank 207 memo resets with the device"
        );
        assert!(state
            .bank_reads
            .contains_key(&(0, 9, MemoryBankReadPreset::Power as u8)));
    }

    #[test]
    fn a_settings_change_cannot_shrink_the_period_below_the_floor() {
        let mut state = test_state();
        let mut settings = state.settings;
        settings.interval_ms = 0;
        adopt_settings(&mut state, settings);
        assert!(
            state.next_cycle.duration_since(state.last_cycle) >= Duration::from_millis(200),
            "the REST floor (200 ms) holds even for a crafted registry view"
        );
    }
}
