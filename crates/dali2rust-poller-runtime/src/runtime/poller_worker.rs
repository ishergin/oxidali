use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, CorrelationIdAllocator, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusEventPayload, DaliAttributeGroup,
    BusHealthVerdict, DaliAttributeReadOutcomesEvent, DaliBusHealthProbeCommand,
    DaliBusHealthProbedEvent, DaliReadAttributesCommand, DeliveryStatus,
    MemoryBankReadPreset, Origin, PollerSettingsChangedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{PollTargetView, PollerReadPort, PollerSettingsView};

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

#[derive(Clone, Copy, Debug)]
struct PollTarget {
    adapter_id: u8,
    short_address: u8,
    groups_mask: u8,
    memory_banks: MemoryBankReadPreset,
}

#[derive(Clone, Copy, Debug)]
struct Outstanding {
    correlation_id: u64,
    adapter_id: u8,
    short_address: u8,
    groups_mask: u8,
    memory_banks: MemoryBankReadPreset,
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
    outstanding: Option<Outstanding>,
    probe_due: VecDeque<u8>,
    pending_probe: Option<PendingProbe>,
    probe_cursor: u8,
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
            outstanding: None,
            probe_due: VecDeque::new(),
            pending_probe: None,
            probe_cursor: 0,
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

struct PollerDeps {
    publisher: BusPublisher,
    read_port: Arc<dyn PollerReadPort>,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    adapter_count: u8,
    counters: Arc<PollerCounters>,
    role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
}

#[allow(clippy::too_many_arguments, reason = "one param per wired dependency; a struct here would only move the list")]
pub fn spawn_poller_worker(
    ev_rx: BusSubscriberRx,
    conf_rx: BusSubscriberRx,
    publisher: BusPublisher,
    read_port: Arc<dyn PollerReadPort>,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    adapter_count: u8,
    counters: Arc<PollerCounters>,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    hold: Arc<dali2rust_platform::firmware::MaintenanceHold>,
) -> std::thread::JoinHandle<()> {
    let deps = PollerDeps {
        publisher,
        read_port,
        correlation,
        bus_id,
        adapter_count,
        counters,
        interactive,
        role,
        hold,
    };
    esp_thread::spawn_named_stack_in(
        c"poller",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || run_loop(ev_rx, conf_rx, deps),
    )
}

fn run_loop(ev_rx: BusSubscriberRx, conf_rx: BusSubscriberRx, deps: PollerDeps) {
    let mut state = PollerState::boot(deps.read_port.poller_settings_view());
    loop {
        if Instant::now() >= state.next_cycle {
            start_cycle(&mut state, &deps);
        }
        let wait = state
            .next_cycle
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

fn cycle_period(settings: &PollerSettingsView) -> Duration {
    Duration::from_millis(u64::from(settings.interval_ms.max(MIN_INTERVAL_MS)))
}

fn probe_rota(cursor: u8, adapter_count: u8) -> VecDeque<u8> {
    (0..adapter_count)
        .map(|i| ((u16::from(cursor) + u16::from(i)) % u16::from(adapter_count)) as u8)
        .collect()
}

fn start_cycle(state: &mut PollerState, deps: &PollerDeps) {
    deps.counters.cycles_total.fetch_add(1, Ordering::Relaxed);
    state.last_cycle = Instant::now();
    state.next_cycle = state.last_cycle + cycle_period(&state.settings);
    if !state.queue.is_empty() {
        deps.counters.window_deferred.fetch_add(1, Ordering::Relaxed);
    }
    state.queue = if state.settings.enabled && deps.role.dali_settings_view().application_active {
        state.probe_due = probe_rota(state.probe_cursor, deps.adapter_count);
        select_targets(state, deps)
    } else {
        deps.counters.targets_excluded.store(0, Ordering::Relaxed);
        state.probe_due.clear();
        VecDeque::new()
    };
}

fn publish_health_probe(state: &mut PollerState, deps: &PollerDeps) {
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

fn select_targets(state: &mut PollerState, deps: &PollerDeps) -> VecDeque<PollTarget> {
    let mut queue = VecDeque::new();
    let mut listed: std::collections::HashSet<(u8, u8)> = std::collections::HashSet::new();
    let mut excluded: u32 = 0;
    for adapter_id in 0..deps.adapter_count {
        let selection = deps.read_port.list_poll_targets(adapter_id);
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
            });
        }
    }
    deps.counters.targets_excluded.store(excluded, Ordering::Relaxed);
    prune_delisted_device_state(state, &listed);
    rotate_after_cursor(queue, state.cursor)
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
    state
        .outstanding
        .is_some_and(|o| o.adapter_id == adapter_id && o.short_address == short_address)
}

fn groups_mask_for(settings: &PollerSettingsView, target: &PollTargetView) -> u8 {
    let mut mask = settings.attribute_groups_mask;
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

fn pump_queue(state: &mut PollerState, deps: &PollerDeps) {
    if !state.settings.enabled {
        state.queue.clear();
        state.probe_due.clear();
        return;
    }
    if deps.hold.engaged() {
        return;
    }
    let probe_ready = state.pending_probe.is_none() && !state.probe_due.is_empty();
    let read_ready = state.outstanding.is_none() && !state.queue.is_empty();
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
    if read_ready {
        let target = state.queue.pop_front().expect("read_ready");
        publish_read(state, deps, target);
    }
}

fn charge_duty_deferral(state: &mut PollerState, deps: &PollerDeps) {
    if state.duty_deferred_for == Some(state.rest_until) {
        return;
    }
    state.duty_deferred_for = Some(state.rest_until);
    deps.counters.duty_deferred.fetch_add(1, Ordering::Relaxed);
}

fn charge_interactive_deferral(state: &mut PollerState, deps: &PollerDeps) {
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

fn publish_read(state: &mut PollerState, deps: &PollerDeps, target: PollTarget) {
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
        let now = Instant::now();
        state.outstanding = Some(Outstanding {
            correlation_id,
            adapter_id: target.adapter_id,
            short_address: target.short_address,
            groups_mask: target.groups_mask,
            memory_banks: target.memory_banks,
            published_at: now,
            deadline: now + OUTSTANDING_ABANDON_TTL,
        });
        state.cursor = Some((target.adapter_id, target.short_address));
        deps.counters.reads_published.fetch_add(1, Ordering::Relaxed);
    } else {
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
        state.queue.push_back(target);
    }
}

fn drain_confirmations(state: &mut PollerState, deps: &PollerDeps, conf_rx: &BusSubscriberRx) {
    while let Ok(frame) = conf_rx.try_recv() {
        let BusFrame::Confirmation(conf) = frame else {
            continue;
        };
        if conf.status != DeliveryStatus::DeliveryRejected {
            continue;
        }
        if take_pending_probe(state, conf.meta.correlation_id).is_some() {
            deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let Some(outstanding) = take_outstanding(state, conf.meta.correlation_id) else {
            continue;
        };
        deps.counters.skipped_inbox_full.fetch_add(1, Ordering::Relaxed);
        requeue_target(state, &outstanding);
    }
}

fn sweep_stale(state: &mut PollerState, deps: &PollerDeps) {
    sweep_stale_probe(state, deps);
    let Some(o) = state.outstanding.filter(|o| o.deadline <= Instant::now()) else {
        return;
    };
    state.outstanding = None;
    deps.counters.outstanding_expired.fetch_add(1, Ordering::Relaxed);
    push_tombstone(state, o.correlation_id, o.published_at);
    requeue_target(state, &o);
}

fn sweep_stale_probe(state: &mut PollerState, deps: &PollerDeps) {
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

fn apply_event(state: &mut PollerState, deps: &PollerDeps, frame: &BusFrame) {
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
        deps: &PollerDeps,
        corr: u64,
    );
    payload = payload;
    ignored = { deps.counters.ignored_events.fetch_add(1, Ordering::Relaxed); };
    PollerSettingsChangedEvent(body) => apply_settings_changed(state, body),
    RegistrySliceReloadedEvent(_reloaded) => adopt_settings(state, deps.read_port.poller_settings_view()),
    DaliAttributeReadOutcomesEvent(body) => complete_read(state, deps, corr, body),
    DaliBusHealthProbedEvent(body) => record_health_probe(state, deps, corr, body),
}

fn record_health_probe(
    state: &mut PollerState,
    deps: &PollerDeps,
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
}

fn apply_settings_changed(state: &mut PollerState, body: &PollerSettingsChangedEvent) {
    adopt_settings(
        state,
        PollerSettingsView {
            enabled: body.enabled,
            interval_ms: body.interval_ms,
            attribute_groups_mask: body.attribute_groups_mask,
            include_dt8_color: body.include_dt8_color,
            include_energy: body.include_energy,
            include_diagnostics: body.include_diagnostics,
            skip_unbound_virtual_lamps: body.skip_unbound_virtual_lamps,
        },
    );
}

fn adopt_settings(state: &mut PollerState, settings: PollerSettingsView) {
    state.settings = settings;
    state.next_cycle = state.last_cycle + cycle_period(&state.settings);
}

fn complete_read(state: &mut PollerState, deps: &PollerDeps, corr: u64, body: &DaliAttributeReadOutcomesEvent) {
    let Some(outstanding) = take_outstanding(state, corr) else {
        if let Some(t) = take_tombstone(state, corr) {
            charge_wire_time(state, t.published_at.elapsed());
        }
        return;
    };
    match read_verdict(body) {
        ReadVerdict::Completed => {
            deps.counters.reads_completed.fetch_add(1, Ordering::Relaxed);
            if outstanding.memory_banks != MemoryBankReadPreset::None {
                state.bank_reads.insert(
                    (
                        outstanding.adapter_id,
                        outstanding.short_address,
                        outstanding.memory_banks as u8,
                    ),
                    Instant::now(),
                );
            }
            state
                .cooldown
                .remove(&(outstanding.adapter_id, outstanding.short_address));
        }
        ReadVerdict::Preempted => {
            deps.counters.reads_preempted.fetch_add(1, Ordering::Relaxed);
            requeue_target(state, &outstanding);
        }
        ReadVerdict::Absent => {
            cool_absent_device(state, &deps.counters, outstanding.adapter_id, outstanding.short_address);
        }
        ReadVerdict::Failed => {
            fail_device(state, &deps.counters, outstanding.adapter_id, outstanding.short_address);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReadVerdict {
    Completed,
    Preempted,
    Absent,
    Failed,
}

fn read_verdict(body: &DaliAttributeReadOutcomesEvent) -> ReadVerdict {
    let sections = body.section_outcomes();
    if sections
        .iter()
        .any(|o| matches!(o, AttributeGroupReadOutcome::Preempted))
    {
        return ReadVerdict::Preempted;
    }
    if sections
        .iter()
        .any(|o| matches!(o, AttributeGroupReadOutcome::DeviceAbsent))
    {
        return ReadVerdict::Absent;
    }
    if sections.iter().all(|o| {
        matches!(
            o,
            AttributeGroupReadOutcome::NotRequested | AttributeGroupReadOutcome::Success
        )
    }) {
        return ReadVerdict::Completed;
    }
    ReadVerdict::Failed
}

fn requeue_target(state: &mut PollerState, outstanding: &Outstanding) {
    state.queue.push_back(PollTarget {
        adapter_id: outstanding.adapter_id,
        short_address: outstanding.short_address,
        groups_mask: outstanding.groups_mask,
        memory_banks: outstanding.memory_banks,
    });
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

    use dali2rust_bus::{BusConfig, BusHost};
    use dali2rust_domain::registry::{PollTargetReadPort, PollTargetSelection, PollerSettingsReadPort};

    struct NullPort;

    impl PollTargetReadPort for NullPort {
        fn list_poll_targets(&self, _adapter_id: u8) -> PollTargetSelection {
            PollTargetSelection { targets: Vec::new(), excluded_unbound: 0 }
        }
    }

    impl PollerSettingsReadPort for NullPort {
        fn poller_settings_view(&self) -> PollerSettingsView {
            test_state().settings
        }
    }

    struct TestDeps {
        deps: PollerDeps,
        _host: BusHost,
    }

    fn test_deps() -> TestDeps {
        let (_host, publisher, ()) = BusHost::spawn(BusConfig::default(), |_| ());
        let deps = PollerDeps {
            role: Arc::new(ActiveRole),
            publisher,
            read_port: Arc::new(NullPort),
            correlation: Arc::new(CorrelationIdAllocator::new()),
            bus_id: BusId::default(),
            adapter_count: 1,
            counters: Arc::new(PollerCounters::default()),
            interactive: Arc::new(dali2rust_platform::dali::WireActivity::new()),
            hold: Arc::new(dali2rust_platform::firmware::MaintenanceHold::new()),
        };
        TestDeps { deps, _host }
    }

    fn poll_target(short_address: u8) -> PollTarget {
        PollTarget {
            adapter_id: 0,
            short_address,
            groups_mask: 0,
            memory_banks: MemoryBankReadPreset::None,
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
    fn a_settings_event_cannot_shrink_the_period_below_the_floor() {
        let mut state = test_state();
        apply_settings_changed(
            &mut state,
            &PollerSettingsChangedEvent {
                enabled: true,
                interval_ms: 0,
                attribute_groups_mask: 0,
                include_dt8_color: false,
                include_energy: false,
                include_diagnostics: false,
                skip_unbound_virtual_lamps: false,
            },
        );
        assert!(
            state.next_cycle.duration_since(state.last_cycle) >= Duration::from_millis(200),
            "the REST floor (200 ms) holds even for a crafted event"
        );
    }
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
