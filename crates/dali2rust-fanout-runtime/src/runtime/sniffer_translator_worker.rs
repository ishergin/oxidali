use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use dali2rust_bsp::{esp_thread, std_thread_stack};
use dali2rust_bus::{
    publish_required, BusChannel, BusFrame, BusId, BusPublisher, PublishResult,
    REQUIRED_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_UNCAPPED,
};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    Dali103ApplicationControlObservedEvent,
    ColorMode, ColorValue, DaliInputDeviceLifecycleEvent, DaliInputEventObservedEvent,
    DaliObservedFrameEvent, DaliTargetScope, DecodeStatus, DeviceCommandScope,
    InputDeviceLifecycleKind,
    LevelTransition,
    InputEventKind, LightSetpoint, ObservedFrameWidth, ObservedKind, Origin, PowerState,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::dali::dev103::{
    decode_event, type_event, EventSource, ForwardFrame24, InputEvent, TypedInputEvent,
};
use dali2rust_domain::registry::InputInstanceTypeReadPort;
use dali2rust_domain::dali::devices::dt8_color::{dim_level_to_srgb_channel, Dt8Command};
use dali2rust_domain::dali::devices::DeviceType;
use dali2rust_domain::dali::net::address::{decode_wire_address, DaliAddress};
use dali2rust_domain::dali::pres::codec::dali_command_from_wire;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::pres::DaliCommand;
use dali2rust_domain::registry::mirek_to_kelvin;
use dali2rust_platform::dali::{
    FrameDirection, ObservedRawFrame, ObservedRawFrameKind, SnifferRecord, SnifferTap,
};

const DT8_SET_TEMPORARY_X_COORDINATE_OPCODE: u8 = Dt8Command::SetTemporaryXCoordinate.opcode();
const DT8_SET_TEMPORARY_Y_COORDINATE_OPCODE: u8 = Dt8Command::SetTemporaryYCoordinate.opcode();
const DT8_ACTIVATE_OPCODE: u8 = Dt8Command::Activate.opcode();
const DT8_SET_TEMPERATURE_TC_OPCODE: u8 = Dt8Command::SetTemporaryColourTemperature.opcode();
const DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE: u8 = Dt8Command::SetTemporaryRgbDimLevel.opcode();
const DT8_SET_TEMPORARY_WAF_DIMLEVEL_OPCODE: u8 = Dt8Command::SetTemporaryWafDimLevel.opcode();
const DT8_SET_TEMPORARY_RGBWAF_CONTROL_OPCODE: u8 = Dt8Command::SetTemporaryRgbwafControl.opcode();
const DT8_DEVICE_TYPE: u8 = DeviceType::Color.code();
const DAPC_MASK_LEVEL: u8 = 255;

pub const SNIFFER_TRANSLATOR_REQUIRED_EVENTS: &[&str] = &[
    "DaliInputEventObservedEvent",
    "DaliInputDeviceLifecycleEvent",
    "Dali103ApplicationControlObservedEvent",
];

#[derive(Debug, Default)]
pub struct SnifferTranslatorCounters {
    pub observed_published: AtomicU32,
    pub unknown_seen: AtomicU32,
    pub dimming_unprojectable: AtomicU32,
    pub special_tracked: AtomicU32,
    pub dt8_staged: AtomicU32,
    pub backward_ignored: AtomicU32,
    pub publish_failed: AtomicU32,
    pub input_events_typed: AtomicU32,
    pub input_events_generic: AtomicU32,
    pub input_events_typed_from_registry: AtomicU32,
    pub input_events_ambiguous_scheme: AtomicU32,
    pub input_lifecycle: AtomicU32,
    pub input_publish_retried: AtomicU32,
    pub app_control_pairs: AtomicU32,
    pub scene_writes_observed: AtomicU32,
}

struct ColourStage {
    wire_address: u8,
    x: Option<u16>,
    y: Option<u16>,
    rgb: Option<[u8; 3]>,
    waf: Option<[u8; 3]>,
}

#[derive(Default)]
struct DecoderState {
    dtr0: Option<u8>,
    dtr1: Option<u8>,
    dtr2: Option<u8>,
    dt8_armed: bool,
    colour_stage: Option<ColourStage>,
    pending_device_cmd: Option<([u8; 3], u32)>,
    pending_scene_write: Option<([u8; 3], u32)>,
}

impl DecoderState {
    fn dtr_pair(&self) -> Option<u16> {
        Some(u16::from(self.dtr1?) << 8 | u16::from(self.dtr0?))
    }

    fn dtr_triple(&self) -> Option<[u8; 3]> {
        Some([self.dtr0?, self.dtr1?, self.dtr2?])
    }

    fn colour_stage_for(&mut self, wire_address: u8) -> &mut ColourStage {
        let stale = self
            .colour_stage
            .as_ref()
            .is_none_or(|stage| stage.wire_address != wire_address);
        if stale {
            self.colour_stage = Some(ColourStage {
                wire_address,
                x: None,
                y: None,
                rgb: None,
                waf: None,
            });
        }
        self.colour_stage.as_mut().expect("stage just ensured")
    }
}

pub fn spawn_sniffer_translator_worker(
    raw_rx: Receiver<ObservedRawFrame>,
    publisher: BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: Arc<SnifferTranslatorCounters>,
    sniffer_tap: Option<Arc<SnifferTap>>,
    instance_types: Arc<dyn InputInstanceTypeReadPort>,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack_in(
        c"sniffer_translator_worker",
        std_thread_stack::EVENT_WORKER_STACK,
        esp_thread::StackHome::ExternalOnXip,
        move || {
            let mut state = DecoderState::default();
            while let Ok(raw) = raw_rx.recv() {
                mirror_to_sniffer_window(sniffer_tap.as_deref(), registry_adapter_id, &raw);
                translate_raw_frame(
                    &publisher,
                    bus_id,
                    registry_adapter_id,
                    &counters,
                    &mut state,
                    &raw,
                    instance_types.as_ref(),
                );
                while let Ok(raw) = raw_rx.try_recv() {
                    mirror_to_sniffer_window(sniffer_tap.as_deref(), registry_adapter_id, &raw);
                    translate_raw_frame(
                        &publisher,
                        bus_id,
                        registry_adapter_id,
                        &counters,
                        &mut state,
                        &raw,
                        instance_types.as_ref(),
                    );
                }
            }
        },
    )
}

fn mirror_to_sniffer_window(
    tap: Option<&SnifferTap>,
    registry_adapter_id: u8,
    raw: &ObservedRawFrame,
) {
    let Some(tap) = tap else {
        return;
    };
    let direction = match raw.kind {
        ObservedRawFrameKind::Backward8 => FrameDirection::Reply,
        _ => FrameDirection::ForeignRx,
    };
    tap.record(SnifferRecord {
        at_ms: raw.observed_at_ms,
        direction,
        kind: raw.kind,
        bytes: raw.bytes,
        adapter_id: registry_adapter_id,
        attempt: 0,
    });
}

fn translate_raw_frame(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    state: &mut DecoderState,
    raw: &ObservedRawFrame,
    instance_types: &dyn InputInstanceTypeReadPort,
) {
    if raw.kind != ObservedRawFrameKind::Forward24 {
        state.pending_device_cmd = None;
    }
    if raw.kind != ObservedRawFrameKind::Forward16 {
        state.pending_scene_write = None;
    }
    match raw.kind {
        ObservedRawFrameKind::Backward8 => {
            counters.backward_ignored.fetch_add(1, Ordering::Relaxed);
        }
        ObservedRawFrameKind::Forward24 => {
            translate_forward24(
                publisher,
                bus_id,
                registry_adapter_id,
                counters,
                state,
                raw,
                instance_types,
            );
        }
        ObservedRawFrameKind::Forward16 => {
            translate_forward16(publisher, bus_id, registry_adapter_id, counters, state, raw);
        }
    }
}

#[inline(never)]
fn translate_forward16(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    state: &mut DecoderState,
    raw: &ObservedRawFrame,
) {
    let (wire_address, command) = (raw.bytes[0], raw.bytes[1]);
    let held_scene_write = state.pending_scene_write.take();
    let dt8_armed = std::mem::take(&mut state.dt8_armed);
    let dt8 = decode_dt8(state, dt8_armed, wire_address, command);
    if handle_dt8_outcome(publisher, bus_id, registry_adapter_id, counters, raw, dt8) {
        return;
    }
    match dali_command_from_wire(wire_address, command, 0) {
        Ok(DaliCommand::Special(special)) => {
            track_special(state, counters, special);
        }
        Ok(DaliCommand::Standard { address, command }) => match scene_write_of(command) {
            Some(write) => {
                let publish = Publish { publisher, bus_id, registry_adapter_id, counters };
                track_scene_write(&publish, state, raw, held_scene_write, address, write);
            }
            None => publish_standard(
                publisher,
                bus_id,
                registry_adapter_id,
                counters,
                raw,
                address,
                command,
            ),
        },
        Ok(_) | Err(_) => count_unknown(counters),
    }
}

struct Publish<'a> {
    publisher: &'a BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &'a Arc<SnifferTranslatorCounters>,
}

fn scene_write_of(command: StandardCommand) -> Option<(ObservedKind, u8)> {
    match command {
        StandardCommand::SetScene { scene } => Some((ObservedKind::SceneWriteObserved, scene)),
        StandardCommand::RemoveScene { scene } => Some((ObservedKind::SceneRemovalObserved, scene)),
        _ => None,
    }
}

// IEC 62386-101 §9.3
fn track_scene_write(
    publish: &Publish<'_>,
    state: &mut DecoderState,
    raw: &ObservedRawFrame,
    held: Option<([u8; 3], u32)>,
    address: DaliAddress,
    (kind, scene): (ObservedKind, u8),
) {
    let paired = held.is_some_and(|(first, at)| {
        first == raw.bytes && raw.observed_at_mono_ms.wrapping_sub(at) <= SEND_TWICE_WINDOW_MS
    });
    if !paired {
        state.pending_scene_write = Some((raw.bytes, raw.observed_at_mono_ms));
        return;
    }
    if address == DaliAddress::BroadcastUnaddressed {
        return;
    }
    publish.counters.scene_writes_observed.fetch_add(1, Ordering::Relaxed);
    let (scope, short_address, group_id) = scope_of(address);
    let fact = ObservedFact {
        kind,
        scope,
        short_address,
        group_id,
        scene_id: Some(scene),
        setpoint: None,
        dapc_observed: false,
        level_transition: None,
    };
    publish_observed(
        publish.publisher,
        publish.bus_id,
        publish.registry_adapter_id,
        publish.counters,
        raw,
        fact,
    );
}

fn publish_standard(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    raw: &ObservedRawFrame,
    address: DaliAddress,
    command: StandardCommand,
) {
    match classify_standard(address, command) {
        Some(observed) => {
            publish_observed(publisher, bus_id, registry_adapter_id, counters, raw, observed)
        }
        None if is_unprojectable_dimming(command) => {
            counters
                .dimming_unprojectable
                .fetch_add(1, Ordering::Relaxed);
        }
        None => count_unknown(counters),
    }
}

fn track_special(
    state: &mut DecoderState,
    counters: &Arc<SnifferTranslatorCounters>,
    special: SpecialCommand,
) {
    match special {
        SpecialCommand::Dtr0(v) => state.dtr0 = Some(v),
        SpecialCommand::Dtr1(v) => state.dtr1 = Some(v),
        SpecialCommand::Dtr2(v) => state.dtr2 = Some(v),
        SpecialCommand::EnableDeviceType(DT8_DEVICE_TYPE) => state.dt8_armed = true,
        _ => {}
    }
    counters.special_tracked.fetch_add(1, Ordering::Relaxed);
}

struct ObservedFact {
    kind: ObservedKind,
    scope: DaliTargetScope,
    short_address: Option<u8>,
    group_id: Option<u8>,
    scene_id: Option<u8>,
    setpoint: Option<LightSetpoint>,
    dapc_observed: bool,
    level_transition: Option<LevelTransition>,
}

fn scope_of(address: DaliAddress) -> (DaliTargetScope, Option<u8>, Option<u8>) {
    match address {
        DaliAddress::Short(a) => (DaliTargetScope::Short, Some(a), None),
        DaliAddress::Group(g) => (DaliTargetScope::Group, None, Some(g)),
        DaliAddress::Broadcast | DaliAddress::BroadcastUnaddressed => {
            (DaliTargetScope::Broadcast, None, None)
        }
    }
}

fn classify_standard(address: DaliAddress, command: StandardCommand) -> Option<ObservedFact> {
    let (scope, short_address, group_id) = scope_of(address);
    let shape = |(kind, scene_id, setpoint, dapc_observed)| ObservedFact {
        kind,
        scope,
        short_address,
        group_id,
        scene_id,
        setpoint,
        dapc_observed,
        level_transition: None,
    };
    classify_product_shape(command)
        .map(shape)
        .or_else(|| level_transition_of(command).map(|verb| ObservedFact {
            kind: ObservedKind::LevelTransitionObserved,
            scope,
            short_address,
            group_id,
            scene_id: None,
            setpoint: None,
            dapc_observed: false,
            level_transition: Some(verb),
        }))
}

fn classify_product_shape(
    command: StandardCommand,
) -> Option<(ObservedKind, Option<u8>, Option<LightSetpoint>, bool)> {
    match command {
        StandardCommand::DirectArcPower { level } if level == DAPC_MASK_LEVEL => None,
        StandardCommand::DirectArcPower { level } => Some((
            ObservedKind::TargetStateObserved,
            None,
            Some(level_setpoint(level)),
            true,
        )),
        StandardCommand::Off => Some((
            ObservedKind::TargetStateObserved,
            None,
            Some(LightSetpoint {
                power: PowerState::Off,
                level: 0,
                color: None,
            }),
            false,
        )),
        StandardCommand::GoToScene { scene } => {
            Some((ObservedKind::SceneRecallObserved, Some(scene), None, false))
        }
        _ => None,
    }
}

fn level_transition_of(command: StandardCommand) -> Option<LevelTransition> {
    Some(match command {
        StandardCommand::RecallMaxLevel => LevelTransition::RecallMaxLevel,
        StandardCommand::RecallMinLevel => LevelTransition::RecallMinLevel,
        StandardCommand::StepUp => LevelTransition::StepUp,
        StandardCommand::StepDown => LevelTransition::StepDown,
        StandardCommand::StepDownAndOff => LevelTransition::StepDownAndOff,
        StandardCommand::OnAndStepUp => LevelTransition::OnAndStepUp,
        StandardCommand::GoToLastActiveLevel => LevelTransition::GoToLastActiveLevel,
        _ => return None,
    })
}

// IEC 62386-102 §9.5.6
fn is_unprojectable_dimming(command: StandardCommand) -> bool {
    matches!(command, StandardCommand::Up | StandardCommand::Down)
}

fn level_setpoint(level: u8) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::for_level(level),
        level,
        color: None,
    }
}

fn handle_dt8_outcome(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    raw: &ObservedRawFrame,
    outcome: Dt8Outcome,
) -> bool {
    match outcome {
        Dt8Outcome::NotDt8 => false,
        Dt8Outcome::Staged => {
            counters.dt8_staged.fetch_add(1, Ordering::Relaxed);
            true
        }
        Dt8Outcome::Consumed => true,
        Dt8Outcome::Observed(observed) => {
            publish_observed(publisher, bus_id, registry_adapter_id, counters, raw, observed);
            true
        }
        Dt8Outcome::Ambiguous => {
            count_unknown(counters);
            true
        }
    }
}

enum Dt8Outcome {
    NotDt8,
    Staged,
    Observed(ObservedFact),
    Ambiguous,
    Consumed,
}

fn decode_dt8(
    state: &mut DecoderState,
    dt8_armed: bool,
    wire_address: u8,
    command: u8,
) -> Dt8Outcome {
    if !dt8_armed || wire_address & 0x01 != 1 {
        return Dt8Outcome::NotDt8;
    }
    let Ok(address) = decode_wire_address(wire_address) else {
        return Dt8Outcome::NotDt8;
    };
    match command {
        DT8_SET_TEMPORARY_X_COORDINATE_OPCODE => stage_xy_half(state, wire_address, true),
        DT8_SET_TEMPORARY_Y_COORDINATE_OPCODE => stage_xy_half(state, wire_address, false),
        DT8_ACTIVATE_OPCODE => activate_color(state, wire_address, address),
        DT8_SET_TEMPERATURE_TC_OPCODE => decode_cct(state, address),
        DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE => stage_channels(state, wire_address, true),
        DT8_SET_TEMPORARY_WAF_DIMLEVEL_OPCODE => stage_channels(state, wire_address, false),
        DT8_SET_TEMPORARY_RGBWAF_CONTROL_OPCODE => Dt8Outcome::Consumed,
        _ => Dt8Outcome::NotDt8,
    }
}

fn stage_xy_half(state: &mut DecoderState, wire_address: u8, is_x: bool) -> Dt8Outcome {
    let Some(pair) = state.dtr_pair() else {
        return Dt8Outcome::Ambiguous;
    };
    let stage = state.colour_stage_for(wire_address);
    if is_x {
        stage.x = Some(pair);
    } else {
        stage.y = Some(pair);
    }
    Dt8Outcome::Staged
}

fn stage_channels(state: &mut DecoderState, wire_address: u8, is_rgb: bool) -> Dt8Outcome {
    let Some(levels) = state.dtr_triple() else {
        return Dt8Outcome::Ambiguous;
    };
    let stage = state.colour_stage_for(wire_address);
    if is_rgb {
        stage.rgb = Some(levels);
    } else {
        stage.waf = Some(levels);
    }
    Dt8Outcome::Staged
}

fn activate_color(state: &mut DecoderState, wire_address: u8, address: DaliAddress) -> Dt8Outcome {
    let Some(stage) = state.colour_stage.take() else {
        return Dt8Outcome::Consumed;
    };
    if stage.wire_address != wire_address {
        return Dt8Outcome::Ambiguous;
    }
    match staged_colour(&stage) {
        Some(color) => Dt8Outcome::Observed(color_fact(address, color)),
        None => Dt8Outcome::Ambiguous,
    }
}

fn staged_colour(stage: &ColourStage) -> Option<ColorValue> {
    match (stage.x, stage.y, stage.rgb, stage.waf) {
        (Some(x), Some(y), None, None) => Some(ColorValue {
            x,
            y,
            ..sniffer_color(ColorMode::Xy)
        }),
        (None, None, Some(rgb), None) => Some(channel_colour(ColorMode::Rgb, rgb, [0; 3])),
        (None, None, Some(rgb), Some(waf)) => Some(channel_colour(ColorMode::Rgbwaf, rgb, waf)),
        _ => None,
    }
}

fn decode_cct(state: &DecoderState, address: DaliAddress) -> Dt8Outcome {
    let Some(kelvin) = state.dtr_pair().filter(|m| *m != 0).and_then(mirek_to_kelvin) else {
        return Dt8Outcome::Ambiguous;
    };
    let mut color = sniffer_color(ColorMode::Cct);
    color.color_temperature_kelvin = kelvin;
    Dt8Outcome::Observed(color_fact(address, color))
}

fn channel_colour(mode: ColorMode, rgb: [u8; 3], waf: [u8; 3]) -> ColorValue {
    let [r, g, b] = rgb.map(dim_level_to_srgb_channel);
    let [w, a, f] = waf.map(dim_level_to_srgb_channel);
    ColorValue {
        r,
        g,
        b,
        w,
        a,
        f,
        ..sniffer_color(mode)
    }
}

fn sniffer_color(mode: ColorMode) -> ColorValue {
    ColorValue {
        mode,
        ..ColorValue::default()
    }
}

fn color_fact(address: DaliAddress, color: ColorValue) -> ObservedFact {
    let (scope, short_address, group_id) = scope_of(address);
    ObservedFact {
        kind: ObservedKind::TargetStateObserved,
        scope,
        short_address,
        group_id,
        scene_id: None,
        setpoint: Some(LightSetpoint {
            power: PowerState::Unknown,
            level: 0,
            color: Some(color),
        }),
        dapc_observed: false,
        level_transition: None,
    }
}

// IEC 62386-101 Table 20
const SEND_TWICE_WINDOW_MS: u32 = 105;

// IEC 62386-103 §9.5.1
fn application_control(bytes: [u8; 3]) -> Option<(DeviceCommandScope, bool)> {
    if bytes[1] != 0xFE {
        return None;
    }
    let enable = match bytes[2] {
        0x16 => true,
        0x17 => false,
        _ => return None,
    };
    match bytes[0] {
        0xFF => Some((DeviceCommandScope::Broadcast, enable)),
        0xFD => Some((DeviceCommandScope::Unaddressed, enable)),
        addr if addr & 0x81 == 0x01 => Some((DeviceCommandScope::Short((addr >> 1) & 0x3F), enable)),
        _ => None,
    }
}

fn track_device_command_pair(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    state: &mut DecoderState,
    raw: &ObservedRawFrame,
) {
    let pending = state.pending_device_cmd.take();
    if application_control(raw.bytes).is_none() {
        return;
    }
    match pending {
        Some((first, at)) if first == raw.bytes
            && raw.observed_at_mono_ms.wrapping_sub(at) <= SEND_TWICE_WINDOW_MS =>
        {
            let Some((scope, enable)) = application_control(raw.bytes) else {
                return;
            };
            counters.app_control_pairs.fetch_add(1, Ordering::Relaxed);
            publish_input_fact(
                publisher,
                bus_id,
                counters,
                Dali103ApplicationControlObservedEvent {
                    registry_adapter_id,
                    scope,
                    enable,
                    observed_at_ms: raw.observed_at_ms,
                },
            );
        }
        _ => {
            state.pending_device_cmd = Some((raw.bytes, raw.observed_at_mono_ms));
        }
    }
}

#[inline(never)]
fn translate_forward24(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    state: &mut DecoderState,
    raw: &ObservedRawFrame,
    instance_types: &dyn InputInstanceTypeReadPort,
) {
    let Some(decoded) = decode_event(ForwardFrame24::from_bytes(raw.bytes)) else {
        track_device_command_pair(publisher, bus_id, registry_adapter_id, counters, state, raw);
        count_unknown(counters);
        return;
    };
    state.pending_device_cmd = None;

    match decoded {
        InputEvent::PowerCycle {
            short_address,
            device_group,
        } => {
            counters.input_lifecycle.fetch_add(1, Ordering::Relaxed);
            publish_input_fact(
                publisher,
                bus_id,
                counters,
                DaliInputDeviceLifecycleEvent {
                    registry_adapter_id,
                    kind: InputDeviceLifecycleKind::PowerCycle,
                    short_address,
                    device_group,
                    observed_at_ms: raw.observed_at_ms,
                    observed_at_mono_ms: raw.observed_at_mono_ms,
                },
            );
        }
        InputEvent::Instance { source, info } => publish_instance_event(
            publisher,
            bus_id,
            registry_adapter_id,
            counters,
            raw,
            instance_types,
            &source,
            info,
        ),
    }
}

#[allow(clippy::too_many_arguments, reason = "threaded, not rebuilt per call")]
#[inline(never)]
fn publish_instance_event(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    raw: &ObservedRawFrame,
    instance_types: &dyn InputInstanceTypeReadPort,
    source: &EventSource,
    info: u16,
) {
    count_typed(counters, source, type_event(source.instance_type, info));
    let resolved = resolve_instance_type(registry_adapter_id, source, counters, instance_types);
    publish_input_fact(
        publisher,
        bus_id,
        counters,
        input_event_payload(
            registry_adapter_id,
            source,
            info,
            type_event(resolved, info),
            resolved,
            raw,
        ),
    );
}

fn resolve_instance_type(
    registry_adapter_id: u8,
    source: &EventSource,
    counters: &Arc<SnifferTranslatorCounters>,
    instance_types: &dyn InputInstanceTypeReadPort,
) -> Option<u8> {
    if source.instance_type.is_some() {
        return source.instance_type;
    }
    let (Some(short_address), Some(instance_number)) =
        (source.short_address, source.instance_number)
    else {
        return None;
    };
    let resolved =
        instance_types.input_instance_type(registry_adapter_id, short_address, instance_number);
    if resolved.is_some() {
        counters
            .input_events_typed_from_registry
            .fetch_add(1, Ordering::Relaxed);
    }
    resolved
}

fn input_event_payload(
    registry_adapter_id: u8,
    source: &EventSource,
    info: u16,
    typed: TypedInputEvent,
    instance_type: Option<u8>,
    raw: &ObservedRawFrame,
) -> DaliInputEventObservedEvent {
    let (kind, value) = match typed {
        TypedInputEvent::Button(b) => (InputEventKind::Button, b.code()),
        TypedInputEvent::Occupancy(o) => (InputEventKind::Occupancy, o.to_info()),
        TypedInputEvent::Position(m) => (InputEventKind::Position, m.raw),
        TypedInputEvent::Illuminance(m) => (InputEventKind::Illuminance, m.raw),
        TypedInputEvent::Generic { .. } => (InputEventKind::Generic, 0),
    };
    DaliInputEventObservedEvent {
        registry_adapter_id,
        scheme: source.scheme.code(),
        short_address: source.short_address,
        device_group: source.device_group,
        instance_group: source.instance_group,
        instance_number: source.instance_number,
        instance_type,
        event_info: info,
        typed: kind,
        typed_value: value,
        observed_at_ms: raw.observed_at_ms,
        observed_at_mono_ms: raw.observed_at_mono_ms,
    }
}

fn count_typed(
    counters: &Arc<SnifferTranslatorCounters>,
    source: &EventSource,
    typed: TypedInputEvent,
) {
    if matches!(typed, TypedInputEvent::Generic { .. }) {
        counters.input_events_generic.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.input_events_typed.fetch_add(1, Ordering::Relaxed);
    }
    if !source.scheme.identifies_device() {
        counters
            .input_events_ambiguous_scheme
            .fetch_add(1, Ordering::Relaxed);
    }
}

fn publish_input_fact<P>(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<SnifferTranslatorCounters>,
    payload: P,
) where
    dali2rust_contracts::msg::BusEventPayload: From<P>,
{
    let env = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        CORRELATION_NONE,
        bus_id.0,
        Some(Origin::Sniffer),
        payload,
    );
    let outcome = publish_required(
        publisher,
        BusChannel::Events,
        BusFrame::event(env),
        &REQUIRED_PUBLISH_BACKOFF_MS,
        REQUIRED_PUBLISH_UNCAPPED,
        "sniffer-translator-input-event",
    );
    if outcome.queued {
        counters.observed_published.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.publish_failed.fetch_add(1, Ordering::Relaxed);
    }
    if outcome.retries > 0 {
        counters
            .input_publish_retried
            .fetch_add(outcome.retries, Ordering::Relaxed);
    }
}

fn publish_observed(
    publisher: &BusPublisher,
    bus_id: BusId,
    registry_adapter_id: u8,
    counters: &Arc<SnifferTranslatorCounters>,
    raw: &ObservedRawFrame,
    fact: ObservedFact,
) {
    publish_event(publisher, bus_id, counters, DaliObservedFrameEvent {
        registry_adapter_id,
        observed_kind: fact.kind,
        scope: fact.scope,
        short_address: fact.short_address,
        group_id: fact.group_id,
        scene_id: fact.scene_id,
        setpoint: fact.setpoint,
        dapc_observed: fact.dapc_observed,
        level_transition: fact.level_transition,
        raw_frame: raw.bytes,
        raw_width: ObservedFrameWidth::Forward16,
        decode_status: DecodeStatus::Decoded,
        observed_at_ms: raw.observed_at_ms,
        observed_at_mono_ms: raw.observed_at_mono_ms,
    });
}

fn count_unknown(counters: &Arc<SnifferTranslatorCounters>) {
    counters.unknown_seen.fetch_add(1, Ordering::Relaxed);
}

fn publish_event(
    publisher: &BusPublisher,
    bus_id: BusId,
    counters: &Arc<SnifferTranslatorCounters>,
    event: DaliObservedFrameEvent,
) {
    if publisher.try_publish(
        BusChannel::Events,
        BusFrame::event(event_envelope(
            SOURCE_ID_UNSPECIFIED,
            CORRELATION_NONE,
            bus_id.0,
            Some(Origin::Sniffer),
            event,
        )),
    ) == PublishResult::Queued
    {
        counters.observed_published.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.publish_failed.fetch_add(1, Ordering::Relaxed);
    }
}
