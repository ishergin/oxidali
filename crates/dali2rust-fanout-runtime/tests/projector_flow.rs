use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_test_support::{recv_command_matching, wait_until};

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx, PublishResult,
};
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusCommandPayload, BusEventPayload, ColorMode, ColorValue,
    DaliAttributeReadChunk,
    DaliAttributeReadOutcomesEvent, DaliAttributesReadEvent,
    DaliObservedFrameEvent, DaliSceneRecalledEvent, DaliSceneTargetState, DaliTargetScope,
    DaliTargetStateAppliedEvent, DecodeStatus, LastDapcSource, LightSetpoint,
    ObservedFrameWidth, ObservedKind, Origin, PowerState, RegistryRuntimeUpdateCommand,
    RuntimeObservation, RuntimeSource, StatusFlags,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{
    AdapterReadPort, AdapterView, CapabilityFlagsView, GroupApplyRowView, GroupApplySnapshot,
    GroupMembershipMatrixView, GroupReadPort, GroupView, SceneApplyRowView, SceneApplySnapshot,
    SceneMatrixView, SceneReadPort, SceneView, VirtualLampCapabilityReadPort,
    VirtualLampCapabilityView,
};
use dali2rust_fanout_runtime::{spawn_projector_worker, ProjectorCounters};

#[derive(Default)]
struct FakeReadPort {
    lamps: Vec<VirtualLampCapabilityView>,
    group_rows: Vec<GroupApplyRowView>,
    scene_rows: Vec<SceneApplyRowView>,
    group_snapshot_missing: bool,
}

impl AdapterReadPort for FakeReadPort {
    fn adapter_count(&self) -> u8 {
        1
    }
    fn adapter_view(&self, _adapter_id: u8) -> Option<AdapterView> {
        None
    }
    fn list_adapter_views(&self) -> Vec<AdapterView> {
        Vec::new()
    }
}

impl VirtualLampCapabilityReadPort for FakeReadPort {
    fn virtual_lamp_capability_view(&self, _a: u8, l: u8) -> VirtualLampCapabilityView {
        self.lamps
            .iter()
            .find(|v| v.virtual_lamp_id == l)
            .copied()
            .unwrap_or_default()
    }

    fn list_virtual_lamp_capability_views(&self, _a: u8) -> Vec<VirtualLampCapabilityView> {
        self.lamps.clone()
    }
}

impl GroupReadPort for FakeReadPort {
    fn group_view(&self, _a: u8, _g: u8) -> Option<GroupView> {
        None
    }
    fn list_group_views(&self, _a: u8) -> Vec<GroupView> {
        Vec::new()
    }
    fn group_membership_matrix_view(&self, _a: u8) -> Option<GroupMembershipMatrixView> {
        None
    }

    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        if self.group_snapshot_missing {
            return None;
        }
        Some(GroupApplySnapshot {
            adapter_id,
            rows: self.group_rows.clone(),
        })
    }
}

impl SceneReadPort for FakeReadPort {
    fn scene_view(&self, _a: u8, _s: u8) -> Option<SceneView> {
        None
    }
    fn list_scene_views(&self, _a: u8) -> Vec<SceneView> {
        Vec::new()
    }
    fn scene_matrix_view(&self, _a: u8, _s: u8) -> Option<SceneMatrixView> {
        None
    }
    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot> {
        Some(SceneApplySnapshot {
            adapter_id,
            scene_id,
            rows: self.scene_rows.clone(),
        })
    }
}

fn bound_lamp(vl: u8, short: u8) -> VirtualLampCapabilityView {
    VirtualLampCapabilityView {
        virtual_lamp_id: vl,
        binding_short: Some(short),
        ..VirtualLampCapabilityView::default()
    }
}

fn caps(cct: bool, xy: bool, rgb: bool) -> CapabilityFlagsView {
    CapabilityFlagsView {
        brightness: true,
        cct,
        xy,
        rgb,
        ..CapabilityFlagsView::default()
    }
}

fn lamp_with_caps(
    vl: u8,
    short: u8,
    capabilities: CapabilityFlagsView,
) -> VirtualLampCapabilityView {
    VirtualLampCapabilityView {
        virtual_lamp_id: vl,
        binding_short: Some(short),
        capabilities,
    }
}

fn group_row(vl: u8, short: u8) -> GroupApplyRowView {
    GroupApplyRowView {
        virtual_lamp_id: vl,
        desired_groups_mask: 1 << 7,
        applied_groups_mask: 1 << 7,
        binding_short: Some(short),
    }
}

fn cct_color(kelvin: u16) -> ColorValue {
    ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: kelvin,
        ..ColorValue::default()
    }
}

fn rgb_color(r: u8, g: u8, b: u8) -> ColorValue {
    ColorValue {
        mode: ColorMode::Rgb,
        r,
        g,
        b,
        ..ColorValue::default()
    }
}

fn xy_color(x: u16, y: u16) -> ColorValue {
    ColorValue {
        mode: ColorMode::Xy,
        x,
        y,
        ..ColorValue::default()
    }
}

fn color_setpoint(color: ColorValue) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level: None,
        color: Some(color),
    }
}

fn color_by_vl(tap: &BusSubscriberRx, count: usize) -> HashMap<u8, Option<ColorValue>> {
    let mut map = HashMap::new();
    for _ in 0..count {
        let (_, cmd) = recv_runtime_update(tap);
        let vl = cmd.update.virtual_lamp_id.expect("vl target");
        map.insert(vl, cmd.update.setpoint.and_then(|sp| sp.color));
    }
    map
}

struct Harness {
    publisher: BusPublisher,
    tap: BusSubscriberRx,
    counters: Arc<ProjectorCounters>,
    _host: BusHost,
}

fn spawn_harness(port: FakeReadPort) -> Harness {
    let (host, publisher, (ev_rx, cmd_tap)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(32, dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS),
            reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
        )
    });
    let counters = Arc::new(ProjectorCounters::default());
    let _join = spawn_projector_worker(
        ev_rx,
        publisher.clone(),
        BusId::default(),
        Arc::new(port),
        Arc::clone(&counters),
    );
    Harness {
        publisher,
        tap: cmd_tap,
        counters,
        _host: host,
    }
}

fn publish_event(publisher: &BusPublisher, corr: u64, payload: impl Into<dali2rust_contracts::msg::BusEventPayload>) {
    let env = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        BusId::default().0,
        Some(Origin::Internal),
        payload,
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(env)),
        PublishResult::Queued
    );
}

fn recv_runtime_update(tap: &BusSubscriberRx) -> (u64, RegistryRuntimeUpdateCommand) {
    let ce = recv_command_matching(tap, Duration::from_secs(1), |payload| {
        matches!(payload, BusCommandPayload::RegistryRuntimeUpdateCommand(_))
    });
    let BusCommandPayload::RegistryRuntimeUpdateCommand(cmd) = &ce.payload else {
        unreachable!("predicate selected this variant");
    };
    (ce.meta.correlation_id, cmd.clone())
}

fn assert_no_more_updates(tap: &BusSubscriberRx) {
    assert!(
        tap.recv_timeout(Duration::from_millis(100)).is_err(),
        "expected no further runtime updates"
    );
}

fn setpoint(level: u8) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level: Some(level),
        color: None,
    }
}

fn applied(scope: DaliTargetScope, sp: LightSetpoint, dapc: bool) -> DaliTargetStateAppliedEvent {
    applied_from(scope, sp, dapc, RuntimeSource::Api)
}

const PRODUCER_MONO_MS: u32 = 4_242_424;

fn applied_from(
    scope: DaliTargetScope,
    sp: LightSetpoint,
    dapc: bool,
    source: RuntimeSource,
) -> DaliTargetStateAppliedEvent {
    DaliTargetStateAppliedEvent {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: None,
        short_address: None,
        group_id: None,
        setpoint: sp,
        dapc_applied: dapc,
        source,
        applied_at_mono_ms: PRODUCER_MONO_MS,
        hold_hcl: true,
    }
}

fn observed(
    kind: ObservedKind,
    scope: DaliTargetScope,
    sp: Option<LightSetpoint>,
    dapc: bool,
) -> DaliObservedFrameEvent {
    DaliObservedFrameEvent {
        registry_adapter_id: 0,
        observed_kind: kind,
        scope,
        short_address: None,
        group_id: None,
        scene_id: None,
        setpoint: sp,
        dapc_observed: dapc,
        level_transition: None,
        raw_frame: [0; 3],
        raw_width: ObservedFrameWidth::Forward16,
        decode_status: DecodeStatus::Decoded,
        observed_at_ms: 42,
        observed_at_mono_ms: PRODUCER_MONO_MS,
    }
}

#[test]
fn applied_virtual_lamp_scope_projects_dual_entry_with_correlation_fan001() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(DaliTargetScope::VirtualLamp, setpoint(180), true);
    body.virtual_lamp_id = Some(12);
    body.short_address = Some(17);
    publish_event(&publisher, 77, body);

    let (corr, cmd) = recv_runtime_update(tap);
    assert_eq!(corr, 77, "product correlation rides into the registry commit");
    assert_eq!(cmd.update.virtual_lamp_id, Some(12));
    assert_eq!(cmd.update.short_address, Some(17));
    assert_eq!(cmd.update.source, RuntimeSource::Api);
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Unknown));
    assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(Some(180)));
    assert_no_more_updates(tap);
}

#[test]
fn applied_color_only_keeps_last_dapc_source_fan015() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let sp = LightSetpoint {
        power: PowerState::Unknown,
        level: None,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 4000,
            ..ColorValue::default()
        }),
    };
    let mut body = applied(DaliTargetScope::Short, sp, false);
    body.short_address = Some(17);
    publish_event(&publisher, 78, body);

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.last_dapc_source, None);
}

#[test]
fn applied_group_scope_expands_applied_membership_only_fan002() {
    let port = FakeReadPort {
        group_rows: vec![
            GroupApplyRowView {
                virtual_lamp_id: 1,
                desired_groups_mask: 1 << 7,
                applied_groups_mask: 1 << 7,
                binding_short: Some(2),
            },
            GroupApplyRowView {
                virtual_lamp_id: 2,
                desired_groups_mask: 1 << 7,
                applied_groups_mask: 1 << 7,
                binding_short: Some(3),
            },
            GroupApplyRowView {
                virtual_lamp_id: 3,
                desired_groups_mask: 1 << 7,
                applied_groups_mask: 0,
                binding_short: Some(4),
            },
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(DaliTargetScope::Group, setpoint(200), true);
    body.group_id = Some(7);
    publish_event(&publisher, 79, body);

    let mut lamps = Vec::new();
    for _ in 0..2 {
        let (corr, cmd) = recv_runtime_update(tap);
        assert_eq!(corr, CORRELATION_NONE, "fan-out entries never confirm");
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
        assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(Some(200)));
        lamps.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    lamps.sort_unstable();
    assert_eq!(lamps, vec![1, 2]);
    assert_no_more_updates(tap);
    assert_eq!(counters.group_expansions.load(Ordering::Relaxed), 1);
}

#[test]
fn applied_group_fanout_carries_the_commands_source() {
    let port = FakeReadPort {
        group_rows: vec![GroupApplyRowView {
            virtual_lamp_id: 1,
            desired_groups_mask: 1 << 7,
            applied_groups_mask: 1 << 7,
            binding_short: Some(2),
        }],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _counters) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied_from(DaliTargetScope::Group, setpoint(200), true, RuntimeSource::Hcl);
    body.group_id = Some(7);
    publish_event(&publisher, 81, body);

    let (_corr, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.source, RuntimeSource::Hcl, "fan-out lost the HCL source");
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(
        obs.value_source,
        Some(RuntimeSource::Hcl),
        "the observation must agree with the fact's source",
    );
}

#[test]
fn every_commit_a_fact_leads_to_says_whether_the_fact_holds_the_schedule() {
    let port = FakeReadPort {
        lamps: vec![bound_lamp(1, 2)],
        group_rows: vec![group_row(1, 2)],
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap) = (h.publisher.clone(), &h.tap);
    let spared = |scope: DaliTargetScope| DaliTargetStateAppliedEvent {
        hold_hcl: false,
        virtual_lamp_id: Some(1),
        short_address: Some(2),
        group_id: Some(7),
        ..applied_from(scope, setpoint(90), true, RuntimeSource::Rules)
    };
    for scope in [DaliTargetScope::VirtualLamp, DaliTargetScope::Group, DaliTargetScope::Broadcast] {
        publish_event(&publisher, CORRELATION_NONE, spared(scope));
        let (_, cmd) = recv_runtime_update(tap);
        assert!(!cmd.update.hold_hcl, "{scope:?}: the rule said hold_hcl false");
        assert_eq!(cmd.update.source, RuntimeSource::Rules, "{scope:?}: provenance stays truthful");
    }
    publish_event(
        &publisher,
        CORRELATION_NONE,
        DaliSceneRecalledEvent {
            hold_hcl: false,
            source: RuntimeSource::Rules,
            ..foreign_recall(DaliTargetScope::Short, 2, 0)
        },
    );
    let (_, cmd) = recv_runtime_update(tap);
    assert!(!cmd.update.hold_hcl, "a recall's rows carry the recall's flag");

    let mut sniffed = observed(
        ObservedKind::TargetStateObserved,
        DaliTargetScope::Short,
        Some(setpoint(30)),
        true,
    );
    sniffed.short_address = Some(2);
    publish_event(&publisher, CORRELATION_NONE, sniffed);
    let (_, cmd) = recv_runtime_update(tap);
    assert!(cmd.update.hold_hcl, "a foreign master's command overrides the schedule");
    assert_no_more_updates(tap);
}

#[test]
fn applied_broadcast_scope_expands_bound_lamps_fan003() {
    let port = FakeReadPort {
        lamps: vec![
            bound_lamp(1, 2),
            bound_lamp(2, 3),
            bound_lamp(3, 4),
            VirtualLampCapabilityView {
                virtual_lamp_id: 9,
                ..VirtualLampCapabilityView::default()
            },
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        80,
        applied(DaliTargetScope::Broadcast, setpoint(100), true),
    );

    let mut lamps = Vec::new();
    for _ in 0..3 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
        lamps.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    lamps.sort_unstable();
    assert_eq!(lamps, vec![1, 2, 3]);
    assert_no_more_updates(tap);
    assert_eq!(counters.skipped_unbound.load(Ordering::Relaxed), 1);
}

#[test]
fn applied_group_level_only_ignores_capability_fan016() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(DaliTargetScope::Group, setpoint(200), true);
    body.group_id = Some(7);
    publish_event(&publisher, 90, body);

    let mut lamps = Vec::new();
    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(Some(200)));
        assert!(cmd
            .update
            .setpoint
            .as_ref()
            .and_then(|sp| sp.color)
            .is_none());
        lamps.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    lamps.sort_unstable();
    assert_eq!(lamps, vec![1, 2]);
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_cct_filters_rgb_only_member_fan017() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(DaliTargetScope::Group, color_setpoint(cct_color(4000)), false);
    body.group_id = Some(7);
    publish_event(&publisher, 91, body);

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert_eq!(
        by_vl[&1].as_ref().map(|c| c.color_temperature_kelvin),
        Some(4000)
    );
    assert!(
        by_vl[&2].is_none(),
        "rgb-only sibling must not receive the cct colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_rgb_filters_cct_only_member_fan018() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(
        DaliTargetScope::Group,
        color_setpoint(rgb_color(10, 20, 30)),
        false,
    );
    body.group_id = Some(7);
    publish_event(&publisher, 92, body);

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&2].as_ref().map(|c| c.mode), Some(ColorMode::Rgb));
    assert_eq!(
        by_vl[&2].as_ref().map(|c| (c.r, c.g, c.b)),
        Some((10, 20, 30))
    );
    assert!(
        by_vl[&1].is_none(),
        "cct-only sibling must not receive the rgb colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_xy_filters_cct_only_member_fan019() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(false, true, false)),
            lamp_with_caps(2, 3, caps(true, false, false)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(
        DaliTargetScope::Group,
        color_setpoint(xy_color(0x1234, 0x5678)),
        false,
    );
    body.group_id = Some(7);
    publish_event(&publisher, 93, body);

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Xy));
    assert_eq!(
        by_vl[&1].as_ref().map(|c| (c.x, c.y)),
        Some((0x1234, 0x5678))
    );
    assert!(
        by_vl[&2].is_none(),
        "cct-only sibling must not receive the xy colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_dual_capable_member_accepts_cct_fan020() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2)],
        lamps: vec![lamp_with_caps(1, 2, caps(true, false, true))],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(DaliTargetScope::Group, color_setpoint(cct_color(3000)), false);
    body.group_id = Some(7);
    publish_event(&publisher, 94, body);

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Cct)
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_dual_capable_member_accepts_rgb_fan021() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2)],
        lamps: vec![lamp_with_caps(1, 2, caps(true, false, true))],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(
        DaliTargetScope::Group,
        color_setpoint(rgb_color(1, 2, 3)),
        false,
    );
    body.group_id = Some(7);
    publish_event(&publisher, 95, body);

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Rgb)
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_broadcast_cct_filters_rgb_only_member_fan022() {
    let port = FakeReadPort {
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        96,
        applied(DaliTargetScope::Broadcast, color_setpoint(cct_color(4000)), false),
    );

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert!(
        by_vl[&2].is_none(),
        "rgb-only bound lamp must not receive the cct colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn applied_group_unknown_capability_is_not_filtered_fan023() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2)],
        lamps: vec![lamp_with_caps(1, 2, caps(false, false, false))],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = applied(
        DaliTargetScope::Group,
        color_setpoint(rgb_color(1, 2, 3)),
        false,
    );
    body.group_id = Some(7);
    publish_event(&publisher, 97, body);

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Rgb),
        "unknown capability must fail open, not strip the colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn observed_short_dapc_projects_sniffer_fact_fan010() {
    let port = FakeReadPort {
        lamps: vec![bound_lamp(12, 17)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::TargetStateObserved,
        DaliTargetScope::Short,
        Some(setpoint(200)),
        true,
    );
    body.short_address = Some(17);
    publish_event(&publisher, CORRELATION_NONE, body);

    let (corr, cmd) = recv_runtime_update(tap);
    assert_eq!(corr, CORRELATION_NONE);
    assert_eq!(cmd.update.virtual_lamp_id, Some(12));
    assert_eq!(cmd.update.short_address, Some(17));
    assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Sniffer));
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(obs.value_source, Some(RuntimeSource::Sniffer));
    assert_eq!(obs.last_seen_ms, Some(42));
}

#[test]
fn observed_group_and_broadcast_classify_group_fan011_fan012() {
    let port = FakeReadPort {
        lamps: vec![bound_lamp(1, 2), bound_lamp(2, 3), bound_lamp(3, 4)],
        group_rows: vec![
            GroupApplyRowView {
                virtual_lamp_id: 1,
                desired_groups_mask: 1 << 7,
                applied_groups_mask: 1 << 7,
                binding_short: Some(2),
            },
            GroupApplyRowView {
                virtual_lamp_id: 2,
                desired_groups_mask: 1 << 7,
                applied_groups_mask: 1 << 7,
                binding_short: Some(3),
            },
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut group_fact = observed(
        ObservedKind::TargetStateObserved,
        DaliTargetScope::Group,
        Some(setpoint(100)),
        true,
    );
    group_fact.group_id = Some(7);
    publish_event(&publisher, CORRELATION_NONE, group_fact);
    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
        assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    }

    publish_event(
        &publisher,
        CORRELATION_NONE,
        observed(
            ObservedKind::TargetStateObserved,
            DaliTargetScope::Broadcast,
            Some(setpoint(100)),
            true,
        ),
    );
    for _ in 0..3 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
    }
    assert_no_more_updates(tap);
}

#[test]
fn observed_group_color_filters_incapable_member_fan024() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::TargetStateObserved,
        DaliTargetScope::Group,
        Some(color_setpoint(rgb_color(4, 5, 6))),
        false,
    );
    body.group_id = Some(7);
    publish_event(&publisher, CORRELATION_NONE, body);

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&2].as_ref().map(|c| c.mode), Some(ColorMode::Rgb));
    assert!(
        by_vl[&1].is_none(),
        "cct-only sibling must not receive the sniffed rgb colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn observed_unknown_projects_nothing_sys214() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        CORRELATION_NONE,
        observed(
            ObservedKind::UnknownObserved,
            DaliTargetScope::Broadcast,
            None,
            false,
        ),
    );
    assert_no_more_updates(tap);
    assert!(counters.skipped_unknown_observed.load(Ordering::Relaxed) >= 1);
}

#[test]
fn attribute_read_runtime_status_chunk_projects_fan030() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        81,
        DaliAttributesReadEvent {
            registry_adapter_id: 0,
            short_address: 17,
            last_chunk: false,
            chunk: DaliAttributeReadChunk::RuntimeStatus {
                setpoint: Some(setpoint(90)),
                observation: RuntimeObservation::api_timestamped(1_000),
                read_started_mono_ms: PRODUCER_MONO_MS,
            },
        },
    );
    publish_event(
        &publisher,
        81,
        DaliAttributesReadEvent {
            registry_adapter_id: 0,
            short_address: 17,
            last_chunk: true,
            chunk: DaliAttributeReadChunk::Groups { membership: Some(3) },
        },
    );

    let (corr, cmd) = recv_runtime_update(tap);
    assert_eq!(corr, CORRELATION_NONE);
    assert_eq!(cmd.update.virtual_lamp_id, None);
    assert_eq!(cmd.update.short_address, Some(17));
    assert_eq!(cmd.update.last_dapc_source, None);
    assert_eq!(cmd.update.source, RuntimeSource::Readback);
    assert_no_more_updates(tap);
}

fn scene_rows() -> Vec<SceneApplyRowView> {
    vec![
        SceneApplyRowView {
            virtual_lamp_id: 1,
            desired_included: true,
            desired_target: None,
            applied_included: true,
            applied_target: Some(DaliSceneTargetState {
                power: None,
                level: Some(100),
                color: None,
            }),
            binding_short: Some(2),
        },
        SceneApplyRowView {
            virtual_lamp_id: 2,
            desired_included: true,
            desired_target: None,
            applied_included: true,
            applied_target: Some(DaliSceneTargetState {
                power: None,
                level: Some(120),
                color: Some(ColorValue {
                    mode: ColorMode::Cct,
                    color_temperature_kelvin: 2700,
                    ..Default::default()
                }),
            }),
            binding_short: Some(3),
        },
        SceneApplyRowView {
            virtual_lamp_id: 3,
            desired_included: true,
            desired_target: Some(DaliSceneTargetState {
                power: None,
                level: Some(50),
                color: None,
            }),
            applied_included: false,
            applied_target: None,
            binding_short: Some(4),
        },
    ]
}

#[test]
fn scene_recall_expands_applied_rows_only_fan050_fan051() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        82,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Broadcast,
            short_address: 0,
            group_id: 0,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    let mut projected = Vec::new();
    for _ in 0..2 {
        let (corr, cmd) = recv_runtime_update(tap);
        assert_eq!(corr, CORRELATION_NONE);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
        projected.push((
            cmd.update.virtual_lamp_id.expect("vl target"),
            cmd.update.setpoint.as_ref().and_then(|sp| sp.level),
        ));
    }
    projected.sort_unstable();
    assert_eq!(projected, vec![(1, Some(100)), (2, Some(120))]);
    assert_no_more_updates(tap);
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 1);

    publish_event(
        &publisher,
        83,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Broadcast,
            short_address: 0,
            group_id: 0,
            scene_id: 3,
            error: Some(dali2rust_contracts::msg::CompactErrorPayload::new(
                dali2rust_contracts::msg::ErrorCode::ExecutionFailed,
                "recall_failed",
            )),
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );
    assert_no_more_updates(tap);
}

#[test]
fn a_short_address_recall_projects_the_bound_row_only() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        84,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Short,
            short_address: 3,
            group_id: 0,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Rules,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.virtual_lamp_id, Some(2), "the lamp bound to short 3");
    assert_eq!(cmd.update.setpoint.as_ref().and_then(|sp| sp.level), Some(120));
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
    assert_eq!(
        cmd.update.source,
        RuntimeSource::Rules,
        "the commit names who recalled the scene, not a blanket api"
    );
    assert_eq!(
        cmd.update.observation.as_ref().map(|obs| obs.value_source),
        Some(Some(RuntimeSource::Rules))
    );
    assert_no_more_updates(tap);
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 1);
}

#[test]
fn a_recall_that_names_a_lamp_instead_of_an_address_is_ignored() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        85,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::VirtualLamp,
            short_address: 3,
            group_id: 0,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    assert_no_more_updates(tap);
    wait_until(
        || counters.ignored_events.load(Ordering::Relaxed) >= 1,
        Duration::from_millis(500),
    );
    assert_eq!(
        counters.scene_expansions.load(Ordering::Relaxed),
        0,
        "the DALI worker names the address a recall went to; a lamp scope must not read as broadcast"
    );
}

fn foreign_recall(scope: DaliTargetScope, short_address: u8, group_id: u8) -> DaliSceneRecalledEvent {
    DaliSceneRecalledEvent {
        registry_adapter_id: 0,
        scope,
        short_address,
        group_id,
        scene_id: 3,
        error: None,
        recalled_at_mono_ms: PRODUCER_MONO_MS,
        source: RuntimeSource::Sniffer,
        hold_hcl: true,
        virtual_lamp_id: None,
    }
}

#[test]
fn an_observed_recall_frame_projects_nothing_its_recall_fact_does() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Broadcast,
        None,
        false,
    );
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);
    publish_event(&publisher, CORRELATION_NONE, foreign_recall(DaliTargetScope::Broadcast, 0, 0));

    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    }
    assert_no_more_updates(tap);
    assert_eq!(
        counters.scene_expansions.load(Ordering::Relaxed),
        1,
        "one foreign recall is one expansion: only its recall fact expands"
    );
}

#[test]
fn a_foreign_recall_projects_applied_rows_fan052() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(&publisher, CORRELATION_NONE, foreign_recall(DaliTargetScope::Broadcast, 0, 0));

    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
        assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    }
    assert_no_more_updates(tap);
}

fn scene_rows_capability_case() -> Vec<SceneApplyRowView> {
    let row = |vl: u8, short: u8, level: u8| SceneApplyRowView {
        virtual_lamp_id: vl,
        desired_included: true,
        desired_target: None,
        applied_included: true,
        applied_target: Some(DaliSceneTargetState {
            power: None,
            level: Some(level),
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: 2700,
                ..Default::default()
            }),
        }),
        binding_short: Some(short),
    };
    vec![row(1, 2, 100), row(2, 3, 120)]
}

#[test]
fn scene_recall_filters_incapable_member_colour_fan053() {
    let port = FakeReadPort {
        scene_rows: scene_rows_capability_case(),
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        98,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Broadcast,
            short_address: 0,
            group_id: 0,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(tap);
        let vl = cmd.update.virtual_lamp_id.expect("vl target");
        let sp = cmd.update.setpoint.expect("setpoint");
        match vl {
            1 => {
                assert_eq!(sp.level, Some(100));
                assert_eq!(sp.color.map(|c| c.mode), Some(ColorMode::Cct));
            }
            2 => {
                assert_eq!(sp.level, Some(120), "incapable member still gets its row's level");
                assert!(
                    sp.color.is_none(),
                    "rgb-only member must not receive the cct colour"
                );
            }
            other => panic!("unexpected vl {other}"),
        }
    }
    assert_no_more_updates(tap);
}

#[test]
fn a_foreign_recall_filters_incapable_member_colour_fan054() {
    let port = FakeReadPort {
        scene_rows: scene_rows_capability_case(),
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, false, true)),
        ],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(&publisher, CORRELATION_NONE, foreign_recall(DaliTargetScope::Broadcast, 0, 0));

    let by_vl = color_by_vl(tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert!(
        by_vl[&2].is_none(),
        "rgb-only member must not receive the sniffed cct colour"
    );
    assert_no_more_updates(tap);
}

#[test]
fn group_scoped_scene_recall_projects_members_only_fan055() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        group_rows: vec![group_row(1, 2)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        99,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Group,
            short_address: 0,
            group_id: 7,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    let (corr, cmd) = recv_runtime_update(tap);
    assert_eq!(corr, CORRELATION_NONE);
    assert_eq!(cmd.update.virtual_lamp_id, Some(1));
    assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(Some(100)));
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
    assert_no_more_updates(tap);
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 1);
}

#[test]
fn a_foreign_group_recall_projects_members_only_fan056() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        group_rows: vec![group_row(1, 2)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(&publisher, CORRELATION_NONE, foreign_recall(DaliTargetScope::Group, 0, 7));

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.virtual_lamp_id, Some(1));
    assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
    assert_no_more_updates(tap);
}

#[test]
fn a_foreign_short_recall_projects_the_bound_row_only_fan057() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(&publisher, CORRELATION_NONE, foreign_recall(DaliTargetScope::Short, 3, 0));

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.virtual_lamp_id, Some(2));
    assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    assert_no_more_updates(tap);
}

#[test]
fn group_recall_without_a_group_snapshot_is_ignored_not_a_success() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        group_snapshot_missing: true,
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        99,
        DaliSceneRecalledEvent {
            registry_adapter_id: 0,
            scope: DaliTargetScope::Group,
            short_address: 0,
            group_id: 7,
            scene_id: 3,
            error: None,
            recalled_at_mono_ms: PRODUCER_MONO_MS,
            source: RuntimeSource::Api,
            hold_hcl: true,
            virtual_lamp_id: None,
        },
    );

    assert_no_more_updates(tap);
    wait_until(
        || counters.ignored_events.load(Ordering::Relaxed) >= 1,
        Duration::from_millis(500),
    );
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 0);
}

struct QueuedBurst {
    tap: BusSubscriberRx,
    counters: Arc<ProjectorCounters>,
    _host: BusHost,
}

fn lamp_17_port() -> FakeReadPort {
    FakeReadPort {
        lamps: vec![bound_lamp(12, 17)],
        ..FakeReadPort::default()
    }
}

fn project_queued_burst(bodies: Vec<DaliObservedFrameEvent>) -> QueuedBurst {
    project_queued_events(lamp_17_port(), bodies.into_iter().map(BusEventPayload::from).collect())
}

fn project_queued_events(port: FakeReadPort, events: Vec<BusEventPayload>) -> QueuedBurst {
    let (host, publisher, (ev_rx, cmd_tap)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(32, dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS),
            reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
        )
    });
    let queued = u32::try_from(events.len()).expect("a short burst");
    for event in events {
        publish_event(&publisher, CORRELATION_NONE, event);
    }
    wait_until(
        || {
            host.counters_snapshot()
                .event_subscribers
                .first()
                .is_some_and(|s| s.delivered >= queued)
        },
        Duration::from_secs(2),
    );
    let counters = Arc::new(ProjectorCounters::default());
    let _join = spawn_projector_worker(
        ev_rx,
        publisher,
        BusId::default(),
        Arc::new(port),
        Arc::clone(&counters),
    );
    QueuedBurst {
        tap: cmd_tap,
        counters,
        _host: host,
    }
}

fn sniffed_at_17(sp: LightSetpoint, dapc: bool) -> DaliObservedFrameEvent {
    let mut body = observed(
        ObservedKind::TargetStateObserved,
        DaliTargetScope::Short,
        Some(sp),
        dapc,
    );
    body.short_address = Some(17);
    body
}

#[test]
fn observed_burst_for_same_target_coalesces_to_final_value() {
    let burst = [10u8, 20, 30]
        .into_iter()
        .map(|level| sniffed_at_17(setpoint(level), true))
        .collect();
    let projected = project_queued_burst(burst);

    let (_, cmd) = recv_runtime_update(&projected.tap);
    assert_eq!(
        cmd.update.setpoint.as_ref().map(|sp| sp.level),
        Some(Some(30)),
        "only the final value of the burst projects"
    );
    assert_no_more_updates(&projected.tap);
    assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 2);
}

const LATER_MONO_MS: u32 = PRODUCER_MONO_MS + 40;

#[test]
fn a_burst_that_drives_a_level_and_a_colour_commits_both_under_their_own_stamps() {
    let dapc = sniffed_at_17(LightSetpoint::from_level(120, None), true);
    let activated = LightSetpoint {
        power: PowerState::Unknown,
        level: None,
        color: Some(cct_color(3000)),
    };
    let colour = sniffed_at_17(activated, false);
    for [mut first, mut second] in [[dapc.clone(), colour.clone()], [colour, dapc]] {
        first.observed_at_mono_ms = PRODUCER_MONO_MS;
        second.observed_at_mono_ms = LATER_MONO_MS;
        let projected = project_queued_burst(vec![first.clone(), second.clone()]);

        for sent in [&first, &second] {
            let (_, cmd) = recv_runtime_update(&projected.tap);
            assert_eq!(
                cmd.update.setpoint,
                sent.setpoint,
                "a fact the next one does not restate is committed, not folded away"
            );
            assert_eq!(
                cmd.update.observed_at_mono_ms,
                Some(sent.observed_at_mono_ms),
                "each fact keeps its stamp, so the registry orders it against its own commits"
            );
        }
        assert_no_more_updates(&projected.tap);
        assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn a_fade_to_zero_keeps_the_last_level_that_was_on() {
    let burst = [200u8, 150, 0]
        .into_iter()
        .map(|level| sniffed_at_17(LightSetpoint::from_level(level, None), true))
        .collect();
    let projected = project_queued_burst(burst);

    for level in [150u8, 0] {
        let (_, cmd) = recv_runtime_update(&projected.tap);
        assert_eq!(
            cmd.update.setpoint.as_ref().and_then(|sp| sp.level),
            Some(level),
            "a zero does not restate the level lastActiveLevel takes (IEC 62386-102 §9.4)"
        );
    }
    assert_no_more_updates(&projected.tap);
    assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 1);
}

fn stamped(mut body: DaliObservedFrameEvent, after_ms: u32) -> DaliObservedFrameEvent {
    body.observed_at_mono_ms = PRODUCER_MONO_MS + after_ms;
    body
}

fn colour_only(color: ColorValue) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::Unknown,
        level: None,
        color: Some(color),
    }
}

fn assert_commits_in_order(projected: &QueuedBurst, sent: &[DaliObservedFrameEvent], why: &str) {
    for fact in sent {
        let (_, cmd) = recv_runtime_update(&projected.tap);
        assert_eq!(cmd.update.setpoint, fact.setpoint, "{why}");
        assert_eq!(cmd.update.observed_at_mono_ms, Some(fact.observed_at_mono_ms), "{why}");
    }
    assert_no_more_updates(&projected.tap);
}

#[test]
fn an_interleaved_level_and_colour_burst_commits_the_last_fact_of_each() {
    let burst = vec![
        stamped(sniffed_at_17(LightSetpoint::from_level(100, None), true), 0),
        stamped(sniffed_at_17(colour_only(cct_color(3000)), false), 40),
        stamped(sniffed_at_17(LightSetpoint::from_level(120, None), true), 80),
        stamped(sniffed_at_17(colour_only(cct_color(3500)), false), 120),
    ];
    let last_of_each = [burst[2].clone(), burst[3].clone()];
    let projected = project_queued_burst(burst);

    assert_commits_in_order(
        &projected,
        &last_of_each,
        "a tunable-white fade folds per dimension, each fact under its own stamp",
    );
    assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 2);
}

#[test]
fn an_off_after_a_zero_arc_power_keeps_the_arc_power_commit() {
    let off = LightSetpoint {
        power: PowerState::Off,
        level: Some(0),
        color: None,
    };
    let burst = vec![
        stamped(sniffed_at_17(LightSetpoint::from_level(0, None), true), 0),
        stamped(sniffed_at_17(off, false), 40),
    ];
    let projected = project_queued_burst(burst);

    let sources: Vec<_> = (0..2)
        .map(|_| recv_runtime_update(&projected.tap).1.update.last_dapc_source)
        .collect();
    assert_eq!(
        sources,
        [Some(LastDapcSource::Sniffer), None],
        "an OFF is no arc power command, so it does not restate the DAPC before it"
    );
    assert_no_more_updates(&projected.tap);
    assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 0);
}

#[test]
fn a_burst_wider_than_the_held_facts_commits_every_fact_in_order() {
    let off = LightSetpoint {
        power: PowerState::Off,
        level: Some(0),
        color: None,
    };
    let burst = vec![
        stamped(sniffed_at_17(LightSetpoint::from_level(200, None), true), 0),
        stamped(sniffed_at_17(LightSetpoint::from_level(0, None), true), 40),
        stamped(sniffed_at_17(off, false), 80),
        stamped(sniffed_at_17(colour_only(cct_color(3000)), false), 120),
        stamped(sniffed_at_17(colour_only(xy_color(20_000, 20_000)), false), 160),
        stamped(sniffed_at_17(colour_only(rgb_color(4, 5, 6)), false), 200),
    ];
    let projected = project_queued_burst(burst.clone());

    assert_commits_in_order(&projected, &burst, "no fact restates another, so every one commits");
    assert_eq!(projected.counters.coalesced_observed.load(Ordering::Relaxed), 0);
}

#[test]
fn a_group_colour_in_another_mode_does_not_fold_the_colour_before_it() {
    let port = FakeReadPort {
        group_rows: vec![group_row(1, 2), group_row(2, 3)],
        lamps: vec![
            lamp_with_caps(1, 2, caps(true, false, false)),
            lamp_with_caps(2, 3, caps(false, true, false)),
        ],
        ..FakeReadPort::default()
    };
    let group_colour = |color, after_ms| {
        let mut body = observed(
            ObservedKind::TargetStateObserved,
            DaliTargetScope::Group,
            Some(colour_only(color)),
            false,
        );
        body.group_id = Some(7);
        BusEventPayload::from(stamped(body, after_ms))
    };
    let projected = project_queued_events(
        port,
        vec![group_colour(cct_color(3000), 0), group_colour(xy_color(20_000, 20_000), 40)],
    );

    let tc = color_by_vl(&projected.tap, 2);
    assert_eq!(
        tc[&1].as_ref().map(|c| c.mode),
        Some(ColorMode::Cct),
        "the Tc-only member keeps the Tc, since it cannot take the xy that follows"
    );
    let xy = color_by_vl(&projected.tap, 2);
    assert_eq!(xy[&2].as_ref().map(|c| c.mode), Some(ColorMode::Xy));
    assert!(xy[&1].is_none());
    assert_no_more_updates(&projected.tap);
}

#[test]
fn our_own_command_between_two_foreign_facts_keeps_its_place() {
    let foreign_level = stamped(sniffed_at_17(LightSetpoint::from_level(100, None), true), 0);
    let mut own = applied(DaliTargetScope::Short, setpoint(200), true);
    own.short_address = Some(17);
    own.applied_at_mono_ms = PRODUCER_MONO_MS + 40;
    let foreign_colour = stamped(sniffed_at_17(colour_only(cct_color(3000)), false), 80);
    let projected = project_queued_events(
        lamp_17_port(),
        vec![foreign_level.into(), own.into(), foreign_colour.into()],
    );

    let stamps: Vec<_> = (0..3)
        .map(|_| recv_runtime_update(&projected.tap).1.update.observed_at_mono_ms)
        .collect();
    assert_eq!(
        stamps,
        [0, 40, 80].map(|after| Some(PRODUCER_MONO_MS + after)),
        "a foreign fact is never merged across our own commit, so the registry orders all three"
    );
    assert_no_more_updates(&projected.tap);
}

#[test]
fn every_projected_fact_carries_the_producers_stamp_fan060() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);

    let mut body = applied(DaliTargetScope::VirtualLamp, setpoint(180), true);
    body.virtual_lamp_id = Some(12);
    body.short_address = Some(17);
    publish_event(&publisher, 77, body);
    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(
        cmd.update.observed_at_mono_ms,
        Some(PRODUCER_MONO_MS),
        "an applied fact must keep the stamp the worker took when it landed"
    );

    publish_event(
        &publisher,
        0,
        observed(
            ObservedKind::TargetStateObserved,
            DaliTargetScope::Short,
            Some(setpoint(90)),
            true,
        )
        .tap_short(17),
    );
    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(
        cmd.update.observed_at_mono_ms,
        Some(PRODUCER_MONO_MS),
        "a sniffed frame must keep the stamp the transport took at the drain"
    );
}

trait TapShort {
    fn tap_short(self, short: u8) -> Self;
}

impl TapShort for DaliObservedFrameEvent {
    fn tap_short(mut self, short: u8) -> Self {
        self.short_address = Some(short);
        self
    }
}

#[test]
fn a_runtime_status_chunk_projects_the_read_start_stamp_fan061() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    publish_event(
        &publisher,
        81,
        DaliAttributesReadEvent {
            registry_adapter_id: 0,
            short_address: 17,
            last_chunk: true,
            chunk: DaliAttributeReadChunk::RuntimeStatus {
                setpoint: Some(setpoint(90)),
                observation: RuntimeObservation::api_timestamped(1_000),
                read_started_mono_ms: PRODUCER_MONO_MS,
            },
        },
    );

    let (_, cmd) = recv_runtime_update(tap);
    assert_eq!(cmd.update.observed_at_mono_ms, Some(PRODUCER_MONO_MS));
}

#[test]
fn a_fact_with_no_status_read_projects_absent_status_flags_fan062() {
    let port = FakeReadPort {
        lamps: vec![bound_lamp(12, 17)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);

    publish_event(
        &publisher,
        CORRELATION_NONE,
        observed(
            ObservedKind::TargetStateObserved,
            DaliTargetScope::Short,
            Some(setpoint(200)),
            true,
        )
        .tap_short(17),
    );
    let (_, cmd) = recv_runtime_update(tap);
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(
        obs.status_flags, None,
        "a sniffed DAPC frame carries no status byte, and must say so"
    );
    assert_eq!(obs.failure_status, None);
    assert_eq!(obs.error, None);

    let mut body = applied(DaliTargetScope::VirtualLamp, setpoint(180), true);
    body.virtual_lamp_id = Some(12);
    body.short_address = Some(17);
    publish_event(&publisher, 77, body);
    let (_, cmd) = recv_runtime_update(tap);
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(
        obs.status_flags, None,
        "an applied target-state fact does not read status either"
    );
}

#[test]
fn a_runtime_status_chunk_forwards_the_observed_flags_fan063() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut observation = RuntimeObservation::api_timestamped(1_000);
    observation.status_flags = Some(StatusFlags {
        raw: 0x02,
        gear_failure: false,
        lamp_failure: true,
        lamp_on: false,
        limit_error: false,
        fade_running: false,
        reset_state: false,
        missing_short_address: false,
        power_cycle_seen: false,
    });
    publish_event(
        &publisher,
        81,
        DaliAttributesReadEvent {
            registry_adapter_id: 0,
            short_address: 17,
            last_chunk: true,
            chunk: DaliAttributeReadChunk::RuntimeStatus {
                setpoint: Some(setpoint(90)),
                observation,
                read_started_mono_ms: PRODUCER_MONO_MS,
            },
        },
    );

    let (_, cmd) = recv_runtime_update(tap);
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(
        obs.status_flags.as_ref().map(|s| s.raw),
        Some(0x02),
        "the projector forwards the read's observation and does not rebuild it"
    );
    assert!(obs.status_flags.is_some_and(|s| s.lamp_failure));
}

#[test]
fn a_broadcast_expansion_loses_no_fact_to_a_narrow_ingress() {
    const LAMPS: u8 = 40;
    const TINY_INGRESS: usize = 4;

    let port = FakeReadPort {
        lamps: (1..=LAMPS)
            .map(|vl| lamp_with_caps(vl, vl, caps(true, true, true)))
            .collect(),
        ..FakeReadPort::default()
    };
    let (_host, publisher, (ev_rx, cmd_tap)) = BusHost::spawn(
        BusConfig {
            commands_ingress: TINY_INGRESS,
            ..BusConfig::default()
        },
        |reg| {
            (
                reg.subscribe_events(32, dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS),
                reg.subscribe_commands(
                    usize::from(LAMPS) * 2,
                    dali2rust_contracts::msg::COMMAND_VARIANT_NAMES,
                ),
            )
        },
    );
    let counters = Arc::new(ProjectorCounters::default());
    let _join = spawn_projector_worker(
        ev_rx,
        publisher.clone(),
        BusId::default(),
        Arc::new(port),
        Arc::clone(&counters),
    );

    publish_event(
        &publisher,
        4_242,
        applied(DaliTargetScope::Broadcast, setpoint(100), true),
    );

    let mut seen: Vec<u8> = Vec::new();
    for _ in 0..LAMPS {
        let (_, cmd) = recv_runtime_update(&cmd_tap);
        seen.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        (1..=LAMPS).collect::<Vec<_>>(),
        "every bound lamp must get its fact; a gap here is a lamp left showing \
         pre-command state until an unrelated observation happens along"
    );
    assert_eq!(
        counters.publish_failed.load(Ordering::Relaxed),
        0,
        "with the bounded retry and the inter-batch pause, a narrow ingress \
         must cost time rather than facts"
    );
    assert!(
        counters.runtime_updates_retried.load(Ordering::Relaxed) > 0,
        "the ingress never filled, so this run says nothing about pacing — \
         narrow it further rather than trusting the pass"
    );
}

fn outcomes(short_address: u8, runtime_status: AttributeGroupReadOutcome) -> DaliAttributeReadOutcomesEvent {
    let n = AttributeGroupReadOutcome::NotRequested;
    DaliAttributeReadOutcomesEvent {
        registry_adapter_id: 0,
        short_address,
        identity: n,
        runtime_status,
        common_102: n,
        dt8_color: n,
        dt6_led: n,
        groups: n,
        scenes: n,
        extended: n,
        memory_banks: n,
        scene_colours: n,
    }
}

#[test]
fn a_device_absent_read_projects_a_reachability_fault_fan070() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);

    publish_event(
        &publisher,
        99,
        outcomes(17, AttributeGroupReadOutcome::DeviceAbsent),
    );

    let (corr, cmd) = recv_runtime_update(tap);
    assert_eq!(corr, CORRELATION_NONE, "an absence verdict is not a requested commit");
    let entry = &cmd.update;
    assert_eq!(entry.short_address, Some(17));
    let obs = entry.observation.as_ref().expect("an observation");
    assert_eq!(
        obs.error,
        Some(dali2rust_contracts::msg::ErrorCode::DeviceAbsent),
    );
    let sp = entry.setpoint.as_ref().expect("a setpoint");
    assert_eq!(sp.power, PowerState::Unknown);
    assert_eq!(sp.level, None);
    assert!(sp.color.is_none());
    assert_eq!(obs.last_seen_ms, None);
}

#[test]
fn only_absence_projects_a_fault_not_a_preempt_or_a_success_fan071() {
    for outcome in [
        AttributeGroupReadOutcome::Success,
        AttributeGroupReadOutcome::Preempted,
        AttributeGroupReadOutcome::ContendedAbort,
        AttributeGroupReadOutcome::TransportAbort,
        AttributeGroupReadOutcome::NotAttempted,
        AttributeGroupReadOutcome::SequenceIncomplete,
    ] {
        let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
        publish_event(&publisher, 99, outcomes(17, outcome));
        assert!(
            tap.recv_timeout(Duration::from_millis(120)).is_err(),
            "{outcome:?} must not project a runtime fact"
        );
    }
}
