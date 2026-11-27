use dali2rust_contracts::msg::{
    CompactErrorPayload, DaliLevelTransitionAppliedEvent, DaliSceneRecalledEvent, DaliTargetStateAppliedEvent,
    ErrorCode, LightSetpoint, PowerState,
};
use dali2rust_domain::registry::{ClusterPartKeys, ClusterPartView};

use super::*;
use crate::frame::BirthMark;

const MARK: BirthMark = BirthMark { generation: 2, nonce: 9 };
const CHANNEL: RecordRef = RecordRef { number: 18, mark: MARK };
const SCENE: RecordRef = RecordRef { number: 20, mark: MARK };
const GROUP: u8 = 7;
const LOCAL_SCENE: u8 = 4;

struct Parts(Vec<ClusterPartView>);

impl ClusterPartReadPort for Parts {
    fn cluster_part(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        self.0.iter().find(|part| part.kind == kind && part.adapter == adapter && part.local == local).cloned()
    }

    fn cluster_part_keys(&self, _kind: ClusterRecordKind) -> ClusterPartKeys {
        ClusterPartKeys::default()
    }
}

fn part(kind: ClusterRecordKind, local: u8, record: RecordRef, send: bool) -> ClusterPartView {
    ClusterPartView { kind, adapter: 0, local, record, send, accept: true, revision: 1 }
}

fn linked(send: bool) -> Parts {
    Parts(vec![
        part(ClusterRecordKind::Channel, GROUP, CHANNEL, send),
        part(ClusterRecordKind::Scene, LOCAL_SCENE, SCENE, send),
    ])
}

fn settings() -> ClusterSettingsView {
    ClusterSettingsView::default()
}

fn level(value: u8) -> LightSetpoint {
    LightSetpoint { power: PowerState::On, level: Some(value), color: None }
}

fn applied(scope: DaliTargetScope, source: RuntimeSource) -> BusEventPayload {
    DaliTargetStateAppliedEvent {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: None,
        short_address: None,
        group_id: (scope == DaliTargetScope::Group).then_some(GROUP),
        setpoint: level(200),
        dapc_applied: true,
        source,
        applied_at_mono_ms: 0,
        hold_hcl: true,
    }
    .into()
}

fn transition(scope: DaliTargetScope, transition: LevelTransition) -> BusEventPayload {
    DaliLevelTransitionAppliedEvent {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: None,
        short_address: None,
        group_id: Some(GROUP),
        transition,
        source: RuntimeSource::Api,
        applied_at_mono_ms: 0,
        hold_hcl: true,
    }
    .into()
}

fn recalled(scope: DaliTargetScope, source: RuntimeSource) -> DaliSceneRecalledEvent {
    DaliSceneRecalledEvent {
        registry_adapter_id: 0,
        scope,
        short_address: 0,
        group_id: GROUP,
        scene_id: LOCAL_SCENE,
        error: None,
        recalled_at_mono_ms: 0,
        source,
        hold_hcl: true,
        virtual_lamp_id: None,
    }
}

fn decide(payload: &BusEventPayload, settings: &ClusterSettingsView, parts: &Parts) -> Option<Result<ClusterCommandCommand, Skip>> {
    Fact::of(payload).map(|fact| forwarding(&fact, settings, parts))
}

#[test]
fn an_operator_s_group_command_goes_to_the_channel_the_group_sends_to() {
    let request = decide(&applied(DaliTargetScope::Group, RuntimeSource::Api), &settings(), &linked(true));
    let action = Action::Light { target: Target::Channel(CHANNEL), verb: LightVerb::Setpoint(level(200)) };
    let wanted = ClusterCommandCommand { action, source: SourceClass::Operator, hold_hcl: true };
    assert_eq!(request, Some(Ok(wanted)));
}

#[test]
fn a_group_without_a_sending_part_forwards_nothing() {
    let fact = applied(DaliTargetScope::Group, RuntimeSource::Api);
    assert_eq!(decide(&fact, &settings(), &linked(false)), Some(Err(Skip::NotSent)));
    assert_eq!(decide(&fact, &settings(), &Parts(Vec::new())), Some(Err(Skip::NotSent)));
}

#[test]
fn a_command_to_one_lamp_is_no_trigger_at_all() {
    for scope in [DaliTargetScope::Short, DaliTargetScope::VirtualLamp, DaliTargetScope::AddressRange] {
        assert_eq!(decide(&applied(scope, RuntimeSource::Api), &settings(), &linked(true)), None, "{scope:?}");
        let scene = recalled(scope, RuntimeSource::Api).into();
        assert_eq!(decide(&scene, &settings(), &linked(true)), None, "{scope:?}");
    }
}

#[test]
fn a_fact_that_came_over_the_cluster_never_goes_back_whatever_the_switches_say() {
    let all_on = ClusterSettingsView {
        forwarding: ClusterForwarding { operator: true, home_assistant: true, hcl: true, foreign_master: true },
        send_all_lamps: true,
        ..settings()
    };
    for source in [RuntimeSource::Cluster, RuntimeSource::ClusterHcl] {
        for scope in [DaliTargetScope::Group, DaliTargetScope::Broadcast] {
            let fact = applied(scope, source);
            assert_eq!(decide(&fact, &all_on, &linked(true)), Some(Err(Skip::ClusterOrigin)), "{source:?} {scope:?}");
        }
    }
}

#[test]
fn only_the_operator_home_assistant_and_hcl_have_a_switch_and_hcl_starts_off() {
    let fact = |source| applied(DaliTargetScope::Group, source);
    let factory = settings();
    assert!(matches!(decide(&fact(RuntimeSource::Mqtt), &factory, &linked(true)), Some(Ok(_))));
    assert_eq!(decide(&fact(RuntimeSource::Hcl), &factory, &linked(true)), Some(Err(Skip::SourceOff)));
    let hcl_on = ClusterSettingsView { forwarding: ClusterForwarding { hcl: true, ..ClusterForwarding::FACTORY }, ..settings() };
    let hcl = decide(&fact(RuntimeSource::Hcl), &hcl_on, &linked(true));
    assert_eq!(hcl.and_then(Result::ok).map(|request| request.source), Some(SourceClass::Hcl));
    let operator_off = ClusterSettingsView { forwarding: ClusterForwarding::default(), ..settings() };
    assert_eq!(decide(&fact(RuntimeSource::Api), &operator_off, &linked(true)), Some(Err(Skip::SourceOff)));
}

#[test]
fn the_foreign_master_and_the_rules_are_not_forwarded_by_this_stage_whatever_the_switch_says() {
    let every_switch = ClusterSettingsView {
        forwarding: ClusterForwarding { operator: true, home_assistant: true, hcl: true, foreign_master: true },
        ..settings()
    };
    let sources = [
        RuntimeSource::Sniffer,
        RuntimeSource::Rules,
        RuntimeSource::Poller,
        RuntimeSource::Readback,
        RuntimeSource::AdapterProxy,
    ];
    for source in sources {
        let fact = applied(DaliTargetScope::Group, source);
        assert_eq!(decide(&fact, &every_switch, &linked(true)), Some(Err(Skip::SourceOff)), "{source:?}");
    }
}

#[test]
fn a_broadcast_goes_to_all_lamps_only_while_send_all_lamps_is_on() {
    let fact = applied(DaliTargetScope::Broadcast, RuntimeSource::Api);
    assert_eq!(decide(&fact, &settings(), &linked(true)), Some(Err(Skip::NotSent)));
    let sending = ClusterSettingsView { send_all_lamps: true, ..settings() };
    let action = decide(&fact, &sending, &linked(true)).and_then(Result::ok).map(|request| request.action);
    assert_eq!(action, Some(Action::Light { target: Target::All, verb: LightVerb::Setpoint(level(200)) }));
}

#[test]
fn recall_max_and_min_are_forwarded_and_the_relative_verbs_wait_for_their_slice() {
    let forwarded = |verb| decide(&transition(DaliTargetScope::Group, verb), &settings(), &linked(true));
    for verb in [LevelTransition::RecallMaxLevel, LevelTransition::RecallMinLevel] {
        let action = forwarded(verb).and_then(Result::ok).map(|request| request.action);
        assert_eq!(action, Some(Action::Light { target: Target::Channel(CHANNEL), verb: LightVerb::Transition(verb) }));
    }
    for verb in [LevelTransition::Up, LevelTransition::Down, LevelTransition::StepUp, LevelTransition::StepDown] {
        assert_eq!(forwarded(verb), None, "{verb:?}");
    }
}

#[test]
fn a_broadcast_scene_recall_is_a_recall_on_the_whole_node() {
    let fact = recalled(DaliTargetScope::Broadcast, RuntimeSource::Mqtt).into();
    let action = decide(&fact, &settings(), &linked(true)).and_then(Result::ok).map(|request| request.action);
    assert_eq!(action, Some(Action::RecallScene { scene: SCENE, scope: RecallScope::Node }));
}

#[test]
fn a_scene_recalled_on_a_group_needs_both_the_scene_and_the_group_to_send() {
    let fact = recalled(DaliTargetScope::Group, RuntimeSource::Api).into();
    let action = decide(&fact, &settings(), &linked(true)).and_then(Result::ok).map(|request| request.action);
    assert_eq!(action, Some(Action::RecallScene { scene: SCENE, scope: RecallScope::Channel(CHANNEL) }));
    let only_scene = Parts(vec![part(ClusterRecordKind::Scene, LOCAL_SCENE, SCENE, true)]);
    assert_eq!(decide(&fact, &settings(), &only_scene), Some(Err(Skip::NotSent)));
    let group_silent = Parts(vec![
        part(ClusterRecordKind::Channel, GROUP, CHANNEL, false),
        part(ClusterRecordKind::Scene, LOCAL_SCENE, SCENE, true),
    ]);
    assert_eq!(decide(&fact, &settings(), &group_silent), Some(Err(Skip::NotSent)));
}

#[test]
fn a_scene_recall_that_failed_on_the_wire_is_no_trigger() {
    let failed = DaliSceneRecalledEvent {
        error: Some(CompactErrorPayload::new(ErrorCode::ExecutionFailed, "no_answer")),
        ..recalled(DaliTargetScope::Broadcast, RuntimeSource::Api)
    };
    assert_eq!(decide(&failed.into(), &settings(), &linked(true)), None);
}
