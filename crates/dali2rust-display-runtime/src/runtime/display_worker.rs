use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dali2rust_bsp::esp_thread;
use dali2rust_bsp::std_thread_stack;
use dali2rust_bus::{recv_then_drain, BusFrame, BusSubscriberRx, WorkerTurn};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliBusHealthProbedEvent, DaliDiscoveryProgressEvent,
    DaliInputEventObservedEvent, DaliSceneRecalledEvent, DaliTargetScope,
    DaliTargetStateAppliedEvent, DaliTargetStateFailedEvent, DisplaySettingsChangedEvent,
    ErrorCode, FixedText32, HclScheduleChangedEvent, InputEventKind, DaliObservedFrameEvent,
    IpAddressAssignedEvent, LevelTransition, ObservedKind, OperationStatus,
    OperationStatusChangedEvent, OperationType, RuntimeSource, ARC_POWER_OFF,
};
use dali2rust_domain::dali::dev103::ButtonEvent;

pub use super::panel_driver::{HardwareDisplay, PAGE_DWELL};
use super::panel_driver::PanelDriver;
use crate::screen::{
    render, ActionLine, BusReading, EventLabel, InputLine, OperationLine, PollerLine,
    ScreenLines, ScreenState, TargetTag,
};
use crate::screen::{spark_heights_into, SPARK_COLS};
use crate::source::{DisplaySample, DisplaySource};

#[derive(Debug, Default)]
pub struct DisplayView {
    state: Mutex<ScreenState>,
}

impl DisplayView {
    pub fn snapshot(&self) -> ScreenState {
        self.state.lock().map_or_else(|e| *e.into_inner(), |g| *g)
    }

    pub fn lines(&self) -> ScreenLines {
        render(&self.snapshot())
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut ScreenState) -> R) -> R {
        match self.state.lock() {
            Ok(mut g) => f(&mut g),
            Err(e) => f(&mut e.into_inner()),
        }
    }
}

const POLL_MS: u64 = 1000;

const RATE_GAP_MS: u64 = 2 * POLL_MS;

const IDLE_WAIT: Duration = Duration::from_secs(60);

const ERROR_LABEL_COLS: usize = 12;

const RATE_WINDOW: usize = 60;

const SPARK_SAMPLE_S: u32 = 4;

const POLLER_FLAG_HOLD_MIN_MS: u64 = 30_000;
const POLLER_FLAG_HOLD_INTERVALS: u64 = 4;

const BUS_VERDICT_TTL_MIN_MS: u64 = 60_000;
const BUS_VERDICT_TTL_INTERVALS: u64 = 3;

struct RateWindow {
    samples: [u32; RATE_WINDOW],
    idx: usize,
    filled: usize,
}

impl RateWindow {
    fn new() -> Self {
        Self { samples: [0; RATE_WINDOW], idx: 0, filled: 0 }
    }

    fn clear(&mut self) {
        self.idx = 0;
        self.filled = 0;
    }

    fn push(&mut self, total: u32) -> u32 {
        let oldest = match self.filled {
            0 => total,
            RATE_WINDOW => self.samples[self.idx],
            _ => self.samples[0],
        };
        self.samples[self.idx] = total;
        self.idx = (self.idx + 1) % RATE_WINDOW;
        self.filled = (self.filled + 1).min(RATE_WINDOW);
        total.saturating_sub(oldest)
    }
}

#[derive(Default)]
struct Facts {
    ip: Option<[u8; 4]>,
    bus: BusReading,
    bus_at: Option<Instant>,
    hcl_kelvin: Option<u16>,
    last_input: Option<InputLine>,
    last_action: Option<ActionLine>,
    action_at: Option<Instant>,
    operation: Option<OperationLine>,
    operation_key: Option<FixedText32>,
    scan_found: u16,
    activity_seq: u32,
    settings_changed: bool,
}

impl Facts {
    fn record_action(&mut self, line: ActionLine) {
        if line.failed || line.source.is_none_or(wakes_the_screen) {
            self.activity_seq = self.activity_seq.wrapping_add(1);
        }
        self.last_action = Some(line);
        self.action_at = Some(Instant::now());
    }

    fn note_activity(&mut self) {
        self.activity_seq = self.activity_seq.wrapping_add(1);
    }
}

fn wakes_the_screen(source: RuntimeSource) -> bool {
    !matches!(source, RuntimeSource::Hcl | RuntimeSource::Poller | RuntimeSource::Readback)
}

dali2rust_contracts::dispatch_bus_events! {
    pub const DISPLAY_WORKER_HANDLED_EVENTS;
    fn dispatch_display_event(
        payload: &BusEventPayload,
        facts: &mut Facts,
        adapter: &u8,
        _corr: u64,
    );
    payload = payload;
    ignored = {};
    IpAddressAssignedEvent(body) => apply_ip(facts, body),
    DaliBusHealthProbedEvent(body) => apply_health(facts, *adapter, body),
    DaliInputEventObservedEvent(body) => apply_input(facts, *adapter, body),
    OperationStatusChangedEvent(body) => apply_operation(facts, body),
    DaliDiscoveryProgressEvent(body) => apply_discovery(facts, *adapter, body),
    DaliTargetStateAppliedEvent(body) => apply_applied(facts, *adapter, body),
    DaliTargetStateFailedEvent(body) => apply_failed(facts, *adapter, body),
    DaliSceneRecalledEvent(body) => apply_scene(facts, *adapter, body),
    DaliObservedFrameEvent(body) => apply_observed(facts, *adapter, body),
    HclScheduleChangedEvent(body) => apply_hcl_changed(facts, body),
    DisplaySettingsChangedEvent(body) => apply_display_settings(facts, body),
}

fn apply_display_settings(facts: &mut Facts, _body: &DisplaySettingsChangedEvent) {
    facts.settings_changed = true;
}

fn apply_ip(facts: &mut Facts, body: &IpAddressAssignedEvent) {
    facts.ip = if body.ip_v4 == [0, 0, 0, 0] { None } else { Some(body.ip_v4) };
}

fn speaks_for(adapter: u8, event_adapter: u8) -> bool {
    adapter == event_adapter
}

fn apply_health(facts: &mut Facts, adapter: u8, body: &DaliBusHealthProbedEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    facts.bus = BusReading::from_probe(body.control_answered, body.lamp_failure);
    facts.bus_at = Some(Instant::now());
}

fn apply_hcl_changed(facts: &mut Facts, _body: &HclScheduleChangedEvent) {
    facts.hcl_kelvin = None;
}

fn apply_input(facts: &mut Facts, adapter: u8, body: &DaliInputEventObservedEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    facts.last_input = Some(InputLine {
        short_address: body.short_address,
        instance: body.instance_number,
        what: input_label(body),
    });
    if matches!(body.typed, InputEventKind::Button | InputEventKind::Position) {
        facts.note_activity();
    }
}

fn input_label(body: &DaliInputEventObservedEvent) -> EventLabel {
    match body.typed {
        InputEventKind::Button => button_label(body.typed_value),
        InputEventKind::Occupancy => {
            EventLabel::from_fmt(format_args!("OCC {:02X}", body.typed_value & 0xFF))
        }
        InputEventKind::Position => EventLabel::from_fmt(format_args!("POS {}", body.typed_value)),
        InputEventKind::Illuminance => {
            EventLabel::from_fmt(format_args!("LUX {}", body.typed_value))
        }
        InputEventKind::Generic => EventLabel::from_fmt(format_args!("EV {:03X}", body.event_info)),
    }
}

fn button_label(code: u16) -> EventLabel {
    let Some(event) = ButtonEvent::from_info(code) else {
        return EventLabel::from_fmt(format_args!("BTN {code:02X}"));
    };
    let name = match event {
        ButtonEvent::Release => "RELEASE",
        ButtonEvent::Press => "PRESS",
        ButtonEvent::ShortPress => "SHORT",
        ButtonEvent::DoublePress => "DOUBLE",
        ButtonEvent::LongPressStart => "LONG",
        ButtonEvent::LongPressRepeat => "REPEAT",
        ButtonEvent::LongPressStop => "LONG END",
        ButtonEvent::ButtonFree => "FREE",
        ButtonEvent::ButtonStuck => "STUCK",
    };
    EventLabel::from_fmt(format_args!("{name}"))
}

fn apply_operation(facts: &mut Facts, body: &OperationStatusChangedEvent) {
    match body.status {
        OperationStatus::Accepted | OperationStatus::Running => start_operation(facts, body),
        _ => end_operation(facts, body),
    }
}

fn start_operation(facts: &mut Facts, body: &OperationStatusChangedEvent) {
    if facts.operation.is_some() {
        return;
    }
    facts.operation = Some(OperationLine {
        name: EventLabel::from_fmt(format_args!("{}", operation_tag(body.operation_type))),
        done: 0,
        total: 0,
    });
    facts.operation_key = Some(body.operation_key.clone());
    facts.scan_found = 0;
    facts.note_activity();
}

fn end_operation(facts: &mut Facts, body: &OperationStatusChangedEvent) {
    if facts.operation_key.as_ref().is_none_or(|k| *k == body.operation_key) {
        facts.operation = None;
        facts.operation_key = None;
    }
    let code = body.error.as_ref().map(|e| e.code);
    if body.status == OperationStatus::Succeeded || is_benign_end(body.status, code) {
        return;
    }
    facts.record_action(ActionLine {
        target: TargetTag::from_fmt(format_args!("{}", operation_tag(body.operation_type))),
        what: error_label(code),
        source: None,
        failed: true,
        age_ms: 0,
    });
}

fn is_benign_end(status: OperationStatus, code: Option<ErrorCode>) -> bool {
    status == OperationStatus::Cancelled
        || matches!(code, Some(ErrorCode::Preempted | ErrorCode::Superseded))
}

fn operation_tag(kind: OperationType) -> &'static str {
    match kind {
        OperationType::Discovery => "SCAN",
        OperationType::AttributeRead => "READ",
        OperationType::MemoryBankRead => "BANK",
        OperationType::GroupApply => "GROUP",
        OperationType::SceneApply => "SCENE",
        OperationType::HaDiscoveryPublish => "HA",
        OperationType::CommissioningIdentify => "IDENT",
        OperationType::CommissioningAddressChange => "ADDR",
        OperationType::CommissioningReplaceDevice => "REPL",
        OperationType::AttributeWrite => "WRITE",
        OperationType::ConfigWrite => "CFG",
        OperationType::PolicyApply => "POLCY",
        OperationType::FirmwareUpdate => "FW",
    }
}

fn error_label(code: Option<dali2rust_contracts::msg::ErrorCode>) -> EventLabel {
    let Some(code) = code else {
        return EventLabel::from_fmt(format_args!("FAILED"));
    };
    let mut out = EventLabel::new();
    for ch in code.rest_name().chars().take(ERROR_LABEL_COLS) {
        let mut one = [0u8; 4];
        let _ = core::fmt::Write::write_str(&mut out, ch.to_ascii_uppercase().encode_utf8(&mut one));
    }
    out
}

fn apply_discovery(facts: &mut Facts, adapter: u8, body: &DaliDiscoveryProgressEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    facts.scan_found = facts.scan_found.saturating_add(1);
    facts.operation = Some(OperationLine {
        name: EventLabel::from_fmt(format_args!("SCAN {} FOUND", facts.scan_found)),
        done: 0,
        total: 0,
    });
    facts.note_activity();
}

fn apply_applied(facts: &mut Facts, adapter: u8, body: &DaliTargetStateAppliedEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    if body.source == RuntimeSource::Hcl {
        if let Some(color) = body.setpoint.color.as_ref() {
            if color.color_temperature_kelvin > 0 {
                facts.hcl_kelvin = Some(color.color_temperature_kelvin);
            }
        }
    }
    facts.record_action(ActionLine {
        target: target_label(body.scope, body.short_address, body.group_id, body.virtual_lamp_id),
        what: setpoint_label(body),
        source: Some(body.source),
        failed: false,
        age_ms: 0,
    });
}

fn apply_failed(facts: &mut Facts, adapter: u8, body: &DaliTargetStateFailedEvent) {
    if !speaks_for(adapter, body.adapter_id) {
        return;
    }
    facts.record_action(ActionLine {
        target: target_label(body.scope, Some(body.short_address), body.group_id, body.virtual_lamp_id),
        what: error_label(Some(body.error.code)),
        source: None,
        failed: true,
        age_ms: 0,
    });
}

fn transition_label(verb: LevelTransition) -> &'static str {
    match verb {
        LevelTransition::RecallMaxLevel => "MAX",
        LevelTransition::RecallMinLevel => "MIN",
        LevelTransition::StepUp => "ST+",
        LevelTransition::StepDown => "ST-",
        LevelTransition::StepDownAndOff => "ST0",
        LevelTransition::OnAndStepUp => "ON+",
        LevelTransition::GoToLastActiveLevel => "LAST",
        LevelTransition::Up => "UP",
        LevelTransition::Down => "DN",
    }
}

fn apply_observed(facts: &mut Facts, adapter: u8, body: &DaliObservedFrameEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    let what = match body.observed_kind {
        ObservedKind::UnknownObserved | ObservedKind::SceneRecallObserved => return,
        ObservedKind::LevelTransitionObserved => match body.level_transition {
            Some(verb) => EventLabel::from_fmt(format_args!("{}", transition_label(verb))),
            None => return,
        },
        _ => match body.setpoint.as_ref() {
            Some(sp) => observed_label(sp),
            None => return,
        },
    };
    facts.record_action(ActionLine {
        target: target_label(body.scope, body.short_address, body.group_id, None),
        what,
        source: Some(RuntimeSource::Sniffer),
        failed: false,
        age_ms: 0,
    });
}

fn observed_label(setpoint: &dali2rust_contracts::msg::LightSetpoint) -> EventLabel {
    if setpoint.dapc_level() == Some(ARC_POWER_OFF) {
        return EventLabel::from_fmt(format_args!("OFF"));
    }
    match (setpoint.color.as_ref(), setpoint.level) {
        (Some(c), Some(level)) if c.color_temperature_kelvin > 0 => {
            EventLabel::from_fmt(format_args!("{} {}K", level, c.color_temperature_kelvin))
        }
        (Some(c), None) if c.color_temperature_kelvin > 0 => {
            EventLabel::from_fmt(format_args!("{}K", c.color_temperature_kelvin))
        }
        (_, level) => on_label(level),
    }
}

fn on_label(level: Option<u8>) -> EventLabel {
    match level {
        Some(level) => EventLabel::from_fmt(format_args!("ON {level}")),
        None => EventLabel::from_fmt(format_args!("ON")),
    }
}

fn apply_scene(facts: &mut Facts, adapter: u8, body: &DaliSceneRecalledEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    let scope = match body.virtual_lamp_id {
        Some(_) => DaliTargetScope::VirtualLamp,
        None => body.scope,
    };
    facts.record_action(ActionLine {
        target: target_label(
            scope,
            Some(body.short_address),
            Some(body.group_id),
            body.virtual_lamp_id,
        ),
        what: EventLabel::from_fmt(format_args!("SC{}", body.scene_id)),
        source: Some(body.source),
        failed: body.error.is_some(),
        age_ms: 0,
    });
}

fn target_label(
    scope: DaliTargetScope,
    short_address: Option<u8>,
    group_id: Option<u8>,
    virtual_lamp_id: Option<u8>,
) -> TargetTag {
    match scope {
        DaliTargetScope::Short => {
            TargetTag::from_fmt(format_args!("A{:02}", short_address.unwrap_or(0)))
        }
        DaliTargetScope::Group => TargetTag::from_fmt(format_args!("G{:02}", group_id.unwrap_or(0))),
        DaliTargetScope::VirtualLamp => {
            TargetTag::from_fmt(format_args!("L{:02}", virtual_lamp_id.unwrap_or(0)))
        }
        DaliTargetScope::Broadcast => TargetTag::from_fmt(format_args!("BC")),
        DaliTargetScope::AddressRange => TargetTag::from_fmt(format_args!("RNG")),
    }
}

fn setpoint_label(body: &DaliTargetStateAppliedEvent) -> EventLabel {
    if body.setpoint.dapc_level() == Some(ARC_POWER_OFF) {
        return EventLabel::from_fmt(format_args!("OFF"));
    }
    if body.dapc_applied {
        return on_label(body.setpoint.level);
    }
    match body.setpoint.color.as_ref().filter(|_| body.setpoint.states_color()) {
        Some(c) if c.color_temperature_kelvin > 0 => {
            EventLabel::from_fmt(format_args!("{}K", c.color_temperature_kelvin))
        }
        Some(_) => EventLabel::from_fmt(format_args!("COLOUR")),
        None => EventLabel::from_fmt(format_args!("ON")),
    }
}

struct Tick {
    boot: Instant,
    frames: Box<RateWindow>,
    foreign: Box<RateWindow>,
    last_reads_failed: Option<u32>,
    last_reads_absent: Option<u32>,
    failing_until: Option<Instant>,
    absent_until: Option<Instant>,
    sampled_at: Option<Instant>,
    frames_per_min: u32,
    foreign_per_min: u32,
    spark_elapsed_s: u32,
    poller: PollerLine,
}

impl Tick {
    fn new() -> Self {
        Self {
            boot: Instant::now(),
            frames: Box::new(RateWindow::new()),
            foreign: Box::new(RateWindow::new()),
            last_reads_failed: None,
            last_reads_absent: None,
            failing_until: None,
            absent_until: None,
            sampled_at: None,
            frames_per_min: 0,
            foreign_per_min: 0,
            spark_elapsed_s: SPARK_SAMPLE_S,
            poller: PollerLine::default(),
        }
    }

    fn fill(&mut self, dst: &mut ScreenState, facts: &Facts, sample: DisplaySample, now: Instant) {
        dst.ip = facts.ip;
        dst.uptime_s = now.duration_since(self.boot).as_secs().min(u64::from(u32::MAX)) as u32;
        self.sample_rates(dst, sample, now);
        dst.frames_per_min = self.frames_per_min;
        dst.foreign_per_min = self.foreign_per_min;
        dst.poller = self.poller;
        dst.sample = sample;
        dst.bus = bus_reading(facts, sample, now);
        dst.hcl_kelvin = facts.hcl_kelvin;
        dst.last_input = facts.last_input;
        dst.last_action = aged(facts.last_action, facts.action_at, now, |l, ms| {
            l.age_ms = ms;
        });
        dst.operation = facts.operation;
    }

    fn sample_rates(&mut self, dst: &mut ScreenState, sample: DisplaySample, now: Instant) {
        let since = self.sampled_at.map(|t| now.duration_since(t));
        if since.is_some_and(|gap| gap < Duration::from_millis(POLL_MS)) {
            return;
        }
        if since.is_some_and(|gap| gap > Duration::from_millis(RATE_GAP_MS)) {
            self.frames.clear();
            self.foreign.clear();
        }
        self.sampled_at = Some(now);
        self.frames_per_min = self.frames.push(sample.frames_sent_total);
        self.foreign_per_min = self.foreign.push(sample.foreign_frames_total);
        self.poller = self.poller_line(sample, now, since);
        self.spark_elapsed_s += 1;
        if self.spark_elapsed_s >= SPARK_SAMPLE_S {
            self.spark_elapsed_s = 0;
            dst.load_history.rotate_left(1);
            dst.load_history[SPARK_COLS - 1] = sample.wire_load_permille;
            spark_heights_into(&dst.load_history, &mut dst.spark);
        }
    }
}

impl Tick {
    fn poller_line(
        &mut self,
        sample: DisplaySample,
        now: Instant,
        since: Option<Duration>,
    ) -> PollerLine {
        let hold = poller_flag_hold(sample);
        if since.is_some_and(|gap| gap > hold) {
            self.last_reads_failed = None;
            self.last_reads_absent = None;
        }
        self.note_poller_counters(sample, now + hold);
        PollerLine {
            enabled: sample.poller_enabled,
            interval_ms: sample.poller_interval_ms,
            failing: self.failing_until.is_some_and(|t| now < t),
            absent: self.absent_until.is_some_and(|t| now < t),
        }
    }

    fn note_poller_counters(&mut self, sample: DisplaySample, hold_until: Instant) {
        if self.last_reads_failed.is_some_and(|prev| sample.poller_reads_failed > prev) {
            self.failing_until = Some(hold_until);
        }
        if self.last_reads_absent.is_some_and(|prev| sample.poller_reads_absent > prev) {
            self.absent_until = Some(hold_until);
        }
        self.last_reads_failed = Some(sample.poller_reads_failed);
        self.last_reads_absent = Some(sample.poller_reads_absent);
    }
}

fn poller_flag_hold(sample: DisplaySample) -> Duration {
    Duration::from_millis(
        u64::from(sample.poller_interval_ms)
            .saturating_mul(POLLER_FLAG_HOLD_INTERVALS)
            .max(POLLER_FLAG_HOLD_MIN_MS),
    )
}

fn bus_reading(facts: &Facts, sample: DisplaySample, now: Instant) -> BusReading {
    let ttl = Duration::from_millis(
        u64::from(sample.poller_interval_ms)
            .saturating_mul(BUS_VERDICT_TTL_INTERVALS)
            .max(BUS_VERDICT_TTL_MIN_MS),
    );
    match facts.bus_at {
        Some(at) if now.duration_since(at) <= ttl => facts.bus,
        _ => BusReading::Unprobed,
    }
}

fn aged<T>(
    line: Option<T>,
    at: Option<Instant>,
    now: Instant,
    set: impl Fn(&mut T, u32),
) -> Option<T> {
    let mut line = line?;
    let ms = at.map_or(0, |t| now.duration_since(t).as_millis().min(u128::from(u32::MAX)) as u32);
    set(&mut line, ms);
    Some(line)
}

pub fn spawn_display_worker(
    ev: BusSubscriberRx,
    source: Arc<dyn DisplaySource>,
    view: Arc<DisplayView>,
    hardware: HardwareDisplay,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"display_worker",
        std_thread_stack::DISPLAY_WORKER,
        esp_thread::StackHome::ExternalOnXip,
        move || Worker::new(source, view, hardware).run(&ev),
    )
}

struct Worker {
    facts: Box<Facts>,
    tick: Box<Tick>,
    panel: Box<PanelDriver>,
    source: Arc<dyn DisplaySource>,
    view: Arc<DisplayView>,
    hardware: HardwareDisplay,
}

impl Worker {
    fn new(source: Arc<dyn DisplaySource>, view: Arc<DisplayView>, hardware: HardwareDisplay) -> Self {
        Self {
            facts: Box::new(Facts::default()),
            tick: Box::new(Tick::new()),
            panel: Box::new(PanelDriver::new(Instant::now())),
            source,
            view,
            hardware,
        }
    }

    fn run(mut self, ev: &BusSubscriberRx) {
        let adapter_id = self.source.sample().adapter_id;
        let mut next_render = Instant::now() + render_gap(self.refresh());
        loop {
            let wait = next_render.saturating_duration_since(Instant::now());
            let facts = &mut self.facts;
            if recv_then_drain(ev, wait, |frame| consume(facts, adapter_id, &frame))
                == WorkerTurn::Disconnected
            {
                return;
            }
            if Instant::now() < next_render && !self.facts.settings_changed {
                continue;
            }
            next_render = Instant::now() + render_gap(self.refresh());
        }
    }

    fn refresh(&mut self) -> bool {
        self.facts.settings_changed = false;
        let sample = self.source.sample();
        let now = Instant::now();
        let (tick, facts, panel, hardware) =
            (&mut self.tick, &self.facts, &mut self.panel, &self.hardware);
        self.view.with_state(|state| {
            tick.fill(state, facts, sample, now);
            state.page = panel.turn_page(sample.display.pages_mask, now);
            panel.drive(hardware, state, facts.activity_seq, now);
        });
        sample.display.enabled
    }
}

fn render_gap(live: bool) -> Duration {
    if live {
        Duration::from_millis(POLL_MS)
    } else {
        IDLE_WAIT
    }
}

fn consume(facts: &mut Facts, adapter: u8, frame: &BusFrame) {
    if let BusFrame::Event(arc) = frame {
        dispatch_display_event(&arc.payload, facts, &adapter, arc.meta.correlation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(source: Option<RuntimeSource>, failed: bool) -> ActionLine {
        ActionLine {
            target: TargetTag::from_fmt(format_args!("A01")),
            what: EventLabel::from_fmt(format_args!("ON")),
            source,
            failed,
            age_ms: 0,
        }
    }

    fn input(typed: InputEventKind) -> DaliInputEventObservedEvent {
        DaliInputEventObservedEvent {
            registry_adapter_id: 0,
            scheme: 2,
            short_address: Some(5),
            device_group: None,
            instance_group: None,
            instance_number: Some(2),
            instance_type: Some(1),
            event_info: 0,
            typed,
            typed_value: 0,
            observed_at_ms: 0,
            observed_at_mono_ms: 0,
        }
    }

    #[test]
    fn background_actions_wake_the_screen_only_when_they_fail() {
        let mut facts = Facts::default();
        facts.record_action(action(Some(RuntimeSource::Hcl), false));
        facts.record_action(action(Some(RuntimeSource::Poller), false));
        assert_eq!(facts.activity_seq, 0);
        facts.record_action(action(Some(RuntimeSource::Hcl), true));
        assert_eq!(facts.activity_seq, 1);
        facts.record_action(action(Some(RuntimeSource::Api), false));
        facts.record_action(action(None, false));
        assert_eq!(facts.activity_seq, 3);
    }

    #[test]
    fn only_a_hand_on_a_control_wakes_the_screen() {
        let mut facts = Facts::default();
        for typed in [InputEventKind::Occupancy, InputEventKind::Illuminance, InputEventKind::Generic] {
            apply_input(&mut facts, 0, &input(typed));
        }
        assert_eq!(facts.activity_seq, 0);
        assert!(facts.last_input.is_some(), "the row still shows the sensor");
        apply_input(&mut facts, 0, &input(InputEventKind::Button));
        apply_input(&mut facts, 0, &input(InputEventKind::Position));
        assert_eq!(facts.activity_seq, 2);
    }

    #[test]
    fn a_cleared_window_counts_from_its_first_sample() {
        let mut window = RateWindow::new();
        window.push(0);
        window.push(500);
        window.clear();
        assert_eq!(window.push(9000), 0, "a stale slot is no baseline");
        assert_eq!(window.push(9060), 60);
    }

    const DOCUMENTED_REREAD: Duration = Duration::from_secs(60);

    fn failing_after(gap_ms: u64) -> bool {
        let mut tick = Tick::new();
        let mut state = ScreenState::default();
        let start = Instant::now();
        let sample = DisplaySample { poller_reads_failed: 1, ..DisplaySample::default() };
        tick.fill(&mut state, &Facts::default(), sample, start);
        let grown = DisplaySample { poller_reads_failed: 5, ..sample };
        tick.fill(&mut state, &Facts::default(), grown, start + Duration::from_millis(gap_ms));
        state.poller.failing
    }

    #[test]
    fn a_pause_longer_than_the_flag_hold_forgets_the_poller_counters() {
        assert!(!failing_after(POLLER_FLAG_HOLD_MIN_MS + POLL_MS), "failures of unknown age");
    }

    #[test]
    fn a_short_stall_still_flags_the_failures_it_spans() {
        assert!(failing_after(RATE_GAP_MS + POLL_MS));
    }

    #[test]
    fn a_disabled_display_rereads_its_settings_within_a_minute() {
        assert!(render_gap(false) <= DOCUMENTED_REREAD);
        assert_eq!(render_gap(true), Duration::from_millis(POLL_MS));
    }
}
