use dali2rust_contracts::msg::{ClusterCommandCommand, ColorMode, ColorValue, LevelTransition, LightSetpoint, PowerState};

use super::*;
use crate::runtime::outgoing::TARGET_SPACING_MS;
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

fn warm() -> LightVerb {
    let color = ColorValue { mode: ColorMode::Cct, color_temperature_kelvin: 2700, x: 0, y: 0, r: 0, g: 0, b: 0, w: 0, a: 0, f: 0 };
    LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: None, color: Some(color) })
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
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS - 1), Vec::<Action>::new());
    assert_eq!(line.deadline(), Some(START_MS + RELATIVE_HOLD_MS));
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS), [light(3, up())]);
}

#[test]
fn a_neighbour_frame_with_the_same_target_and_verb_cancels_the_held_one() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    assert_eq!(line.cancel(&light(3, up()), START_MS + 5), (1, false));
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS), Vec::<Action>::new());
}

#[test]
fn another_target_or_another_verb_cancels_nothing() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    let down = LightVerb::Transition(LevelTransition::Down);
    assert_eq!(line.cancel(&light(4, up()), START_MS + 1).0, 0);
    assert_eq!(line.cancel(&light(3, down), START_MS + 2).0, 0);
    assert_eq!(line.cancel(&light(3, level(9)), START_MS + 3).0, 0);
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS), [light(3, up())]);
}

#[test]
fn one_neighbour_up_cancels_one_of_two_held_ups_and_the_other_leaves() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, up())), true, START_MS);
    line.hold(foreign(light(3, up())), true, START_MS + 30);
    assert_eq!(line.cancel(&light(3, up()), START_MS + 31), (1, false));
    assert_eq!(drain(&mut line, START_MS + 30 + RELATIVE_HOLD_MS), [light(3, up())]);
}

#[test]
fn two_neighbour_ups_cancel_two_held_ups_and_never_a_third() {
    let mut line = HoldLine::new();
    for at in [0, 25, 50] {
        line.hold(foreign(light(3, up())), true, START_MS + at);
    }
    line.cancel(&light(3, up()), START_MS + 51);
    line.cancel(&light(3, up()), START_MS + 52);
    assert_eq!(drain(&mut line, START_MS + 50 + RELATIVE_HOLD_MS), [light(3, up())]);
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
    let late = START_MS + ECHO_MS;
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
    assert_eq!(drain(&mut line, START_MS + 30 + ABSOLUTE_HOLD_MS), Vec::<Action>::new());
}

#[test]
fn held_observations_leave_in_their_order_and_one_after_a_held_one_waits_behind_it() {
    let mut line = HoldLine::new();
    let step = LightVerb::Transition(LevelTransition::StepUp);
    line.hold(foreign(light(3, up())), true, START_MS);
    line.hold(foreign(light(3, step.clone())), false, START_MS + 10);
    assert_eq!(drain(&mut line, START_MS + 10), Vec::<Action>::new(), "the one behind waits for the head");
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS), [light(3, up()), light(3, step)]);
}

#[test]
fn a_full_line_pushes_its_head_out_rather_than_lose_an_observation() {
    let mut line = HoldLine::new();
    let actions: Vec<Action> = (0..=HOLD_DEPTH).map(|index| light(u8::try_from(index).unwrap_or(0), up())).collect();
    let outcomes: Vec<Held> =
        actions.iter().map(|action| line.hold(foreign(action.clone()), true, START_MS)).collect();
    let last = outcomes.last().cloned();
    assert_eq!(last, Some(Held::Queued { held: true, pushed_out: actions.first().cloned().map(foreign) }));
    assert_eq!(drain(&mut line, START_MS + RELATIVE_HOLD_MS).len(), HOLD_DEPTH);
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
    assert_eq!(drain(&mut line, near_wrap.wrapping_add(RELATIVE_HOLD_MS - 1)), Vec::<Action>::new());
    assert_eq!(drain(&mut line, near_wrap.wrapping_add(RELATIVE_HOLD_MS)), [light(3, up())]);
}

#[test]
fn a_neighbour_setpoint_of_another_value_cancels_nothing_and_the_foreign_value_leaves() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, level(200))), true, START_MS);
    assert_eq!(line.cancel(&light(3, level(100)), START_MS + 5), (0, false));
    assert_eq!(drain(&mut line, START_MS + ABSOLUTE_HOLD_MS), [light(3, level(200))]);
}

#[test]
fn an_early_neighbour_setpoint_of_another_value_does_not_swallow_the_foreign_one() {
    let mut line = HoldLine::new();
    line.cancel(&light(3, level(100)), START_MS);
    assert_eq!(line.hold(foreign(light(3, level(200))), true, START_MS + 5), Held::Queued { held: true, pushed_out: None });
}

#[test]
fn a_neighbour_setpoint_supersedes_every_earlier_setpoint_held_for_its_target() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, level(150))), true, START_MS);
    line.hold(foreign(light(4, level(150))), true, START_MS + 10);
    line.hold(foreign(light(3, level(200))), true, START_MS + 20);
    assert_eq!(line.cancel(&light(3, level(200)), START_MS + 25), (2, false));
    assert_eq!(drain(&mut line, START_MS + 20 + ABSOLUTE_HOLD_MS), [light(4, level(150))]);
}

#[test]
fn an_early_neighbour_setpoint_supersedes_the_earlier_setpoints_when_its_own_frame_is_heard() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, level(150))), true, START_MS);
    assert_eq!(line.cancel(&light(3, level(200)), START_MS + 20), (0, false));
    assert_eq!(line.hold(foreign(light(3, level(200))), true, START_MS + 30), Held::Cancelled);
    assert_eq!(drain(&mut line, START_MS + ABSOLUTE_HOLD_MS), Vec::<Action>::new());
}

#[test]
fn a_power_on_without_a_level_is_cancelled_only_by_a_command_that_turns_the_lamp_on() {
    let on = LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: None, color: None });
    let off = LightVerb::Setpoint(LightSetpoint { power: PowerState::Off, level: None, color: None });
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, on)), true, START_MS);
    assert_eq!(line.cancel(&light(3, off), START_MS + 1).0, 0);
    assert_eq!(line.cancel(&light(3, level(254)), START_MS + 2).0, 1);
}

const WIRE_GAP_MS: u32 = 25;
const SNIFF_POLL_MS: u32 = 10;
const NEIGHBOUR_LINE_MS: u32 = RELATIVE_SPACING_MS;
const LATENCY_MS: u32 = 20;

#[derive(Debug)]
enum Event {
    Heard(Action),
    Neighbour(Action),
}

fn replay(mut events: Vec<(u32, Event)>) -> Vec<(u32, Action)> {
    events.sort_by_key(|(at, _)| *at);
    let mut line = HoldLine::new();
    let mut released = Vec::new();
    for (at, event) in events {
        let now = START_MS + at;
        released.extend(drain(&mut line, now).into_iter().map(|action| (at, action)));
        match event {
            Event::Heard(action) => {
                line.hold(foreign(action), true, now);
            }
            Event::Neighbour(action) => {
                line.cancel(&action, now);
            }
        }
    }
    released.extend(drain(&mut line, u32::MAX / 2).into_iter().map(|action| (u32::MAX, action)));
    released
}

#[test]
fn a_burst_from_a_neighbour_whose_line_is_four_deep_is_cancelled_one_for_one() {
    let ups = 5;
    let mut events = Vec::new();
    for step in 0..ups {
        events.push((step * WIRE_GAP_MS + SNIFF_POLL_MS, Event::Heard(light(3, up()))));
        events.push((step * NEIGHBOUR_LINE_MS + LATENCY_MS, Event::Neighbour(light(3, up()))));
    }
    assert_eq!(replay(events), Vec::<(u32, Action)>::new());
}

#[test]
fn a_slider_on_a_neighbour_leaves_the_other_wires_at_the_wire_s_last_value() {
    let wire: Vec<(u32, u8)> = (0..10u8).map(|step| (u32::from(step) * 30, 10 * (step + 1))).collect();
    let last_on_wire = |until: u32| wire.iter().take_while(|(at, _)| *at <= until).last().map(|(_, value)| *value);
    let neighbour: Vec<(u32, u8)> =
        (0..=3u32).filter_map(|slot| last_on_wire(slot * TARGET_SPACING_MS).map(|value| (slot * TARGET_SPACING_MS, value))).collect();
    let mut events: Vec<(u32, Event)> =
        wire.iter().map(|(at, value)| (at + SNIFF_POLL_MS, Event::Heard(light(3, level(*value))))).collect();
    events.extend(neighbour.iter().map(|(at, value)| (at + LATENCY_MS, Event::Neighbour(light(3, level(*value))))));
    let mut arrivals: Vec<(u32, Action)> = replay(events);
    arrivals.extend(neighbour.iter().map(|(at, value)| (at + LATENCY_MS, light(3, level(*value)))));
    arrivals.sort_by_key(|(at, _)| *at);
    assert_eq!(arrivals.last().map(|(_, action)| action.clone()), Some(light(3, level(100))));
}

#[test]
fn a_neighbour_colour_without_a_level_leaves_an_earlier_foreign_level_to_leave() {
    let mut line = HoldLine::new();
    line.hold(foreign(light(3, level(50))), true, START_MS);
    line.hold(foreign(light(3, warm())), true, START_MS + 20);
    assert_eq!(line.cancel(&light(3, warm()), START_MS + 25), (1, false));
    assert_eq!(drain(&mut line, START_MS + 20 + ABSOLUTE_HOLD_MS), [light(3, level(50))]);
}

#[test]
fn a_neighbour_level_supersedes_an_earlier_foreign_step_on_its_target_but_not_a_scene() {
    let mut line = HoldLine::new();
    let recall = Action::RecallScene { scene: channel(9), scope: RecallScope::Channel(channel(3)) };
    line.hold(foreign(recall.clone()), true, START_MS);
    line.hold(foreign(light(3, up())), true, START_MS + 10);
    line.hold(foreign(light(3, level(200))), true, START_MS + 20);
    assert_eq!(line.cancel(&light(3, level(200)), START_MS + 25), (2, false));
    assert_eq!(drain(&mut line, START_MS + 10 + RELATIVE_HOLD_MS), [recall]);
}

#[test]
fn an_echo_is_forgotten_once_another_command_on_its_target_is_heard() {
    let mut line = HoldLine::new();
    let recall = Action::RecallScene { scene: channel(9), scope: RecallScope::Channel(channel(3)) };
    line.cancel(&light(3, level(50)), START_MS);
    assert_eq!(line.hold(foreign(light(3, level(50))), true, START_MS + 10), Held::Cancelled);
    line.hold(foreign(recall.clone()), true, START_MS + 20);
    assert_ne!(line.hold(foreign(light(3, level(50))), true, START_MS + 30), Held::Cancelled);
    assert_eq!(drain(&mut line, START_MS + 30 + ABSOLUTE_HOLD_MS), [recall, light(3, level(50))]);
}

