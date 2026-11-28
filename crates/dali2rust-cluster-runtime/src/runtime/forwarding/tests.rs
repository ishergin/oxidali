use dali2rust_contracts::msg::{
    CompactErrorPayload, DaliLevelTransitionAppliedEvent, DaliSceneRecalledEvent, DaliStopFadeAppliedEvent,
    DaliTargetStateAppliedEvent, DecodeStatus, ErrorCode, ObservedFrameWidth,
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
    transition_by(scope, transition, RuntimeSource::Api)
}

fn transition_by(scope: DaliTargetScope, transition: LevelTransition, source: RuntimeSource) -> BusEventPayload {
    DaliLevelTransitionAppliedEvent {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: None,
        short_address: None,
        group_id: Some(GROUP),
        transition,
        source,
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

const OWNER: ForeignStanding = ForeignStanding { forwards: true, shared: false };

fn decide(payload: &BusEventPayload, settings: &ClusterSettingsView, parts: &Parts) -> Option<Result<ClusterCommandCommand, Skip>> {
    decide_on(payload, settings, parts, OWNER)
}

fn decide_on(
    payload: &BusEventPayload,
    settings: &ClusterSettingsView,
    parts: &Parts,
    wire: ForeignStanding,
) -> Option<Result<ClusterCommandCommand, Skip>> {
    Fact::of(payload).map(|fact| forwarding(&fact, settings, parts, wire))
}

fn observed(kind: ObservedKind, scope: DaliTargetScope) -> DaliObservedFrameEvent {
    DaliObservedFrameEvent {
        registry_adapter_id: 0,
        observed_kind: kind,
        scope,
        short_address: (scope == DaliTargetScope::Short).then_some(3),
        group_id: (scope == DaliTargetScope::Group).then_some(GROUP),
        scene_id: None,
        setpoint: None,
        dapc_observed: false,
        level_transition: None,
        raw_frame: [0; 3],
        raw_width: ObservedFrameWidth::Forward16,
        decode_status: DecodeStatus::Decoded,
        observed_at_ms: 0,
        observed_at_mono_ms: 0,
    }
}

fn foreign_setpoint(scope: DaliTargetScope, setpoint: LightSetpoint) -> BusEventPayload {
    DaliObservedFrameEvent { setpoint: Some(setpoint), ..observed(ObservedKind::TargetStateObserved, scope) }.into()
}

fn foreign_transition(kind: ObservedKind, transition: LevelTransition) -> BusEventPayload {
    DaliObservedFrameEvent { level_transition: Some(transition), ..observed(kind, DaliTargetScope::Group) }.into()
}

fn foreign_on() -> ClusterSettingsView {
    ClusterSettingsView { forwarding: ClusterForwarding { foreign_master: true, ..ClusterForwarding::FACTORY }, ..settings() }
}

fn to_channel(verb: LightVerb) -> ClusterCommandCommand {
    let action = Action::Light { target: Target::Channel(CHANNEL), verb };
    ClusterCommandCommand { action, source: SourceClass::ForeignMaster, hold_hcl: true }
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
fn the_rules_and_the_reading_paths_are_not_forwarded_by_this_stage_whatever_the_switch_says() {
    let every_switch = ClusterSettingsView {
        forwarding: ClusterForwarding { operator: true, home_assistant: true, hcl: true, foreign_master: true },
        ..settings()
    };
    let sources = [RuntimeSource::Rules, RuntimeSource::Poller, RuntimeSource::Readback, RuntimeSource::AdapterProxy];
    for source in sources {
        let fact = applied(DaliTargetScope::Group, source);
        assert_eq!(decide(&fact, &every_switch, &linked(true)), Some(Err(Skip::SourceOff)), "{source:?}");
    }
}

#[test]
fn every_foreign_verb_on_a_group_goes_to_the_channel_as_the_foreign_master() {
    let on_without_level = LightSetpoint { power: PowerState::On, level: None, color: None };
    let cases = [
        (foreign_setpoint(DaliTargetScope::Group, level(200)), LightVerb::Setpoint(level(200))),
        (foreign_transition(ObservedKind::LevelTransitionObserved, LevelTransition::RecallMaxLevel), LightVerb::Transition(LevelTransition::RecallMaxLevel)),
        (foreign_transition(ObservedKind::LevelTransitionObserved, LevelTransition::StepDownAndOff), LightVerb::Transition(LevelTransition::StepDownAndOff)),
        (foreign_transition(ObservedKind::LevelTransitionObserved, LevelTransition::GoToLastActiveLevel), LightVerb::Setpoint(on_without_level)),
        (foreign_transition(ObservedKind::DimmingObserved, LevelTransition::Up), LightVerb::Transition(LevelTransition::Up)),
        (foreign_transition(ObservedKind::DimmingObserved, LevelTransition::Down), LightVerb::Transition(LevelTransition::Down)),
        (observed(ObservedKind::FadeStopObserved, DaliTargetScope::Group).into(), LightVerb::StopFade),
    ];
    for (fact, verb) in cases {
        assert_eq!(decide(&fact, &foreign_on(), &linked(true)), Some(Ok(to_channel(verb.clone()))), "{verb:?}");
    }
}

#[test]
fn a_foreign_command_needs_its_switch_and_never_leaves_for_a_single_address() {
    let dapc = foreign_setpoint(DaliTargetScope::Group, level(80));
    assert_eq!(decide(&dapc, &settings(), &linked(true)), Some(Err(Skip::SourceOff)), "the switch is off from the factory");
    assert_eq!(decide(&foreign_setpoint(DaliTargetScope::Short, level(80)), &foreign_on(), &linked(true)), None);
    assert_eq!(decide(&dapc, &foreign_on(), &linked(false)), Some(Err(Skip::NotSent)));
    let recall = DaliSceneRecalledEvent { scope: DaliTargetScope::Short, ..recalled(DaliTargetScope::Short, RuntimeSource::Sniffer) };
    assert_eq!(decide(&recall.into(), &foreign_on(), &linked(true)), None);
}

#[test]
fn only_a_node_that_may_forward_the_foreign_master_on_its_wire_does() {
    let dapc = foreign_setpoint(DaliTargetScope::Group, level(80));
    let refused = ForeignStanding { forwards: false, shared: true };
    assert_eq!(decide_on(&dapc, &foreign_on(), &linked(true), refused), Some(Err(Skip::Wire)));
    let own = applied(DaliTargetScope::Group, RuntimeSource::Api);
    assert!(matches!(decide_on(&own, &foreign_on(), &linked(true), refused), Some(Ok(_))), "the node's own commands are not the wire's");
    let off = decide_on(&dapc, &settings(), &linked(true), refused);
    assert_eq!(off, Some(Err(Skip::SourceOff)), "a switched-off source is told apart from the wire");
}

#[test]
fn a_foreign_broadcast_and_a_foreign_scene_recall_go_where_their_parts_send() {
    let all_lamps = ClusterSettingsView { send_all_lamps: true, ..foreign_on() };
    let broadcast = foreign_transition(ObservedKind::DimmingObserved, LevelTransition::Up);
    let broadcast = match broadcast {
        BusEventPayload::DaliObservedFrameEvent(body) => DaliObservedFrameEvent { scope: DaliTargetScope::Broadcast, group_id: None, ..body },
        _ => unreachable!("an observation"),
    };
    let request = decide(&broadcast.into(), &all_lamps, &linked(true)).and_then(Result::ok).map(|request| request.action);
    assert_eq!(request, Some(Action::Light { target: Target::All, verb: LightVerb::Transition(LevelTransition::Up) }));
    let scene = decide(&recalled(DaliTargetScope::Broadcast, RuntimeSource::Sniffer).into(), &foreign_on(), &linked(true));
    let wanted = ClusterCommandCommand {
        action: Action::RecallScene { scene: SCENE, scope: RecallScope::Node },
        source: SourceClass::ForeignMaster,
        hold_hcl: true,
    };
    assert_eq!(scene, Some(Ok(wanted)));
}

#[test]
fn an_observation_that_names_no_verb_is_no_trigger() {
    for kind in [ObservedKind::UnknownObserved, ObservedKind::SceneWriteObserved, ObservedKind::SceneRemovalObserved] {
        assert_eq!(decide(&observed(kind, DaliTargetScope::Group).into(), &foreign_on(), &linked(true)), None, "{kind:?}");
    }
    assert_eq!(decide(&observed(ObservedKind::TargetStateObserved, DaliTargetScope::Group).into(), &foreign_on(), &linked(true)), None);
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
fn every_transition_but_go_to_last_active_level_is_forwarded_as_its_own_verb() {
    let forwarded = |verb| decide(&transition(DaliTargetScope::Group, verb), &settings(), &linked(true));
    let verbs = [
        LevelTransition::RecallMaxLevel,
        LevelTransition::RecallMinLevel,
        LevelTransition::Up,
        LevelTransition::Down,
        LevelTransition::StepUp,
        LevelTransition::StepDown,
        LevelTransition::OnAndStepUp,
        LevelTransition::StepDownAndOff,
    ];
    for verb in verbs {
        let action = forwarded(verb).and_then(Result::ok).map(|request| request.action);
        let wanted = Action::Light { target: Target::Channel(CHANNEL), verb: LightVerb::Transition(verb) };
        assert_eq!(action, Some(wanted), "{verb:?}");
    }
    assert_eq!(forwarded(LevelTransition::GoToLastActiveLevel), None);
}

fn stopped(scope: DaliTargetScope, source: RuntimeSource) -> BusEventPayload {
    DaliStopFadeAppliedEvent {
        registry_adapter_id: 0,
        scope,
        virtual_lamp_id: None,
        short_address: None,
        group_id: (scope == DaliTargetScope::Group).then_some(GROUP),
        source,
        applied_at_mono_ms: 0,
    }
    .into()
}

#[test]
fn a_stopped_fade_on_a_group_or_on_all_lamps_goes_out_as_a_stop_fade() {
    let on_group = decide(&stopped(DaliTargetScope::Group, RuntimeSource::Api), &settings(), &linked(true));
    let request = on_group.and_then(Result::ok);
    let wanted = Action::Light { target: Target::Channel(CHANNEL), verb: LightVerb::StopFade };
    assert_eq!(request.map(|request| (request.action, request.source)), Some((wanted, SourceClass::Operator)));
    let sending = ClusterSettingsView { send_all_lamps: true, ..settings() };
    let on_all = decide(&stopped(DaliTargetScope::Broadcast, RuntimeSource::Mqtt), &sending, &linked(true));
    let action = on_all.and_then(Result::ok).map(|request| request.action);
    assert_eq!(action, Some(Action::Light { target: Target::All, verb: LightVerb::StopFade }));
}

#[test]
fn a_relative_verb_the_cluster_executed_here_never_goes_back() {
    let all_on = ClusterSettingsView { send_all_lamps: true, ..settings() };
    for source in [RuntimeSource::Cluster, RuntimeSource::ClusterHcl] {
        let step = transition_by(DaliTargetScope::Group, LevelTransition::StepUp, source);
        assert_eq!(decide(&step, &all_on, &linked(true)), Some(Err(Skip::ClusterOrigin)), "{source:?}");
        let stop = stopped(DaliTargetScope::Broadcast, source);
        assert_eq!(decide(&stop, &all_on, &linked(true)), Some(Err(Skip::ClusterOrigin)), "{source:?}");
    }
    let rules = stopped(DaliTargetScope::Group, RuntimeSource::Rules);
    assert_eq!(decide(&rules, &all_on, &linked(true)), Some(Err(Skip::SourceOff)));
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
