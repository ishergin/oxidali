use dali2rust_contracts::msg::{ClusterCommandCommand, LevelTransition, LightSetpoint, PowerState};

use super::*;
use crate::frame::{BirthMark, RecallScope, RecordRef, SourceClass, Target};

const START_MS: u32 = 70_000;
const MARK: BirthMark = BirthMark { generation: 2, nonce: 7 };

fn channel(number: u8) -> RecordRef {
    RecordRef { number, mark: MARK }
}

fn light(number: u8, verb: LightVerb) -> Action {
    Action::Light { target: Target::Channel(channel(number)), verb }
}

fn level(value: u8) -> LightVerb {
    LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: Some(value), color: None })
}

fn up() -> LightVerb {
    LightVerb::Transition(LevelTransition::Up)
}

fn foreign(action: Action) -> ClusterCommandCommand {
    ClusterCommandCommand { action, source: SourceClass::ForeignMaster, hold_hcl: true }
}

fn drain(line: &mut HoldLine, now_ms: u32) -> Vec<Action> {
    core::iter::from_fn(|| line.take_due(now_ms)).map(|request| request.action).collect()
}

#[test]
fn alone_on_the_wire_an_observation_leaves_at_once() {
    let mut line = HoldLine::new();
    let held = line.hold(foreign(light(3, up())), false, START_MS);
    assert_eq!(held, Held::Queued { held: false, pushed_out: None });
    assert_eq!(drain(&mut line, START_MS), [light(3, up())]);
}

#[test]
fn with_a_neighbour_it_waits_the_hold_and_then_leaves() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS - 1), Vec::<Action>::new());
    assert_eq!(line.deadline(), Some(START_MS + FOREIGN_HOLD_MS));
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS), [light(3, up())]);
}

#[test]
fn a_neighbour_frame_with_the_same_target_and_verb_cancels_the_held_one() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    assert_eq!(line.cancel(&light(3, up()), START_MS + 5), (1, false));
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS), Vec::<Action>::new());
}

#[test]
fn another_target_or_another_verb_cancels_nothing() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    let down = LightVerb::Transition(LevelTransition::Down);
    assert_eq!(line.cancel(&light(4, up()), START_MS + 1).0, 0);
    assert_eq!(line.cancel(&light(3, down), START_MS + 2).0, 0);
    assert_eq!(line.cancel(&light(3, level(9)), START_MS + 3).0, 0);
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS), [light(3, up())]);
}

#[test]
fn one_neighbour_up_cancels_one_of_two_held_ups_and_the_other_leaves() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    line.hold(foreign(light(3, up())), true, START_MS + 30);
    assert_eq!(line.cancel(&light(3, up()), START_MS + 31), (1, false));
    assert_eq!(drain(&mut line, START_MS + 30 + FOREIGN_HOLD_MS), [light(3, up())]);
}

#[test]
fn two_neighbour_ups_cancel_two_held_ups_and_never_a_third() {
    let mut line = HoldLine::new();
    for at in [0, 25, 50] {
        line.hold(foreign(light(3, up())), true, START_MS + at);
    }
    line.cancel(&light(3, up()), START_MS + 51);
    line.cancel(&light(3, up()), START_MS + 52);
    assert_eq!(drain(&mut line, START_MS + 50 + FOREIGN_HOLD_MS), [light(3, up())]);
}

#[test]
fn a_neighbour_up_heard_before_the_observation_cancels_it_once() {
    let mut line = HoldLine::new();
    assert_eq!(line.cancel(&light(3, up()), START_MS), (0, false));
    assert_eq!(line.hold(foreign(light(3, up())), true, START_MS + 8), Held::Cancelled);
    assert_eq!(line.hold(foreign(light(3, up())), true, START_MS + 30), Held::Queued { held: true, pushed_out: None });
}

#[test]
fn an_early_neighbour_frame_cancels_nothing_after_the_hold() {
    let mut line = HoldLine::new();
    line.cancel(&light(3, up()), START_MS);
    let late = START_MS + FOREIGN_HOLD_MS;
    assert_eq!(line.hold(foreign(light(3, up())), true, late), Held::Queued { held: true, pushed_out: None });
}

#[test]
fn one_neighbour_setpoint_cancels_every_absolute_frame_its_command_became_on_the_wire() {
    let mut line = HoldLine::new();
    let colour = LightVerb::Setpoint(LightSetpoint { power: PowerState::Unknown, level: None, color: None });
    line.hold(foreign(light(3, colour)), true, START_MS);
    line.hold(foreign(light(3, level(200))), true, START_MS + 20);
    assert_eq!(line.cancel(&light(3, level(200)), START_MS + 25), (2, false));
    assert_eq!(line.hold(foreign(light(3, level(200))), true, START_MS + 30), Held::Cancelled);
    assert_eq!(drain(&mut line, START_MS + 30 + FOREIGN_HOLD_MS), Vec::<Action>::new());
}

#[test]
fn held_observations_leave_in_their_order_and_one_after_a_held_one_waits_behind_it() {
    let mut line = HoldLine::new();
    let step = LightVerb::Transition(LevelTransition::StepUp);
    line.hold(foreign(light(3, up())), true, START_MS);
    line.hold(foreign(light(3, step.clone())), false, START_MS + 10);
    assert_eq!(drain(&mut line, START_MS + 10), Vec::<Action>::new(), "the one behind waits for the head");
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS), [light(3, up()), light(3, step)]);
}

#[test]
fn a_full_line_pushes_its_head_out_rather_than_lose_an_observation() {
    let mut line = HoldLine::new();
    let actions: Vec<Action> = (0..=HOLD_DEPTH).map(|index| light(u8::try_from(index).unwrap_or(0), up())).collect();
    let outcomes: Vec<Held> =
        actions.iter().map(|action| line.hold(foreign(action.clone()), true, START_MS)).collect();
    let last = outcomes.last().cloned();
    assert_eq!(last, Some(Held::Queued { held: true, pushed_out: actions.first().cloned().map(foreign) }));
    assert_eq!(drain(&mut line, START_MS + FOREIGN_HOLD_MS).len(), HOLD_DEPTH);
}

#[test]
fn a_scene_recall_is_cancelled_only_by_the_same_recall() {
    let mut line = HoldLine::new();
    let recall = |scene| Action::RecallScene { scene: channel(scene), scope: RecallScope::Node };
    line.hold(foreign(recall(5)), true, START_MS);
    assert_eq!(line.cancel(&recall(6), START_MS + 1).0, 0);
    assert_eq!(line.cancel(&light(5, level(9)), START_MS + 2).0, 0);
    assert_eq!(line.cancel(&recall(5), START_MS + 3).0, 1);
}

#[test]
fn the_hold_survives_the_wrap_of_the_clock() {
    let mut line = HoldLine::new();
    let near_wrap = u32::MAX - 10;
    line.hold(foreign(light(3, up())), true, near_wrap);
    assert_eq!(drain(&mut line, near_wrap.wrapping_add(FOREIGN_HOLD_MS - 1)), Vec::<Action>::new());
    assert_eq!(drain(&mut line, near_wrap.wrapping_add(FOREIGN_HOLD_MS)), [light(3, up())]);
}
