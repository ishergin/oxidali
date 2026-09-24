use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dali2rust_bsp::esp_thread;
use dali2rust_bsp::std_thread_stack;
use dali2rust_bus::{recv_then_drain, BusFrame, BusSubscriberRx, WorkerTurn};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliBusHealthProbedEvent, DaliDiscoveryProgressEvent,
    DaliInputEventObservedEvent, DaliSceneRecalledEvent, DaliTargetScope,
    DaliTargetStateAppliedEvent, DaliTargetStateFailedEvent, ErrorCode, FixedText32,
    HclScheduleChangedEvent, InputEventKind, DaliObservedFrameEvent, IpAddressAssignedEvent,
    ObservedKind, OperationStatus, OperationStatusChangedEvent, OperationType, PowerState,
    RuntimeSource,
};
use dali2rust_domain::dali::dev103::ButtonEvent;

use crate::screen::{
    render, render_row, ActionLine, BusReading, EventLabel, InputLine, OperationLine, PollerLine,
    ScreenLines, ScreenState, TargetTag, SCREEN_ROWS,
};
use crate::screen::{spark_heights_into, SPARK_COLS};
use crate::source::{DisplaySample, DisplaySource};

pub enum HardwareDisplay {
    None,
    #[cfg(target_os = "espidf")]
    EspOled(Arc<Mutex<crate::display::esp_idf::Ssd1306Display<'static>>>),
}

impl HardwareDisplay {
    pub fn none() -> Self {
        Self::None
    }
}

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

    fn push(&mut self, total: u32) -> u32 {
        let oldest = if self.filled == RATE_WINDOW {
            self.samples[self.idx]
        } else {
            self.samples[0]
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
        OperationStatus::Accepted | OperationStatus::Running => {
            if facts.operation.is_none() {
                facts.operation = Some(OperationLine {
                    name: EventLabel::from_fmt(format_args!(
                        "{}",
                        operation_tag(body.operation_type)
                    )),
                    done: 0,
                    total: 0,
                });
                facts.operation_key = Some(body.operation_key.clone());
                facts.scan_found = 0;
            }
        }
        _ => {
            if facts.operation_key.as_ref().is_none_or(|k| *k == body.operation_key) {
                facts.operation = None;
                facts.operation_key = None;
            }
            let code = body.error.as_ref().map(|e| e.code);
            if body.status != OperationStatus::Succeeded && !is_benign_end(body.status, code) {
                facts.last_action = Some(ActionLine {
                    target: TargetTag::from_fmt(format_args!(
                        "{}",
                        operation_tag(body.operation_type)
                    )),
                    what: error_label(code),
                    source: None,
                    failed: true,
                    age_ms: 0,
                });
                facts.action_at = Some(Instant::now());
            }
        }
    }
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
    facts.last_action = Some(ActionLine {
        target: target_label(body.scope, body.short_address, body.group_id, body.virtual_lamp_id),
        what: setpoint_label(body),
        source: Some(body.source),
        failed: false,
        age_ms: 0,
    });
    facts.action_at = Some(Instant::now());
}

fn apply_failed(facts: &mut Facts, adapter: u8, body: &DaliTargetStateFailedEvent) {
    if !speaks_for(adapter, body.adapter_id) {
        return;
    }
    facts.last_action = Some(ActionLine {
        target: target_label(body.scope, Some(body.short_address), body.group_id, body.virtual_lamp_id),
        what: error_label(Some(body.error.code)),
        source: None,
        failed: true,
        age_ms: 0,
    });
    facts.action_at = Some(Instant::now());
}

fn transition_label(verb: dali2rust_contracts::msg::LevelTransition) -> &'static str {
    use dali2rust_contracts::msg::LevelTransition as T;
    match verb {
        T::RecallMaxLevel => "MAX",
        T::RecallMinLevel => "MIN",
        T::StepUp => "ST+",
        T::StepDown => "ST-",
        T::StepDownAndOff => "ST0",
        T::OnAndStepUp => "ON+",
        T::GoToLastActiveLevel => "LAST",
    }
}

fn apply_observed(facts: &mut Facts, adapter: u8, body: &DaliObservedFrameEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    let what = match (body.observed_kind, body.scene_id) {
        (ObservedKind::SceneRecallObserved, Some(scene)) => {
            EventLabel::from_fmt(format_args!("SC{scene}"))
        }
        (ObservedKind::UnknownObserved, _) => return,
        (ObservedKind::LevelTransitionObserved, _) => match body.level_transition {
            Some(verb) => EventLabel::from_fmt(format_args!("{}", transition_label(verb))),
            None => return,
        },
        _ => match body.setpoint.as_ref() {
            Some(sp) => observed_label(sp),
            None => return,
        },
    };
    facts.last_action = Some(ActionLine {
        target: target_label(body.scope, body.short_address, body.group_id, None),
        what,
        source: Some(RuntimeSource::Sniffer),
        failed: false,
        age_ms: 0,
    });
    facts.action_at = Some(Instant::now());
}

fn observed_label(setpoint: &dali2rust_contracts::msg::LightSetpoint) -> EventLabel {
    if setpoint.power == PowerState::Off {
        return EventLabel::from_fmt(format_args!("OFF"));
    }
    match setpoint.color.as_ref() {
        Some(c) if c.color_temperature_kelvin > 0 => EventLabel::from_fmt(format_args!(
            "{} {}K",
            setpoint.level, c.color_temperature_kelvin
        )),
        _ => EventLabel::from_fmt(format_args!("ON {}", setpoint.level)),
    }
}

fn apply_scene(facts: &mut Facts, adapter: u8, body: &DaliSceneRecalledEvent) {
    if !speaks_for(adapter, body.registry_adapter_id) {
        return;
    }
    facts.last_action = Some(ActionLine {
        target: target_label(
            body.scope,
            Some(body.short_address),
            Some(body.group_id),
            None,
        ),
        what: EventLabel::from_fmt(format_args!("SC{}", body.scene_id)),
        source: None,
        failed: body.error.is_some(),
        age_ms: 0,
    });
    facts.action_at = Some(Instant::now());
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
    if body.setpoint.power == PowerState::Off {
        return EventLabel::from_fmt(format_args!("OFF"));
    }
    if body.dapc_applied {
        return EventLabel::from_fmt(format_args!("ON {}", body.setpoint.level));
    }
    match body.setpoint.color.as_ref() {
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
    drawn: [u64; SCREEN_ROWS],
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
            drawn: [0; SCREEN_ROWS],
        }
    }

    fn fill(&mut self, dst: &mut ScreenState, facts: &Facts, sample: DisplaySample) {
        let now = Instant::now();
        dst.ip = facts.ip;
        dst.uptime_s = now.duration_since(self.boot).as_secs().min(u64::from(u32::MAX)) as u32;
        if self
            .sampled_at
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(POLL_MS))
        {
            self.sampled_at = Some(now);
            self.frames_per_min = self.frames.push(sample.frames_sent_total);
            self.foreign_per_min = self.foreign.push(sample.foreign_frames_total);
            self.poller = self.poller_line(sample, now);
            self.spark_elapsed_s += 1;
            if self.spark_elapsed_s >= SPARK_SAMPLE_S {
                self.spark_elapsed_s = 0;
                dst.load_history.rotate_left(1);
                dst.load_history[SPARK_COLS - 1] = sample.wire_load_permille;
                spark_heights_into(&dst.load_history, &mut dst.spark);
            }
        }
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
}

impl Tick {
    fn poller_line(&mut self, sample: DisplaySample, now: Instant) -> PollerLine {
        let hold = Duration::from_millis(
            u64::from(sample.poller_interval_ms)
                .saturating_mul(POLLER_FLAG_HOLD_INTERVALS)
                .max(POLLER_FLAG_HOLD_MIN_MS),
        );
        if self.last_reads_failed.is_some_and(|prev| sample.poller_reads_failed > prev) {
            self.failing_until = Some(now + hold);
        }
        if self.last_reads_absent.is_some_and(|prev| sample.poller_reads_absent > prev) {
            self.absent_until = Some(now + hold);
        }
        self.last_reads_failed = Some(sample.poller_reads_failed);
        self.last_reads_absent = Some(sample.poller_reads_absent);
        PollerLine {
            enabled: sample.poller_enabled,
            interval_ms: sample.poller_interval_ms,
            failing: self.failing_until.is_some_and(|t| now < t),
            absent: self.absent_until.is_some_and(|t| now < t),
        }
    }
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
        move || {
            let mut facts = Box::new(Facts::default());
            let mut tick = Box::new(Tick::new());
            let adapter_id = source.sample().adapter_id;
            refresh(&mut tick, &facts, &source, &view, &hardware);
            let mut next_render = Instant::now() + Duration::from_millis(POLL_MS);
            loop {
                let wait = next_render.saturating_duration_since(Instant::now());
                let turn = recv_then_drain(&ev, wait, |frame| {
                    consume(&mut facts, adapter_id, &frame);
                });
                if turn == WorkerTurn::Disconnected {
                    return;
                }
                if Instant::now() < next_render {
                    continue;
                }
                refresh(&mut tick, &facts, &source, &view, &hardware);
                next_render = Instant::now() + Duration::from_millis(POLL_MS);
            }
        },
    )
}

fn refresh(
    tick: &mut Tick,
    facts: &Facts,
    source: &Arc<dyn DisplaySource>,
    view: &Arc<DisplayView>,
    hardware: &HardwareDisplay,
) {
    let sample = source.sample();
    view.with_state(|state| {
        tick.fill(state, facts, sample);
        for row in 0..SCREEN_ROWS {
            let line = render_row(state, row);
            let digest = digest_row(&line, &state.spark);
            if tick.drawn[row] == digest {
                continue;
            }
            if draw_row(hardware, row, &line, &state.spark) {
                tick.drawn[row] = digest;
            }
        }
    });
}

fn digest_row(line: &crate::screen::ScreenRow, spark: &[u8; SPARK_COLS]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in line.text.as_bytes() {
        h = (h ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3);
    }
    h = (h ^ u64::from(line.invert)).wrapping_mul(0x1000_0000_01b3);
    h = (h ^ u64::from(line.bar.unwrap_or(u16::MAX))).wrapping_mul(0x1000_0000_01b3);
    if line.spark {
        for column in spark {
            h = (h ^ u64::from(*column)).wrapping_mul(0x1000_0000_01b3);
        }
    }
    h
}

fn consume(facts: &mut Facts, adapter: u8, frame: &BusFrame) {
    if let BusFrame::Event(arc) = frame {
        dispatch_display_event(&arc.payload, facts, &adapter, arc.meta.correlation_id);
    }
}

#[cfg(target_os = "espidf")]
fn draw_row(
    hardware: &HardwareDisplay,
    row: usize,
    line: &crate::screen::ScreenRow,
    spark: &[u8; SPARK_COLS],
) -> bool {
    let HardwareDisplay::EspOled(display) = hardware else {
        return true;
    };
    match display.lock() {
        Ok(mut d) => render::draw_row(&mut d, row, line, spark),
        Err(_) => false,
    }
}

#[cfg(not(target_os = "espidf"))]
fn draw_row(
    _hardware: &HardwareDisplay,
    _row: usize,
    _line: &crate::screen::ScreenRow,
    _spark: &[u8; SPARK_COLS],
) -> bool {
    true
}

#[cfg(target_os = "espidf")]
mod render {
    use dali2rust_platform::display::{DisplayDriver, FontSize};

    use crate::display::esp_idf::Ssd1306Display;
    use crate::screen::{ScreenRow, SPARK_COLS, SPARK_H};

    const ROW_PITCH: u32 = 9;
    const ROW_H: u32 = 9;
    const DISP_W: u32 = 128;
    const BAR_W: u32 = 54;
    const SPARK_X: u32 = DISP_W - SPARK_COLS as u32;

    pub fn draw_row(
        display: &mut Ssd1306Display<'static>,
        row: usize,
        line: &ScreenRow,
        spark: &[u8; SPARK_COLS],
    ) -> bool {
        let y = row as u32 * ROW_PITCH;
        let _ = display.clear_region(0, y, DISP_W, ROW_H);
        let _ = display.draw_text(0, y, &line.text, FontSize::Small);
        if let Some(permille) = line.bar {
            draw_bar(display, y, permille);
        }
        if line.spark {
            draw_spark(display, y, spark);
        }
        if line.invert {
            let _ = display.invert_region(0, y, DISP_W, ROW_H);
        }
        display.flush_region(y, ROW_H).is_ok()
    }

    fn draw_bar(display: &mut Ssd1306Display<'static>, y: u32, permille: u16) {
        let x = DISP_W - BAR_W;
        let _ = display.fill_region(x, y, BAR_W, 1);
        let _ = display.fill_region(x, y + 7, BAR_W, 1);
        let _ = display.fill_region(x, y, 1, 8);
        let _ = display.fill_region(DISP_W - 1, y, 1, 8);
        let filled = u32::from(permille) * (BAR_W - 4) / 1000;
        let _ = display.fill_region(x + 2, y + 2, filled, 4);
    }

    #[inline(always)]
    fn draw_spark(display: &mut Ssd1306Display<'static>, y: u32, spark: &[u8; SPARK_COLS]) {
        for (i, h) in spark.iter().enumerate() {
            let h = u32::from(*h);
            if h == 0 {
                continue;
            }
            let top = y + 1 + u32::from(SPARK_H) - h;
            let _ = display.fill_region(SPARK_X + i as u32, top, 1, h);
        }
    }
}
