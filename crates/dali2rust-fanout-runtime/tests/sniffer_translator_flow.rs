#![allow(clippy::unusual_byte_groupings, reason = "frame literals group their digits by the fields of a Part 103 frame")]

use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusConfig, BusHost, BusId, BusSubscriberRx};
use std::sync::atomic::Ordering::Relaxed;

use dali2rust_contracts::msg::{
    BusCommandPayload, BusEventPayload, BusUnitNvmRow, ColorMode, DaliInputEventObservedEvent, DaliObservedFrameEvent,
    DaliTargetScope, DecodeStatus, InputDeviceLifecycleKind, InputEventKind,
    LevelTransition, ObservedKind, Origin,
};
use dali2rust_fanout_runtime::{spawn_sniffer_translator_worker, SnifferTranslatorCounters};
use dali2rust_platform::bus_unit::{BusUnitEvidence, BusUnitNvm, BusUnitQuiescence, PowerNotice};
use dali2rust_platform::dali::{ObservedItem, ObservedRawFrame, ObservedRawFrameKind};
use dali2rust_test_support::{
    recv_command_matching, recv_event_matching, try_recv_event_matching_envelope, wait_until,
};

struct FrameTx(std::sync::mpsc::SyncSender<ObservedItem>);

impl FrameTx {
    fn send(&self, frame: ObservedRawFrame) -> Result<(), std::sync::mpsc::SendError<ObservedItem>> {
        self.0.send(ObservedItem::Frame(frame))
    }
}

struct Harness {
    tx: FrameTx,
    ev_tap: BusSubscriberRx,
    cmd_tap: BusSubscriberRx,
    counters: Arc<SnifferTranslatorCounters>,
    _host: BusHost,
}

struct StubInstanceTypes(Option<u8>);

impl dali2rust_domain::registry::InputInstanceTypeReadPort for StubInstanceTypes {
    fn input_instance_type(&self, _adapter: u8, _short: u8, _instance: u8) -> Option<u8> {
        self.0
    }
}

fn spawn_harness() -> Harness {
    spawn_harness_knowing(Some(1))
}

fn spawn_harness_knowing(instance_type: Option<u8>) -> Harness {
    let (host, publisher, (ev_tap, cmd_tap)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(32, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
        )
    });
    let (tx, rx) = sync_channel(16);
    let counters = Arc::new(SnifferTranslatorCounters::default());
    let _join = spawn_sniffer_translator_worker(
        rx,
        publisher,
        BusId::default(),
        0,
        Arc::clone(&counters),
        None,
        Arc::new(StubInstanceTypes(instance_type)),
    );
    Harness {
        tx: FrameTx(tx),
        ev_tap,
        cmd_tap,
        counters,
        _host: host,
    }
}

const OBSERVED_MONO_MS: u32 = 7_654_321;

fn forward16(bytes: [u8; 2]) -> ObservedRawFrame {
    ObservedRawFrame {
        bytes: [bytes[0], bytes[1], 0],
        kind: ObservedRawFrameKind::Forward16,
        observed_at_ms: 42,
        observed_at_mono_ms: OBSERVED_MONO_MS,
    }
}

fn recv_observed(harness: &Harness) -> (Origin, DaliObservedFrameEvent) {
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::DaliObservedFrameEvent(_))
    });
    let BusEventPayload::DaliObservedFrameEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    (ev.meta.origin, body.clone())
}

#[test]
fn foreign_short_dapc_decodes_to_target_state_observed_snif001() {
    let harness = spawn_harness();
    harness.tx.send(forward16([17 << 1, 180])).expect("send");

    let (origin, body) = recv_observed(&harness);
    assert_eq!(origin, Origin::Sniffer);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert_eq!(body.scope, DaliTargetScope::Short);
    assert_eq!(body.short_address, Some(17));
    assert_eq!(body.setpoint.as_ref().map(|sp| sp.level), Some(Some(180)));
    assert!(body.dapc_observed);
    assert_eq!(body.decode_status, DecodeStatus::Decoded);
    assert_eq!(body.observed_at_ms, 42);
}

fn recv_recall(harness: &Harness) -> (Origin, dali2rust_contracts::msg::DaliSceneRecalledEvent) {
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::DaliSceneRecalledEvent(_))
    });
    let BusEventPayload::DaliSceneRecalledEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    (ev.meta.origin, body.clone())
}

#[test]
fn a_foreign_go_to_scene_is_a_recall_fact_for_every_addressed_class() {
    let harness = spawn_harness();
    for (frame, scope, short_address, group_id) in [
        ([0xFF, 0x10 | 3], DaliTargetScope::Broadcast, 0, 0),
        ([0x80 | (2 << 1) | 0x01, 0x10 | 3], DaliTargetScope::Group, 0, 2),
        ([(7 << 1) | 0x01, 0x10 | 3], DaliTargetScope::Short, 7, 0),
    ] {
        harness.tx.send(forward16(frame)).expect("send");
        let (origin, body) = recv_recall(&harness);
        assert_eq!(origin, Origin::Sniffer);
        assert_eq!(
            (body.scope, body.short_address, body.group_id, body.scene_id),
            (scope, short_address, group_id, 3),
            "{frame:02X?}"
        );
        assert_eq!(body.source, dali2rust_contracts::msg::RuntimeSource::Sniffer);
        assert!(body.error.is_none(), "a frame seen on the wire is a recall that happened");
        assert_eq!(body.recalled_at_mono_ms, OBSERVED_MONO_MS, "stamped when the wire carried it");
    }
}

fn recv_translated(harness: &Harness) -> BusEventPayload {
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(
            payload,
            BusEventPayload::DaliObservedFrameEvent(_) | BusEventPayload::DaliSceneRecalledEvent(_)
        )
    });
    ev.payload.clone()
}

#[test]
fn a_foreign_go_to_scene_is_its_recall_fact_and_nothing_else_snif002() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xFF, 0x10 | 3])).expect("send");
    harness.tx.send(forward16([17 << 1, 90])).expect("send");

    let BusEventPayload::DaliSceneRecalledEvent(recall) = recv_translated(&harness) else {
        panic!("a GO TO SCENE is translated into its recall fact first and alone");
    };
    assert_eq!((recall.scope, recall.scene_id), (DaliTargetScope::Broadcast, 3));
    let BusEventPayload::DaliObservedFrameEvent(next) = recv_translated(&harness) else {
        panic!("the DAPC after it is an observation");
    };
    assert_eq!(
        (next.observed_kind, next.short_address),
        (ObservedKind::TargetStateObserved, Some(17)),
        "no recall observation travels beside the fact: every consumer takes the fact"
    );
    wait_until(
        || harness.counters.observed_published.load(Relaxed) >= 2,
        Duration::from_secs(1),
    );
    assert_eq!(
        harness.counters.observed_published.load(Relaxed),
        2,
        "one publish per understood frame"
    );
}

const UNADDRESSED_DAPC: u8 = 0xFC;
const UNADDRESSED_COMMAND: u8 = 0xFD;

#[test]
fn a_frame_for_gear_without_a_short_address_projects_nothing() {
    let harness = spawn_harness();
    for frame in [
        [UNADDRESSED_DAPC, 180],
        [UNADDRESSED_COMMAND, 0x00],
        [UNADDRESSED_COMMAND, 0x05],
        [UNADDRESSED_COMMAND, 0x10 | 5],
        [0xA3, 250],
        [0xC3, 0],
        [0xC1, 8],
        [UNADDRESSED_COMMAND, 231],
        [17 << 1, 90],
    ] {
        harness.tx.send(forward16(frame)).expect("send");
    }

    let BusEventPayload::DaliObservedFrameEvent(body) = recv_translated(&harness) else {
        panic!("DAPC, OFF, RECALL MAX LEVEL, GO TO SCENE and a colour to 0xFC/0xFD reach only \
                gear with no short address, which no record describes; nothing may read them \
                as a broadcast");
    };
    assert_eq!(
        (body.scope, body.short_address, body.setpoint.and_then(|sp| sp.level)),
        (DaliTargetScope::Short, Some(17), Some(90)),
        "the first thing translated is the addressed frame that follows them"
    );
    assert_eq!(
        harness.counters.unaddressed_ignored.load(Relaxed),
        5,
        "the DAPC, the OFF, the RECALL MAX LEVEL, the GO TO SCENE and the colour are each \
         counted where they were dropped, so the translator still accounts for the PHY's frames"
    );
    assert_eq!(harness.counters.unknown_seen.load(Relaxed), 0, "they were understood, not unknown");
}

#[test]
fn unrecognized_frames_are_counted_not_published_snif003() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xA1, 0x05])).expect("send");
    harness
        .tx
        .send(ObservedRawFrame {
            bytes: [0x11, 0x22, 0x33],
            kind: ObservedRawFrameKind::Forward24,
            observed_at_ms: 43,
            observed_at_mono_ms: OBSERVED_MONO_MS,
        })
        .expect("send");
    harness.tx.send(forward16([17 << 1, 180])).expect("send");

    let (_, body) = recv_observed(&harness);
    assert_eq!(
        body.observed_kind,
        ObservedKind::TargetStateObserved,
        "the two unknowns before the DAPC must have published nothing"
    );
    assert!(body.dapc_observed);
    assert_eq!(
        harness.counters.unknown_seen.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "both unknowns are counted where they were seen"
    );
}

#[test]
fn foreign_dt8_cct_sequence_decodes_kelvin_sys215() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xA3, 250])).expect("dtr0");
    harness.tx.send(forward16([0xC3, 0])).expect("dtr1");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 231]))
        .expect("set temperature tc");

    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert_eq!(body.short_address, Some(17));
    assert!(!body.dapc_observed, "colour write is not a DAPC fact");
    let color = body
        .setpoint
        .as_ref()
        .and_then(|sp| sp.color.as_ref())
        .expect("cct color");
    assert_eq!(color.mode, ColorMode::Cct);
    assert_eq!(color.color_temperature_kelvin, 4000);
}

#[test]
fn foreign_dt8_xy_sequence_decodes_raw_pair_sys215() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xA3, 0x33])).expect("dtr0 x lo");
    harness.tx.send(forward16([0xC3, 0x33])).expect("dtr1 x hi");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 224]))
        .expect("set temporary x");
    harness.tx.send(forward16([0xA3, 0x66])).expect("dtr0 y lo");
    harness.tx.send(forward16([0xC3, 0x66])).expect("dtr1 y hi");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 225]))
        .expect("set temporary y");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 226]))
        .expect("activate xy");

    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert_eq!(body.short_address, Some(17));
    assert!(!body.dapc_observed);
    let color = body
        .setpoint
        .as_ref()
        .and_then(|sp| sp.color.as_ref())
        .expect("xy color");
    assert_eq!(color.mode, ColorMode::Xy);
    assert_eq!(color.x, 0x3333);
    assert_eq!(color.y, 0x6666);
}

#[test]
fn foreign_dt8_rgb_write_decodes_triple_sys215() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xA3, 254])).expect("dtr0 r");
    harness.tx.send(forward16([0xC3, 10])).expect("dtr1 g");
    harness.tx.send(forward16([0xC5, 20])).expect("dtr2 b");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 235]))
        .expect("set temporary rgb");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 226]))
        .expect("activate");

    let (_, body) = recv_observed(&harness);
    let color = body
        .setpoint
        .as_ref()
        .and_then(|sp| sp.color.as_ref())
        .expect("rgb color");
    assert_eq!(color.mode, ColorMode::Rgb);
    assert_eq!((color.r, color.g, color.b), (255, 55, 79));
    assert!(!body.dapc_observed);
}

fn stage_channels(harness: &Harness, short: u8, opcode: u8, channels: [u8; 3]) {
    harness.tx.send(forward16([0xA3, channels[0]])).expect("dtr0");
    harness.tx.send(forward16([0xC3, channels[1]])).expect("dtr1");
    harness.tx.send(forward16([0xC5, channels[2]])).expect("dtr2");
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(short << 1) | 1, opcode]))
        .expect("stage channels");
}

#[test]
fn a_foreign_six_channel_write_is_one_rgbwaf_observation() {
    let harness = spawn_harness();
    stage_channels(&harness, 17, 235, [254, 10, 20]);
    stage_channels(&harness, 17, 236, [30, 0, 254]);
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 226]))
        .expect("activate");

    let (_, body) = recv_observed(&harness);
    let color = body
        .setpoint
        .as_ref()
        .and_then(|sp| sp.color.as_ref())
        .expect("rgbwaf color");
    assert_eq!(color.mode, ColorMode::Rgbwaf);
    assert_eq!((color.r, color.g, color.b), (255, 55, 79));
    let wire_to_srgb = dali2rust_domain::dali::devices::dt8_color::dim_level_to_srgb_channel;
    assert_eq!((color.w, color.a, color.f), (wire_to_srgb(30), 0, wire_to_srgb(254)));
    let second = try_recv_event_matching_envelope(&harness.ev_tap, Duration::from_millis(150), |ev| {
        matches!(ev.payload, BusEventPayload::DaliObservedFrameEvent(_))
    });
    assert!(second.is_none(), "one six-channel write published two observations: {second:?}");
}

#[test]
fn a_staged_rgb_is_not_an_observation_before_activate() {
    let harness = spawn_harness();
    stage_channels(&harness, 17, 235, [254, 10, 20]);
    harness.tx.send(forward16([17 << 1, 120])).expect("dapc");

    let (_, body) = recv_observed(&harness);
    assert!(body.dapc_observed, "the staged RGB was published before ACTIVATE");
}

#[test]
fn bare_activate_is_consumed_silently() {
    let harness = spawn_harness();
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(17 << 1) | 1, 226]))
        .expect("bare activate");
    harness.tx.send(forward16([17 << 1, 120])).expect("dapc");

    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert!(body.dapc_observed, "bare activate published nothing");
}

#[test]
fn xy_stage_does_not_leak_across_addresses() {
    let harness = spawn_harness();
    for (dtr, op) in [(0x33u8, 224u8), (0x66, 225)] {
        harness.tx.send(forward16([0xA3, dtr])).expect("dtr0");
        harness.tx.send(forward16([0xC3, dtr])).expect("dtr1");
        harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
        harness
            .tx
            .send(forward16([(17 << 1) | 1, op]))
            .expect("stage half");
    }
    harness.tx.send(forward16([0xC1, 8])).expect("enable dt8");
    harness
        .tx
        .send(forward16([(5 << 1) | 1, 226]))
        .expect("activate other short");
    harness.tx.send(forward16([17 << 1, 120])).expect("dapc");

    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert!(body.dapc_observed, "the ambiguous activate published nothing");
    assert_eq!(
        harness.counters.unknown_seen.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn sniffer_path_publishes_no_commands_snif030() {
    let harness = spawn_harness();
    harness.tx.send(forward16([17 << 1, 180])).expect("send");
    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved);
    assert!(
        harness
            .cmd_tap
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "translator must not publish any bus command"
    );
}

fn forward24(bytes: [u8; 3]) -> ObservedRawFrame {
    ObservedRawFrame {
        bytes,
        kind: ObservedRawFrameKind::Forward24,
        observed_at_ms: 42,
        observed_at_mono_ms: OBSERVED_MONO_MS,
    }
}

fn recv_input_event(harness: &Harness) -> (Origin, DaliInputEventObservedEvent) {
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::DaliInputEventObservedEvent(_))
    });
    let BusEventPayload::DaliInputEventObservedEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    (ev.meta.origin, body.clone())
}

#[test]
fn a_button_press_decodes_to_a_typed_input_event_snif040() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0b0_000011_0, 0b0_00001_00, 0x02]))
        .expect("send");

    let (origin, body) = recv_input_event(&harness);
    assert_eq!(origin, Origin::Sniffer);
    assert_eq!(body.scheme, 1);
    assert_eq!(body.short_address, Some(3));
    assert_eq!(body.instance_type, Some(1));
    assert_eq!(body.event_info, 0x002);
    assert_eq!(body.typed, InputEventKind::Button);
    assert_eq!(body.typed_value, 0x002);
    assert_eq!(body.observed_at_mono_ms, OBSERVED_MONO_MS);
    assert_eq!(harness.counters.input_events_typed.load(Relaxed), 1);
    assert_eq!(harness.counters.input_events_generic.load(Relaxed), 0);
}

#[test]
fn scheme_two_is_typed_from_the_registry_snif041() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0b0_000011_0, 0b1_00000_00, 0x02]))
        .expect("send");

    let (_origin, body) = recv_input_event(&harness);
    assert_eq!(body.scheme, 2);
    assert_eq!(body.short_address, Some(3));
    assert_eq!(body.instance_number, Some(0));
    assert_eq!(
        body.instance_type,
        Some(1),
        "the wire carried no type; the registry's enumeration did"
    );
    assert_eq!(body.typed, InputEventKind::Button);
    assert_eq!(body.typed_value, 0x002, "short press");
    assert_eq!(
        body.event_info, 0x002,
        "the raw event info must survive whatever typed it"
    );
    assert_eq!(
        harness
            .counters
            .input_events_typed_from_registry
            .load(Relaxed),
        1
    );
    assert_eq!(harness.counters.input_events_generic.load(Relaxed), 1);
    assert_eq!(
        harness.counters.input_events_typed.load(Relaxed),
        0,
        "the wire typed nothing here"
    );
    assert_eq!(
        harness.counters.input_events_ambiguous_scheme.load(Relaxed),
        0,
        "scheme 2 names its device — it is not ambiguous"
    );
}
#[test]
fn an_unenumerated_instance_stays_generic_snif047() {
    let harness = spawn_harness_knowing(None);
    harness
        .tx
        .send(forward24([0b0_000011_0, 0b1_00000_00, 0x02]))
        .expect("send");

    let (_origin, body) = recv_input_event(&harness);
    assert_eq!(body.instance_type, None);
    assert_eq!(body.typed, InputEventKind::Generic);
    assert_eq!(body.event_info, 0x002);
    assert_eq!(
        harness
            .counters
            .input_events_typed_from_registry
            .load(Relaxed),
        0
    );
    assert_eq!(harness.counters.input_events_generic.load(Relaxed), 1);
}

#[test]
fn a_registry_typed_sensor_carries_its_reading_snif048() {
    let cases: [(u8, u8, InputEventKind, u16); 3] = [
        (3, 0b0000_0011, InputEventKind::Occupancy, 0b0011),
        (4, 0b0101_0101, InputEventKind::Illuminance, 0b0101_0101),
        (2, 0b0011_1100, InputEventKind::Position, 0b0011_1100),
    ];
    for (instance_type, info, want_kind, want_value) in cases {
        let harness = spawn_harness_knowing(Some(instance_type));
        harness
            .tx
            .send(forward24([0b0_000011_0, 0b1_00000_00, info]))
            .expect("send");

        let (_origin, body) = recv_input_event(&harness);
        assert_eq!(body.scheme, 2);
        assert_eq!(body.instance_type, Some(instance_type));
        assert_eq!(body.typed, want_kind, "type {instance_type}");
        assert_eq!(
            body.typed_value, want_value,
            "type {instance_type}: the reading must survive the typing — a zero \
             here is a sensor that reports nothing and says nothing about it"
        );
        assert_eq!(body.event_info, u16::from(info));
    }
}

#[test]
fn an_occupancy_event_keeps_every_bit_of_its_reading_snif042() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0b0_000101_0, 0b0_00011_00, 0b0000_0011]))
        .expect("send");

    let (_origin, body) = recv_input_event(&harness);
    assert_eq!(body.typed, InputEventKind::Occupancy);
    assert_eq!(body.typed_value, 0b0011);
    assert_eq!(body.instance_type, Some(3));
}

#[test]
fn an_event_without_device_identity_is_published_and_counted_snif043() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0b1_0_00001_0, 0b1_00010_00, 0x002]))
        .expect("send");

    let (_origin, body) = recv_input_event(&harness);
    assert_eq!(body.scheme, 0);
    assert_eq!(body.short_address, None, "scheme 0 cannot name a device");
    assert_eq!(body.instance_type, Some(1));
    assert_eq!(body.typed, InputEventKind::Button);
    assert_eq!(
        harness.counters.input_events_ambiguous_scheme.load(Relaxed),
        1
    );
}

#[test]
fn a_power_notification_becomes_a_lifecycle_event_snif044() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0xFE, 0b111_0_0000, 0b01_000101]))
        .expect("send");

    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::DaliInputDeviceLifecycleEvent(_))
    });
    let BusEventPayload::DaliInputDeviceLifecycleEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    assert_eq!(body.kind, InputDeviceLifecycleKind::PowerCycle);
    assert_eq!(body.short_address, Some(5));
    assert_eq!(harness.counters.input_lifecycle.load(Relaxed), 1);
}

#[test]
fn a_twenty_four_bit_command_is_counted_not_published_snif045() {
    let harness = spawn_harness();
    let before = harness.counters.unknown_seen.load(Relaxed);
    harness.tx.send(forward24([0b0_000011_1, 0xFE, 0x30])).expect("send");

    dali2rust_test_support::wait_until(
        || harness.counters.unknown_seen.load(Relaxed) > before,
        Duration::from_secs(1),
    );
    assert_eq!(harness.counters.input_events_typed.load(Relaxed), 0);
    assert_eq!(harness.counters.input_events_generic.load(Relaxed), 0);
}

#[test]
fn input_events_publish_no_commands_snif046() {
    let harness = spawn_harness();
    harness
        .tx
        .send(forward24([0b0_000011_0, 0b0_00001_00, 0x02]))
        .expect("send");
    let _ = recv_input_event(&harness);
    assert!(
        harness.cmd_tap.try_recv().is_err(),
        "a sniffed frame is published as a fact, never as a command (SNIF-030)"
    );
}

#[test]
fn a_foreign_step_up_is_published_as_a_verb() {
    let h = spawn_harness();
    h.tx.send(forward16([0x07, 0x03])).expect("send");
    let (_origin, body) = recv_observed(&h);
    assert_eq!(body.observed_kind, ObservedKind::LevelTransitionObserved);
    assert_eq!(body.level_transition, Some(LevelTransition::StepUp));
    assert_eq!(body.scope, DaliTargetScope::Short);
    assert_eq!(body.short_address, Some(3));
    assert!(
        body.setpoint.is_none(),
        "the frame names no level, and a setpoint here would invent one"
    );
    assert!(
        !body.dapc_observed,
        "a step is not a DAPC — `last_dapc_source` must not say it was"
    );
}

#[test]
fn foreign_up_and_down_are_counted_apart_from_unknown_frames() {
    let h = spawn_harness();
    let before_unknown = h.counters.unknown_seen.load(Relaxed);
    h.tx.send(forward16([0x07, 0x01])).expect("send");
    h.tx.send(forward16([0x07, 0x02])).expect("send");
    dali2rust_test_support::wait_until(
        || h.counters.dimming_unprojectable.load(Relaxed) >= 2,
        Duration::from_secs(1),
    );
    assert_eq!(h.counters.dimming_unprojectable.load(Relaxed), 2);
    assert_eq!(
        h.counters.unknown_seen.load(Relaxed),
        before_unknown,
        "a frame we understood must not be counted as one we could not read"
    );
}

fn forward16_at(bytes: [u8; 2], mono_ms: u32) -> ObservedRawFrame {
    ObservedRawFrame {
        bytes: [bytes[0], bytes[1], 0],
        kind: ObservedRawFrameKind::Forward16,
        observed_at_ms: 42,
        observed_at_mono_ms: mono_ms,
    }
}

const SET_SCENE_5_TO_SHORT_3: [u8; 2] = [(3 << 1) | 1, 0x45];
const REMOVE_SCENE_5_FROM_SHORT_3: [u8; 2] = [(3 << 1) | 1, 0x55];

fn scene_writes(harness: &Harness) -> u32 {
    harness.counters.scene_writes_observed.load(std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn a_set_scene_pair_is_one_scene_write() {
    let harness = spawn_harness();
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_000)).expect("send");
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_040)).expect("send");
    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::SceneWriteObserved);
    assert_eq!(body.scope, DaliTargetScope::Short);
    assert_eq!(body.short_address, Some(3));
    assert_eq!(body.scene_id, Some(5));
    assert_eq!(scene_writes(&harness), 1);
}

#[test]
fn a_remove_from_scene_pair_is_one_scene_removal() {
    let harness = spawn_harness();
    harness.tx.send(forward16_at(REMOVE_SCENE_5_FROM_SHORT_3, 1_000)).expect("send");
    harness.tx.send(forward16_at(REMOVE_SCENE_5_FROM_SHORT_3, 1_040)).expect("send");
    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::SceneRemovalObserved);
    assert_eq!(body.scene_id, Some(5));
}

#[test]
fn a_set_scene_pair_is_judged_by_its_settling_not_its_stamps() {
    let harness = spawn_harness();
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_000)).expect("send");
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_119)).expect("send");
    let _ = recv_observed(&harness);
    assert_eq!(scene_writes(&harness), 1, "104 ms of settling and a 14,2 ms frame are still a pair");
}

#[test]
fn a_set_scene_pair_past_the_window_is_two_singles() {
    let harness = spawn_harness();
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_000)).expect("send");
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_200)).expect("send");
    harness.tx.send(forward16_at(SET_SCENE_5_TO_SHORT_3, 1_240)).expect("send");
    let _ = recv_observed(&harness);
    assert_eq!(scene_writes(&harness), 1, "IEC 62386-101 Table 20: 200 ms apart is no pair");
}

#[test]
fn an_unaddressed_scene_pair_touches_no_registered_gear() {
    let harness = spawn_harness();
    harness.tx.send(forward16_at([0xFD, 0x45], 1_000)).expect("send");
    harness.tx.send(forward16_at([0xFD, 0x45], 1_040)).expect("send");
    harness.tx.send(forward16_at([17 << 1, 180], 1_080)).expect("send");
    let (_, body) = recv_observed(&harness);
    assert_eq!(body.observed_kind, ObservedKind::TargetStateObserved, "the next fact is the DAPC");
    assert_eq!(scene_writes(&harness), 0);
    assert_eq!(
        harness.counters.unaddressed_ignored.load(Relaxed),
        1,
        "the pair is one command, dropped once"
    );
}

#[test]
fn a_bus_unit_report_reaches_the_registry_with_every_field() {
    let harness = spawn_harness();
    let adopted = BusUnitNvm {
        short_address: Some(3),
        random_address: 0x0012_3456,
        device_groups: 0x0000_0010,
        operating_mode: 0x81,
        power_cycle_notification: false,
        application_active: true,
    };
    let observed = BusUnitNvm {
        short_address: Some(17),
        random_address: 0x00AB_CDEF,
        device_groups: 0x8000_0001,
        operating_mode: 0x80,
        power_cycle_notification: true,
        application_active: false,
    };
    let evidence = BusUnitEvidence { adopted, adopted_revision: 9, observed, wire_sequence: 4 };
    harness.tx.0.send(ObservedItem::BusUnit(evidence)).expect("send");
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::BusUnitNvmObservedEvent(_))
    });
    let BusEventPayload::BusUnitNvmObservedEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    let row = |nvm: BusUnitNvm| BusUnitNvmRow {
        short_address: nvm.short_address,
        random_address: nvm.random_address,
        device_groups: nvm.device_groups,
        operating_mode: nvm.operating_mode,
        power_cycle_notification: nvm.power_cycle_notification,
        application_active: nvm.application_active,
    };
    assert_eq!(body.adopted, row(adopted));
    assert_eq!(body.observed, row(observed));
    assert_eq!((body.adopted_revision, body.wire_sequence), (9, 4));
    assert_eq!(ev.meta.origin, Origin::Sniffer);
    assert_eq!(harness.counters.bus_unit_reports.load(Relaxed), 1);
}

#[test]
fn a_quiescence_report_is_published_as_the_registry_s_fact() {
    let harness = spawn_harness();
    let quiescence = BusUnitQuiescence { sequence: 6, remaining_ms: Some(899_000) };
    harness.tx.0.send(ObservedItem::BusUnitQuiescence(quiescence)).expect("send");
    let ev = recv_event_matching(&harness.ev_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusEventPayload::BusUnitQuiescenceObservedEvent(_))
    });
    let BusEventPayload::BusUnitQuiescenceObservedEvent(body) = &ev.payload else {
        unreachable!("predicate selected this variant");
    };
    assert_eq!((body.sequence, body.remaining_ms), (6, Some(899_000)));
    assert_eq!(ev.meta.origin, Origin::Sniffer);
    assert_eq!(harness.counters.bus_unit_reports.load(Relaxed), 1);
}

#[test]
fn a_testframe_the_bus_unit_accepted_goes_to_the_wire_owner_as_a_command() {
    let harness = spawn_harness();
    let item = ObservedItem::BusUnitTestFrame { data: 0b0101_0011, dtr: [0x07, 0xFE, 0x10] };
    harness.tx.0.send(item).expect("send");
    let ce = recv_command_matching(&harness.cmd_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusCommandPayload::Dali103TestFrameCommand(_))
    });
    let BusCommandPayload::Dali103TestFrameCommand(body) = &ce.payload else {
        unreachable!("predicate selected this variant");
    };
    assert_eq!((body.registry_adapter_id, body.data, body.dtr), (0, 0b0101_0011, [0x07, 0xFE, 0x10]));
    assert_eq!(ce.meta.origin, Origin::Sniffer);
}

// IEC 62386-103 §9.12.2
#[test]
fn a_power_notification_the_bus_unit_raised_goes_to_the_wire_owner_as_a_command() {
    let harness = spawn_harness();
    let notice = PowerNotice { short_address: Some(5), device_groups: 0b0110_0100 };
    harness.tx.0.send(ObservedItem::BusUnitPowerNotification(notice)).expect("send");
    let ce = recv_command_matching(&harness.cmd_tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusCommandPayload::Dali103PowerNotificationCommand(_))
    });
    let BusCommandPayload::Dali103PowerNotificationCommand(body) = &ce.payload else {
        unreachable!("predicate selected this variant");
    };
    assert_eq!((body.registry_adapter_id, body.short_address, body.device_groups), (0, Some(5), 0b0110_0100));
    assert_eq!(ce.meta.origin, Origin::Sniffer);
}

#[test]
fn a_sniffed_send_testframe_is_no_command_of_its_own() {
    let harness = spawn_harness();
    harness.tx.send(forward24([0xC1, 0x33, 0b0000_0010])).expect("send");
    harness.tx.send(forward16([17 << 1, 180])).expect("send");
    let (_, body) = recv_observed(&harness);
    assert_eq!(body.short_address, Some(17), "the frame after it was translated, so it was too");
    assert!(
        harness.cmd_tap.recv_timeout(Duration::from_millis(100)).is_err(),
        "only the bus unit's model, never the sniffed bytes, asks for a test frame"
    );
}
