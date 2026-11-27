use std::sync::atomic::Ordering;
use std::time::Duration;

use dali2rust_bus::{BusConfig, BusHost, BusSubscriberRx};
use dali2rust_contracts::bus::{build_confirmation_envelope, build_confirmation_envelope_with_product_error};
use dali2rust_contracts::msg::{
    CommandEnvelope, ErrorCode, PowerState, COMMAND_VARIANT_NAMES,
};
use dali2rust_domain::registry::{AdapterView, ClusterPartKeys, ClusterSettingsView};

use super::*;
use crate::frame::BirthMark;

const BUS: BusId = BusId(1);
const MARK: BirthMark = BirthMark { generation: 5, nonce: 77 };
const CHANNEL: RecordRef = RecordRef { number: 3, mark: MARK };
const SCENE: RecordRef = RecordRef { number: 9, mark: MARK };
const LAMP: crate::frame::LampAddress = crate::frame::LampAddress { node: 1, adapter: 0, lamp: 4 };
const QUIET: Duration = Duration::from_millis(50);
const PATIENCE: Duration = Duration::from_secs(2);

struct Parts(Vec<ClusterPartView>);

impl ClusterPartReadPort for Parts {
    fn cluster_part(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        self.0.iter().find(|part| part.kind == kind && part.adapter == adapter && part.local == local).cloned()
    }

    fn cluster_part_keys(&self, kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        self.0.iter().filter(|part| part.kind == kind).for_each(|part| keys.insert(part.adapter, part.local));
        keys
    }
}

struct Settings(ClusterSettingsView);

impl ClusterSettingsReadPort for Settings {
    fn cluster_settings_view(&self) -> ClusterSettingsView {
        self.0.clone()
    }
}

struct OneAdapter;

impl AdapterReadPort for OneAdapter {
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

fn stamp(hold_hcl: bool) -> Stamp {
    Stamp { hold_hcl, origin: Origin::Cluster }
}

fn part(kind: ClusterRecordKind, (adapter, local): (u8, u8), record: RecordRef, accept: bool) -> ClusterPartView {
    ClusterPartView { kind, adapter, local, record, send: true, accept, revision: 1 }
}

fn group(adapter: u8, local: u8, accept: bool) -> ClusterPartView {
    part(ClusterRecordKind::Channel, (adapter, local), CHANNEL, accept)
}

fn scene(adapter: u8, local: u8, accept: bool) -> ClusterPartView {
    part(ClusterRecordKind::Scene, (adapter, local), SCENE, accept)
}

struct Rig {
    _host: BusHost,
    execution: ClusterExecution,
    commands: BusSubscriberRx,
    counters: Arc<ClusterCounters>,
}

fn rig(parts: Vec<ClusterPartView>) -> Rig {
    rig_with(parts, ClusterSettingsView::default())
}

fn rig_with(parts: Vec<ClusterPartView>, settings: ClusterSettingsView) -> Rig {
    let (host, publisher, commands) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_commands(16, COMMAND_VARIANT_NAMES));
    let counters = Arc::new(ClusterCounters::default());
    let reads = ExecutionReads {
        parts: Arc::new(Parts(parts)),
        settings: Arc::new(Settings(settings)),
        adapters: Arc::new(OneAdapter),
    };
    let execution = ClusterExecution::new((publisher, BUS), reads, Arc::clone(&counters));
    Rig { _host: host, execution, commands, counters }
}

impl Rig {
    fn published(&self, count: usize) -> Vec<CommandEnvelope> {
        let mut seen = Vec::new();
        while seen.len() < count {
            match self.commands.recv_timeout(PATIENCE) {
                Ok(BusFrame::Command(command)) => seen.push(command.as_ref().clone()),
                Ok(_) => {}
                Err(_) => panic!("{} of {count} commands published", seen.len()),
            }
        }
        assert!(self.commands.recv_timeout(QUIET).is_err(), "more than {count} commands published");
        seen
    }

    fn read(&self, counter: impl Fn(&ClusterCounters) -> &AtomicU32) -> u32 {
        counter(&self.counters).load(Ordering::Relaxed)
    }
}

fn level(value: u8) -> LightSetpoint {
    LightSetpoint { power: PowerState::On, level: Some(value), color: None }
}

fn on_channel(verb: LightVerb) -> Action {
    Action::Light { target: Target::Channel(CHANNEL), verb }
}

#[test]
fn a_neighbour_s_setpoint_runs_on_every_accepting_part_of_the_channel_as_a_cluster_command() {
    let rig = rig(vec![group(0, 3, true), group(1, 5, true), group(0, 7, false)]);
    let outcome = rig.execution.execute(&on_channel(LightVerb::Setpoint(level(200))), stamp(false), Gate::Accepting);
    assert_eq!(outcome, Outcome { unsupported: false, matched: 3, accepted: 2, queued: 2 });
    let published = rig.published(2);
    let expected = [(0, 3), (1, 5)].map(|(adapter, local)| {
        let command = DaliSetTargetStateCommand::for_group(adapter, local, &level(200));
        BusCommandPayload::from(DaliSetTargetStateCommand { hold_hcl: false, ..command })
    });
    for (command, expected) in published.iter().zip(expected) {
        assert_eq!(command.payload, expected);
        assert_eq!(command.meta.origin, Origin::Cluster);
        assert_eq!(command.meta.correlation_id & CORRELATION_TAG_MASK, CORRELATION_TAG);
    }
    assert_eq!(rig.read(|c| &c.commands_executed_total), 2);
}

#[test]
fn this_node_s_own_command_runs_on_its_part_whatever_accept_says() {
    let rig = rig(vec![group(0, 3, false)]);
    let outcome = rig.execution.execute(&on_channel(LightVerb::Setpoint(level(10))), stamp(true), Gate::Own);
    assert_eq!(outcome.queued, 1);
    rig.published(1);
}

#[test]
fn a_plan_counts_this_node_s_parts_and_publishes_nothing() {
    let rig = rig(vec![group(0, 3, false), scene(0, 4, true)]);
    let outcome = rig.execution.plan(&on_channel(LightVerb::Setpoint(level(10))), Gate::Own);
    assert_eq!(outcome, Outcome { unsupported: false, matched: 1, accepted: 1, queued: 0 });
    let recall = Action::RecallScene { scene: SCENE, scope: RecallScope::Node };
    assert_eq!(rig.execution.plan(&recall, Gate::Own).accepted, 1);
    rig.published(0);
    assert_eq!(rig.read(|c| &c.commands_executed_total), 0);
}

#[test]
fn a_part_of_another_channel_with_the_same_birth_mark_is_no_part_of_this_one() {
    let other = RecordRef { number: 4, mark: MARK };
    let rig = rig(vec![part(ClusterRecordKind::Channel, (0, 3), other, true)]);
    let outcome = rig.execution.execute(&on_channel(LightVerb::Setpoint(level(10))), stamp(true), Gate::Accepting);
    assert_eq!(outcome.matched, 0);
    rig.published(0);
}

#[test]
fn a_part_whose_record_has_another_birth_mark_is_no_part_of_the_channel() {
    let reborn = RecordRef { number: 3, mark: BirthMark { generation: 6, nonce: 77 } };
    let rig = rig(vec![part(ClusterRecordKind::Channel, (0, 3), reborn, true)]);
    let outcome = rig.execution.execute(&on_channel(LightVerb::Setpoint(level(10))), stamp(true), Gate::Accepting);
    outcome.count_drop(&rig.counters);
    assert_eq!(outcome.matched, 0);
    assert_eq!(rig.read(|c| &c.commands_dropped_no_part_total), 1);
    assert_eq!(rig.read(|c| &c.commands_dropped_accept_off_total), 0);
    rig.published(0);
}

#[test]
fn a_channel_whose_every_part_refuses_is_counted_as_accept_off() {
    let rig = rig(vec![group(0, 3, false)]);
    let outcome = rig.execution.execute(&on_channel(LightVerb::Setpoint(level(10))), stamp(true), Gate::Accepting);
    outcome.count_drop(&rig.counters);
    assert_eq!(rig.read(|c| &c.commands_dropped_accept_off_total), 1);
    assert_eq!(rig.read(|c| &c.commands_dropped_no_part_total), 0);
    rig.published(0);
}

#[test]
fn recall_max_level_on_a_channel_is_the_group_s_level_preset() {
    let rig = rig(vec![group(0, 3, true)]);
    rig.execution.execute(&on_channel(LightVerb::Transition(LevelTransition::RecallMaxLevel)), stamp(true), Gate::Accepting);
    let expected = DaliRecallLevelPresetCommand::for_group(0, 3, LevelPreset::MaxLevel);
    assert_eq!(rig.published(1)[0].payload, BusCommandPayload::from(expected));
}

#[test]
fn a_scene_recalled_on_the_node_is_broadcast_on_each_accepting_scene_part() {
    let rig = rig(vec![scene(0, 4, true), scene(1, 6, false)]);
    let outcome = rig.execution.execute(&Action::RecallScene { scene: SCENE, scope: RecallScope::Node }, stamp(true), Gate::Accepting);
    assert_eq!((outcome.matched, outcome.queued), (2, 1));
    let expected = DaliRecallSceneCommand::broadcast(0, 4);
    assert_eq!(rig.published(1)[0].payload, BusCommandPayload::from(expected));
}

#[test]
fn a_scene_recalled_on_a_channel_needs_both_parts_to_accept_and_runs_on_the_channel_s_group() {
    let action = Action::RecallScene { scene: SCENE, scope: RecallScope::Channel(CHANNEL) };
    for (scene_accepts, group_accepts, runs) in [(true, true, 1), (false, true, 0), (true, false, 0)] {
        let rig = rig(vec![scene(0, 4, scene_accepts), group(0, 3, group_accepts), scene(1, 6, true)]);
        let outcome = rig.execution.execute(&action, stamp(true), Gate::Accepting);
        assert_eq!(outcome.queued, runs, "scene accepts {scene_accepts}, group accepts {group_accepts}");
        let published = rig.published(usize::from(runs));
        if let Some(command) = published.first() {
            let expected = DaliRecallSceneCommand::for_group(0, 3, 4);
            assert_eq!(command.payload, BusCommandPayload::from(expected));
        }
    }
}

#[test]
fn a_lamp_target_and_go_to_last_active_level_are_not_executed_by_this_stage() {
    let rig = rig(vec![group(0, 3, true)]);
    let actions = [
        on_channel(LightVerb::Transition(LevelTransition::GoToLastActiveLevel)),
        Action::Light { target: Target::All, verb: LightVerb::Transition(LevelTransition::GoToLastActiveLevel) },
        Action::Light { target: Target::Lamp(LAMP), verb: LightVerb::Setpoint(level(1)) },
        Action::Light { target: Target::Lamp(LAMP), verb: LightVerb::StopFade },
    ];
    for action in actions {
        assert!(!is_supported(&action));
        let outcome = rig.execution.execute(&action, stamp(true), Gate::Accepting);
        outcome.count_drop(&rig.counters);
    }
    assert_eq!(rig.read(|c| &c.commands_dropped_unsupported_total), 4);
    rig.published(0);
}

fn relative_verbs() -> Vec<(LightVerb, BusCommandPayload)> {
    let (adapter, group) = (0, 3);
    let transition = |transition, payload: BusCommandPayload| (LightVerb::Transition(transition), payload);
    vec![
        transition(LevelTransition::Up, DaliDimCommand::for_group(adapter, group, DimDirection::Up).into()),
        transition(LevelTransition::Down, DaliDimCommand::for_group(adapter, group, DimDirection::Down).into()),
        transition(LevelTransition::StepUp, DaliStepCommand::for_group(adapter, group, StepKind::Up).into()),
        transition(LevelTransition::StepDown, DaliStepCommand::for_group(adapter, group, StepKind::Down).into()),
        transition(LevelTransition::OnAndStepUp, DaliStepCommand::for_group(adapter, group, StepKind::OnAndUp).into()),
        transition(LevelTransition::StepDownAndOff, DaliStepCommand::for_group(adapter, group, StepKind::DownAndOff).into()),
        (LightVerb::StopFade, DaliStopFadeCommand::for_group(adapter, group).into()),
    ]
}

#[test]
fn each_relative_verb_on_a_channel_runs_as_its_own_semantic_command_on_the_part_s_group() {
    for (verb, expected) in relative_verbs() {
        let rig = rig(vec![group(0, 3, true)]);
        let action = on_channel(verb.clone());
        assert!(is_supported(&action), "{verb:?}");
        let outcome = rig.execution.execute(&action, stamp(true), Gate::Accepting);
        assert_eq!(outcome.queued, 1, "{verb:?}");
        let published = rig.published(1);
        assert_eq!(published[0].payload, expected, "{verb:?}");
        assert_eq!(published[0].meta.origin, Origin::Cluster);
    }
}

#[test]
fn a_relative_verb_to_all_lamps_is_a_broadcast_of_the_same_command() {
    let accepting = ClusterSettingsView { accept_all_lamps: true, ..ClusterSettingsView::default() };
    let rig = rig_with(Vec::new(), accepting);
    rig.execution.execute(&to_all(LightVerb::Transition(LevelTransition::Up)), stamp(false), Gate::Accepting);
    rig.execution.execute(&to_all(LightVerb::Transition(LevelTransition::StepDown)), stamp(true), Gate::Accepting);
    rig.execution.execute(&to_all(LightVerb::StopFade), stamp(true), Gate::Accepting);
    let published = rig.published(3);
    let up = DaliDimCommand { hold_hcl: false, ..DaliDimCommand::broadcast(0, DimDirection::Up) };
    assert_eq!(published[0].payload, BusCommandPayload::from(up));
    assert_eq!(published[1].payload, BusCommandPayload::from(DaliStepCommand::broadcast(0, StepKind::Down)));
    assert_eq!(published[2].payload, BusCommandPayload::from(DaliStopFadeCommand::broadcast(0)));
}

fn to_all(verb: LightVerb) -> Action {
    Action::Light { target: Target::All, verb }
}

#[test]
fn a_command_to_all_lamps_is_a_broadcast_on_every_adapter_once_accept_all_lamps_is_on() {
    let accepting = ClusterSettingsView { accept_all_lamps: true, ..ClusterSettingsView::default() };
    let rig = rig_with(Vec::new(), accepting);
    let setpoint = rig.execution.execute(&to_all(LightVerb::Setpoint(level(90))), stamp(false), Gate::Accepting);
    let preset = to_all(LightVerb::Transition(LevelTransition::RecallMinLevel));
    rig.execution.execute(&preset, stamp(true), Gate::Accepting);
    assert_eq!((setpoint.matched, setpoint.queued), (1, 1));
    let published = rig.published(2);
    let broadcast = DaliSetTargetStateCommand { hold_hcl: false, ..DaliSetTargetStateCommand::broadcast(0, &level(90)) };
    assert_eq!(published[0].payload, BusCommandPayload::from(broadcast));
    let min = DaliRecallLevelPresetCommand::broadcast(0, LevelPreset::MinLevel);
    assert_eq!(published[1].payload, BusCommandPayload::from(min));
    assert_eq!(published[0].meta.origin, Origin::Cluster);
}

#[test]
fn a_command_to_all_lamps_is_refused_while_accept_all_lamps_is_off_unless_this_node_gave_it() {
    let rig = rig(Vec::new());
    let refused = rig.execution.execute(&to_all(LightVerb::Setpoint(level(90))), stamp(true), Gate::Accepting);
    refused.count_drop(&rig.counters);
    assert_eq!(rig.read(|c| &c.commands_dropped_accept_off_total), 1);
    rig.published(0);
    assert_eq!(rig.execution.plan(&to_all(LightVerb::Setpoint(level(90))), Gate::Own).accepted, 1);
}

#[test]
fn a_command_of_the_hcl_class_runs_with_the_hcl_cluster_origin_and_every_other_class_with_the_cluster_one() {
    let rig = rig(vec![group(0, 3, true)]);
    for (source, origin) in [
        (SourceClass::Hcl, Origin::ClusterHcl),
        (SourceClass::Operator, Origin::Cluster),
        (SourceClass::HomeAssistant, Origin::Cluster),
    ] {
        let command = Command { action: on_channel(LightVerb::Setpoint(level(5))), source, hold_hcl: true, executed_on_wire: None };
        rig.execution.execute(&command.action, Stamp::of(&command), Gate::Accepting);
        assert_eq!(rig.published(1)[0].meta.origin, origin, "{source:?}");
    }
}

#[test]
fn only_a_refused_confirmation_of_its_own_command_counts_as_rejected() {
    let rig = rig(Vec::new());
    let ours = CORRELATION_TAG | 4;
    let refusal = Some((ErrorCode::Conflict, "controller_passive"));
    let refused = build_confirmation_envelope_with_product_error(ours, DeliveryStatus::ExecutionFailed, 0, 0, refusal);
    rig.execution.note_confirmation(&refused);
    rig.execution.note_confirmation(&build_confirmation_envelope(ours, DeliveryStatus::Ok, 0, 0));
    let someone_else_s = build_confirmation_envelope_with_product_error(4, DeliveryStatus::ExecutionFailed, 0, 0, refusal);
    rig.execution.note_confirmation(&someone_else_s);
    assert_eq!(rig.read(|c| &c.commands_rejected_total), 1);
}
