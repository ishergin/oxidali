use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_test_support::{recv_command_matching, wait_until};

use dali2rust_bus::{
    BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, BusSubscriberRx, PublishResult,
};
use dali2rust_contracts::msg::{
    AttributeGroupReadOutcome, BusCommandPayload, ColorMode, ColorValue, DaliAttributeReadChunk,
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
        level: 0,
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
        level,
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

    let (corr, cmd) = recv_runtime_update(&tap);
    assert_eq!(corr, 77, "product correlation rides into the registry commit");
    assert_eq!(cmd.update.virtual_lamp_id, Some(12));
    assert_eq!(cmd.update.short_address, Some(17));
    assert_eq!(cmd.update.source, RuntimeSource::Api);
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Unknown));
    assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(180));
    assert_no_more_updates(&tap);
}

#[test]
fn applied_color_only_keeps_last_dapc_source_fan015() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let sp = LightSetpoint {
        power: PowerState::Unknown,
        level: 0,
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 4000,
            ..ColorValue::default()
        }),
    };
    let mut body = applied(DaliTargetScope::Short, sp, false);
    body.short_address = Some(17);
    publish_event(&publisher, 78, body);

    let (_, cmd) = recv_runtime_update(&tap);
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
        let (corr, cmd) = recv_runtime_update(&tap);
        assert_eq!(corr, CORRELATION_NONE, "fan-out entries never confirm");
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
        assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(200));
        lamps.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    lamps.sort_unstable();
    assert_eq!(lamps, vec![1, 2]);
    assert_no_more_updates(&tap);
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

    let (_corr, cmd) = recv_runtime_update(&tap);
    assert_eq!(cmd.update.source, RuntimeSource::Hcl, "fan-out lost the HCL source");
    let obs = cmd.update.observation.expect("observation");
    assert_eq!(
        obs.value_source,
        Some(RuntimeSource::Hcl),
        "the observation must agree with the fact's source",
    );
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
        let (_, cmd) = recv_runtime_update(&tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
        lamps.push(cmd.update.virtual_lamp_id.expect("vl target"));
    }
    lamps.sort_unstable();
    assert_eq!(lamps, vec![1, 2, 3]);
    assert_no_more_updates(&tap);
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
        let (_, cmd) = recv_runtime_update(&tap);
        assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(200));
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
    assert_no_more_updates(&tap);
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

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert_eq!(
        by_vl[&1].as_ref().map(|c| c.color_temperature_kelvin),
        Some(4000)
    );
    assert!(
        by_vl[&2].is_none(),
        "rgb-only sibling must not receive the cct colour"
    );
    assert_no_more_updates(&tap);
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

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&2].as_ref().map(|c| c.mode), Some(ColorMode::Rgb));
    assert_eq!(
        by_vl[&2].as_ref().map(|c| (c.r, c.g, c.b)),
        Some((10, 20, 30))
    );
    assert!(
        by_vl[&1].is_none(),
        "cct-only sibling must not receive the rgb colour"
    );
    assert_no_more_updates(&tap);
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

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Xy));
    assert_eq!(
        by_vl[&1].as_ref().map(|c| (c.x, c.y)),
        Some((0x1234, 0x5678))
    );
    assert!(
        by_vl[&2].is_none(),
        "cct-only sibling must not receive the xy colour"
    );
    assert_no_more_updates(&tap);
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

    let (_, cmd) = recv_runtime_update(&tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Cct)
    );
    assert_no_more_updates(&tap);
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

    let (_, cmd) = recv_runtime_update(&tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Rgb)
    );
    assert_no_more_updates(&tap);
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

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert!(
        by_vl[&2].is_none(),
        "rgb-only bound lamp must not receive the cct colour"
    );
    assert_no_more_updates(&tap);
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

    let (_, cmd) = recv_runtime_update(&tap);
    assert_eq!(
        cmd.update.setpoint.and_then(|sp| sp.color).map(|c| c.mode),
        Some(ColorMode::Rgb),
        "unknown capability must fail open, not strip the colour"
    );
    assert_no_more_updates(&tap);
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

    let (corr, cmd) = recv_runtime_update(&tap);
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
        let (_, cmd) = recv_runtime_update(&tap);
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
        let (_, cmd) = recv_runtime_update(&tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Group));
    }
    assert_no_more_updates(&tap);
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

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&2].as_ref().map(|c| c.mode), Some(ColorMode::Rgb));
    assert!(
        by_vl[&1].is_none(),
        "cct-only sibling must not receive the sniffed rgb colour"
    );
    assert_no_more_updates(&tap);
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
    assert_no_more_updates(&tap);
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

    let (corr, cmd) = recv_runtime_update(&tap);
    assert_eq!(corr, CORRELATION_NONE);
    assert_eq!(cmd.update.virtual_lamp_id, None);
    assert_eq!(cmd.update.short_address, Some(17));
    assert_eq!(cmd.update.last_dapc_source, None);
    assert_eq!(cmd.update.source, RuntimeSource::Readback);
    assert_no_more_updates(&tap);
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
        },
    );

    let mut projected = Vec::new();
    for _ in 0..2 {
        let (corr, cmd) = recv_runtime_update(&tap);
        assert_eq!(corr, CORRELATION_NONE);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
        projected.push((
            cmd.update.virtual_lamp_id.expect("vl target"),
            cmd.update.setpoint.as_ref().map(|sp| sp.level),
        ));
    }
    projected.sort_unstable();
    assert_eq!(projected, vec![(1, Some(100)), (2, Some(120))]);
    assert_no_more_updates(&tap);
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
        },
    );
    assert_no_more_updates(&tap);
}

#[test]
fn observed_scene_recall_projects_applied_rows_fan052() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Broadcast,
        None,
        false,
    );
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);

    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(&tap);
        assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
        assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    }
    assert_no_more_updates(&tap);
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
        },
    );

    for _ in 0..2 {
        let (_, cmd) = recv_runtime_update(&tap);
        let vl = cmd.update.virtual_lamp_id.expect("vl target");
        let sp = cmd.update.setpoint.expect("setpoint");
        match vl {
            1 => {
                assert_eq!(sp.level, 100);
                assert_eq!(sp.color.map(|c| c.mode), Some(ColorMode::Cct));
            }
            2 => {
                assert_eq!(sp.level, 120, "incapable member still gets its row's level");
                assert!(
                    sp.color.is_none(),
                    "rgb-only member must not receive the cct colour"
                );
            }
            other => panic!("unexpected vl {other}"),
        }
    }
    assert_no_more_updates(&tap);
}

#[test]
fn observed_scene_recall_filters_incapable_member_colour_fan054() {
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
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Broadcast,
        None,
        false,
    );
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);

    let by_vl = color_by_vl(&tap, 2);
    assert_eq!(by_vl[&1].as_ref().map(|c| c.mode), Some(ColorMode::Cct));
    assert!(
        by_vl[&2].is_none(),
        "rgb-only member must not receive the sniffed cct colour"
    );
    assert_no_more_updates(&tap);
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
        },
    );

    let (corr, cmd) = recv_runtime_update(&tap);
    assert_eq!(corr, CORRELATION_NONE);
    assert_eq!(cmd.update.virtual_lamp_id, Some(1));
    assert_eq!(cmd.update.setpoint.as_ref().map(|sp| sp.level), Some(100));
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
    assert_no_more_updates(&tap);
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 1);
}

#[test]
fn observed_group_scene_recall_projects_members_only_fan056() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        group_rows: vec![group_row(1, 2)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Group,
        None,
        false,
    );
    body.group_id = Some(7);
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);

    let (_, cmd) = recv_runtime_update(&tap);
    assert_eq!(cmd.update.virtual_lamp_id, Some(1));
    assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    assert_eq!(cmd.update.last_dapc_source, Some(LastDapcSource::Scene));
    assert_no_more_updates(&tap);
}

#[test]
fn observed_short_scene_recall_projects_the_bound_row_only_fan057() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Short,
        None,
        false,
    );
    body.short_address = Some(3);
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);

    let (_, cmd) = recv_runtime_update(&tap);
    assert_eq!(cmd.update.virtual_lamp_id, Some(2));
    assert_eq!(cmd.update.source, RuntimeSource::Sniffer);
    assert_no_more_updates(&tap);
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
        },
    );

    assert_no_more_updates(&tap);
    wait_until(
        || counters.ignored_events.load(Ordering::Relaxed) >= 1,
        Duration::from_millis(500),
    );
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 0);
}

#[test]
fn observed_group_recall_with_oversized_group_id_is_ignored() {
    let port = FakeReadPort {
        scene_rows: scene_rows(),
        group_rows: vec![group_row(1, 2)],
        ..FakeReadPort::default()
    };
    let h = spawn_harness(port);
    let (publisher, tap, counters) = (h.publisher.clone(), &h.tap, &h.counters);
    let mut body = observed(
        ObservedKind::SceneRecallObserved,
        DaliTargetScope::Group,
        None,
        false,
    );
    body.group_id = Some(16);
    body.scene_id = Some(3);
    publish_event(&publisher, CORRELATION_NONE, body);

    assert_no_more_updates(&tap);
    wait_until(
        || counters.ignored_events.load(Ordering::Relaxed) >= 1,
        Duration::from_millis(500),
    );
    assert_eq!(counters.scene_expansions.load(Ordering::Relaxed), 0);
}

#[test]
fn observed_burst_for_same_target_coalesces_to_final_value() {
    let port = FakeReadPort {
        lamps: vec![bound_lamp(12, 17)],
        ..FakeReadPort::default()
    };
    let (host, publisher, (ev_rx, cmd_tap)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_events(32, dali2rust_fanout_runtime::PROJECTOR_HANDLED_EVENTS),
            reg.subscribe_commands(32, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
        )
    });
    for level in [10u8, 20, 30] {
        let mut body = observed(
            ObservedKind::TargetStateObserved,
            DaliTargetScope::Short,
            Some(setpoint(level)),
            true,
        );
        body.short_address = Some(17);
        let env = dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            CORRELATION_NONE,
            BusId::default().0,
            Some(Origin::Internal),
            body,
        );
        assert_eq!(
            publisher.try_publish(BusChannel::Events, BusFrame::event(env)),
            PublishResult::Queued
        );
    }
    wait_until(
        || {
            host.counters_snapshot()
                .event_subscribers
                .first()
                .is_some_and(|s| s.delivered >= 3)
        },
        Duration::from_secs(2),
    );

    let counters = Arc::new(ProjectorCounters::default());
    let _join = spawn_projector_worker(
        ev_rx,
        publisher.clone(),
        BusId::default(),
        Arc::new(port),
        Arc::clone(&counters),
    );

    let (_, cmd) = recv_runtime_update(&cmd_tap);
    assert_eq!(
        cmd.update.setpoint.as_ref().map(|sp| sp.level),
        Some(30),
        "only the final value of the burst projects"
    );
    assert_no_more_updates(&cmd_tap);
    assert_eq!(counters.coalesced_observed.load(Ordering::Relaxed), 2);
}

#[test]
fn every_projected_fact_carries_the_producers_stamp_fan060() {
    let h = spawn_harness(FakeReadPort::default());
    let (publisher, tap, _) = (h.publisher.clone(), &h.tap, &h.counters);

    let mut body = applied(DaliTargetScope::VirtualLamp, setpoint(180), true);
    body.virtual_lamp_id = Some(12);
    body.short_address = Some(17);
    publish_event(&publisher, 77, body);
    let (_, cmd) = recv_runtime_update(&tap);
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
    let (_, cmd) = recv_runtime_update(&tap);
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

    let (_, cmd) = recv_runtime_update(&tap);
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
    let (_, cmd) = recv_runtime_update(&tap);
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
    let (_, cmd) = recv_runtime_update(&tap);
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

    let (_, cmd) = recv_runtime_update(&tap);
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

    let (corr, cmd) = recv_runtime_update(&tap);
    assert_eq!(corr, CORRELATION_NONE, "an absence verdict is not a requested commit");
    let entry = &cmd.update;
    assert_eq!(entry.short_address, Some(17));
    let obs = entry.observation.as_ref().expect("an observation");
    assert_eq!(
        obs.error.as_ref().map(|e| e.code),
        Some(dali2rust_contracts::msg::ErrorCode::DeviceAbsent),
    );
    let sp = entry.setpoint.as_ref().expect("a setpoint");
    assert_eq!(sp.power, PowerState::Unknown);
    assert_eq!(sp.level, 0);
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
