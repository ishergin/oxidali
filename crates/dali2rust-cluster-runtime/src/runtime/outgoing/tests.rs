use dali2rust_contracts::msg::{ClusterCommandCommand, ColorMode, ColorValue, LevelTransition, LightSetpoint, PowerState};
use dali2rust_contracts::CORRELATION_NONE;

use super::*;
use crate::frame::{BirthMark, LightVerb, RecordRef, SourceClass};

const START_MS: u32 = 50_000;
const MARK: BirthMark = BirthMark { generation: 4, nonce: 9 };

fn channel(number: u8) -> RecordRef {
    RecordRef { number, mark: MARK }
}

fn setpoint(number: u8, level: u8) -> ClusterCommandCommand {
    let setpoint = LightSetpoint { power: PowerState::On, level: Some(level), color: None };
    let action = Action::Light { target: Target::Channel(channel(number)), verb: LightVerb::Setpoint(setpoint) };
    ClusterCommandCommand { action, source: SourceClass::Operator, hold_hcl: true }
}

fn queued(correlation_id: u64, request: ClusterCommandCommand) -> Queued {
    Queued { correlation_id, request, forwarded: false }
}

fn repeat_of(request: &ClusterCommandCommand, seq: u32) -> Repeat {
    let header = Header { cluster_tag: 1, sender_mac: [2, 0, 0, 0, 0, 1], boot_epoch: 3, node_no: 0, seq };
    let command = Command { action: request.action.clone(), source: request.source, hold_hcl: true, executed_on_wire: None };
    Repeat { header, command }
}

fn send(outgoing: &mut Outgoing, correlation_id: u64, request: ClusterCommandCommand, now_ms: u32) {
    let key = TargetKey::of(&request.action);
    let repeat = repeat_of(&request, u32::try_from(correlation_id).unwrap_or(0));
    let admitted = outgoing.admit(queued(correlation_id, request.clone()), now_ms);
    assert_eq!(admitted, Admission::Now(queued(correlation_id, request)));
    outgoing.sent(key, repeat, now_ms);
}

#[test]
fn the_second_copy_comes_due_twenty_milliseconds_after_the_first() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert_eq!(outgoing.deadline(), Some(START_MS + COPY_GAP_MS));
    assert_eq!(outgoing.take_due(START_MS + COPY_GAP_MS - 1), None);
    let due = outgoing.take_due(START_MS + COPY_GAP_MS);
    assert_eq!(due, Some(Due::Repeat(repeat_of(&setpoint(3, 100), 1))));
    assert_eq!(outgoing.take_due(START_MS + COPY_GAP_MS), None, "the copy went once");
}

#[test]
fn a_setpoint_inside_the_spacing_waits_and_a_later_one_supersedes_it() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    let inside = START_MS + COPY_GAP_MS;
    assert_eq!(outgoing.admit(queued(2, setpoint(3, 150)), inside), Admission::Later { superseded: None });
    let superseded = Some(queued(2, setpoint(3, 150)));
    assert_eq!(outgoing.admit(queued(3, setpoint(3, 200)), inside + 1), Admission::Later { superseded });
    assert!(matches!(outgoing.take_due(inside), Some(Due::Repeat(_))));
    assert_eq!(outgoing.take_due(START_MS + TARGET_SPACING_MS - 1), None);
    let last = outgoing.take_due(START_MS + TARGET_SPACING_MS);
    assert_eq!(last, Some(Due::Waiting(queued(3, setpoint(3, 200)))), "the last setpoint is sent, never dropped");
}

#[test]
fn another_channel_waits_for_the_first_s_second_copy_and_not_for_its_spacing() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    let other = queued(2, setpoint(4, 100));
    assert_eq!(outgoing.admit(other.clone(), START_MS + 1), Admission::AfterCopy);
    assert_eq!(outgoing.deadline(), Some(START_MS + COPY_GAP_MS), "only the copy is waited for");
    assert_eq!(outgoing.take_due(START_MS + COPY_GAP_MS - 1), None);
    let copy = outgoing.take_due(START_MS + COPY_GAP_MS);
    assert_eq!(copy, Some(Due::Repeat(repeat_of(&setpoint(3, 100), 1))));
    assert_eq!(outgoing.take_due(START_MS + COPY_GAP_MS), Some(Due::Waiting(other)));
}

#[test]
fn a_waiting_command_that_came_due_still_lets_the_second_copy_before_it_go_first() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert!(matches!(outgoing.take_due(START_MS + COPY_GAP_MS), Some(Due::Repeat(_))));
    assert_eq!(outgoing.admit(queued(2, setpoint(3, 150)), START_MS + COPY_GAP_MS), Admission::Later { superseded: None });
    let other_at = START_MS + TARGET_SPACING_MS - COPY_GAP_MS / 2;
    send(&mut outgoing, 3, setpoint(4, 100), other_at);
    assert_eq!(outgoing.take_due(START_MS + TARGET_SPACING_MS), None, "the other channel's copy is not out yet");
    assert_eq!(outgoing.deadline(), Some(other_at + COPY_GAP_MS));
    assert!(matches!(outgoing.take_due(other_at + COPY_GAP_MS), Some(Due::Repeat(_))));
    let waiting = outgoing.take_due(other_at + COPY_GAP_MS);
    assert_eq!(waiting, Some(Due::Waiting(queued(2, setpoint(3, 150)))));
}

#[test]
fn a_target_is_free_again_once_its_spacing_has_passed() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert!(outgoing.take_due(START_MS + COPY_GAP_MS).is_some());
    let later = queued(2, setpoint(3, 120));
    assert_eq!(outgoing.admit(later.clone(), START_MS + TARGET_SPACING_MS), Admission::Now(later));
}

#[test]
fn the_waiting_command_of_a_target_survives_the_copy_of_the_one_before_it() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert_eq!(outgoing.admit(queued(2, setpoint(3, 150)), START_MS + 1), Admission::Later { superseded: None });
    assert!(matches!(outgoing.take_due(START_MS + COPY_GAP_MS), Some(Due::Repeat(_))));
    outgoing.prune(START_MS + COPY_GAP_MS);
    let waiting = outgoing.take_due(START_MS + TARGET_SPACING_MS);
    assert_eq!(waiting, Some(Due::Waiting(queued(2, setpoint(3, 150)))));
}

#[test]
fn a_full_table_refuses_a_new_target_rather_than_dropping_one_it_holds() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 0, setpoint(0, 1), START_MS);
    for number in 1..OUTGOING_TARGETS {
        let number = u8::try_from(number).unwrap_or(u8::MAX);
        assert_eq!(outgoing.admit(queued(u64::from(number), setpoint(number, 1)), START_MS), Admission::AfterCopy);
    }
    let one_more = queued(99, setpoint(40, 1));
    assert_eq!(outgoing.admit(one_more.clone(), START_MS + 1), Admission::Full(one_more));
}

#[test]
fn giving_up_hands_back_every_waiting_command_and_empties_the_table() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert_eq!(outgoing.admit(queued(2, setpoint(4, 100)), START_MS), Admission::AfterCopy);
    assert!(matches!(outgoing.admit(queued(3, setpoint(3, 1)), START_MS + 1), Admission::Later { .. }));
    let abandoned: Vec<_> = outgoing.abandon().collect();
    assert_eq!(abandoned, vec![queued(3, setpoint(3, 1)), queued(2, setpoint(4, 100))]);
    assert_eq!(outgoing.deadline(), None);
}

#[test]
fn the_earliest_deadline_is_found_across_a_clock_wrap() {
    let mut outgoing = Outgoing::new();
    let start = u32::MAX - TARGET_SPACING_MS - 10;
    let second = start.wrapping_add(COPY_GAP_MS);
    send(&mut outgoing, 1, setpoint(3, 100), start);
    assert!(outgoing.take_due(second).is_some());
    send(&mut outgoing, 2, setpoint(4, 100), second);
    assert!(outgoing.take_due(second.wrapping_add(COPY_GAP_MS)).is_some());
    let after = second.wrapping_add(COPY_GAP_MS + 1);
    assert!(matches!(outgoing.admit(queued(4, setpoint(4, 1)), after), Admission::Later { .. }));
    assert!(matches!(outgoing.admit(queued(3, setpoint(3, 1)), after), Admission::Later { .. }));
    assert!(second.wrapping_add(TARGET_SPACING_MS) < start, "the later deadline wrapped");
    assert_eq!(outgoing.deadline(), Some(start.wrapping_add(TARGET_SPACING_MS)));
}

#[test]
fn a_recall_on_a_channel_shares_the_channel_s_spacing() {
    let scene = RecordRef { number: 7, mark: MARK };
    let recall = Action::RecallScene { scene, scope: RecallScope::Channel(channel(3)) };
    assert_eq!(TargetKey::of(&recall), TargetKey::of(&setpoint(3, 1).action));
    let on_node = Action::RecallScene { scene, scope: RecallScope::Node };
    assert_eq!(TargetKey::of(&on_node), TargetKey::Node);
}

#[test]
fn the_placed_table_starts_empty() {
    let placed = placed_outgoing();
    assert_eq!(placed.deadline(), None);
    assert!(placed.slots.iter().all(|slot| slot.key.is_none() && slot.line.is_empty()));
}

#[test]
fn a_forwarded_setpoint_and_a_command_given_here_keep_their_own_flag_through_the_wait() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    let inside = START_MS + COPY_GAP_MS;
    let forwarded = Queued { correlation_id: CORRELATION_NONE, request: setpoint(3, 150), forwarded: true };
    assert_eq!(outgoing.admit(forwarded.clone(), inside), Admission::Later { superseded: None });
    let given_here = queued(4, setpoint(3, 200));
    assert_eq!(outgoing.admit(given_here.clone(), inside + 1), Admission::Later { superseded: Some(forwarded) });
    assert!(matches!(outgoing.take_due(inside), Some(Due::Repeat(_))));
    let last = outgoing.take_due(START_MS + TARGET_SPACING_MS);
    assert_eq!(last, Some(Due::Waiting(given_here)), "the command given here still runs this node's own part");
}

fn on_channel(number: u8, setpoint: LightSetpoint, source: SourceClass) -> ClusterCommandCommand {
    let action = Action::Light { target: Target::Channel(channel(number)), verb: LightVerb::Setpoint(setpoint) };
    ClusterCommandCommand { action, source, hold_hcl: true }
}

fn warm_white() -> Option<ColorValue> {
    Some(ColorValue { mode: ColorMode::Cct, color_temperature_kelvin: 2700, ..ColorValue::default() })
}

fn forwarded(request: ClusterCommandCommand) -> Queued {
    Queued { correlation_id: CORRELATION_NONE, request, forwarded: true }
}

fn waiting_after(earlier: Queued, later: Queued) -> Option<Due> {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert_eq!(outgoing.admit(earlier, START_MS + 1), Admission::Later { superseded: None });
    assert!(matches!(outgoing.admit(later, START_MS + 2), Admission::Later { superseded: Some(_) }));
    assert!(matches!(outgoing.take_due(START_MS + COPY_GAP_MS), Some(Due::Repeat(_))));
    outgoing.take_due(START_MS + TARGET_SPACING_MS)
}

#[test]
fn a_forwarded_setpoint_inside_the_spacing_keeps_the_fields_of_the_waiting_one_of_its_class() {
    let colour_only = LightSetpoint { power: PowerState::Unknown, level: None, color: warm_white() };
    let on = LightSetpoint { power: PowerState::On, level: None, color: None };
    let earlier = forwarded(on_channel(3, colour_only, SourceClass::Hcl));
    let due = waiting_after(earlier, forwarded(on_channel(3, on, SourceClass::Hcl)));
    let both = LightSetpoint { power: PowerState::On, level: None, color: warm_white() };
    let folded = forwarded(on_channel(3, both, SourceClass::Hcl));
    assert_eq!(due, Some(Due::Waiting(folded)), "the colour of the waiting tick is not lost");
}

#[test]
fn a_setpoint_of_another_class_hold_or_requester_replaces_the_waiting_one_whole() {
    let off = LightSetpoint { power: PowerState::Off, level: None, color: None };
    let colour_only = LightSetpoint { power: PowerState::Unknown, level: None, color: warm_white() };
    let tick = forwarded(on_channel(3, colour_only.clone(), SourceClass::Hcl));
    let released = ClusterCommandCommand { hold_hcl: false, ..on_channel(3, off.clone(), SourceClass::Hcl) };
    let pairs = [
        (forwarded(on_channel(3, off.clone(), SourceClass::Operator)), tick.clone()),
        (forwarded(released), tick.clone()),
        (queued(2, on_channel(3, off.clone(), SourceClass::Hcl)), tick),
        (forwarded(on_channel(3, off, SourceClass::Hcl)), queued(3, on_channel(3, colour_only, SourceClass::Hcl))),
    ];
    for (earlier, later) in pairs {
        let due = waiting_after(earlier.clone(), later.clone());
        assert_eq!(due, Some(Due::Waiting(later)), "not merged over {earlier:?}");
    }
}

#[test]
fn a_recall_inside_the_spacing_replaces_a_waiting_setpoint_whole() {
    let mut outgoing = Outgoing::new();
    send(&mut outgoing, 1, setpoint(3, 100), START_MS);
    assert_eq!(outgoing.admit(queued(2, setpoint(3, 150)), START_MS + 1), Admission::Later { superseded: None });
    let action = Action::Light { target: Target::Channel(channel(3)), verb: LightVerb::Transition(LevelTransition::RecallMaxLevel) };
    let recall = queued(3, ClusterCommandCommand { action, source: SourceClass::Operator, hold_hcl: true });
    assert!(matches!(outgoing.admit(recall.clone(), START_MS + 2), Admission::Later { superseded: Some(_) }));
    assert!(matches!(outgoing.take_due(START_MS + COPY_GAP_MS), Some(Due::Repeat(_))));
    assert_eq!(outgoing.take_due(START_MS + TARGET_SPACING_MS), Some(Due::Waiting(recall)));
}

fn relative(number: u8, transition: LevelTransition) -> ClusterCommandCommand {
    let action = Action::Light { target: Target::Channel(channel(number)), verb: LightVerb::Transition(transition) };
    ClusterCommandCommand { action, source: SourceClass::Operator, hold_hcl: true }
}

fn stop_fade(number: u8) -> ClusterCommandCommand {
    let action = Action::Light { target: Target::Channel(channel(number)), verb: LightVerb::StopFade };
    ClusterCommandCommand { action, source: SourceClass::Operator, hold_hcl: true }
}

fn drive(arrivals: &[(u32, Queued)], until_ms: u32) -> (Vec<(u32, u64)>, Vec<Admission>) {
    let mut outgoing = Outgoing::new();
    let (mut sent, mut refused) = (Vec::new(), Vec::new());
    let send_now = |outgoing: &mut Outgoing, queued: Queued, at_ms: u32, sent: &mut Vec<(u32, u64)>| {
        sent.push((at_ms, queued.correlation_id));
        let key = TargetKey::of(&queued.request.action);
        outgoing.sent(key, repeat_of(&queued.request, u32::try_from(queued.correlation_id).unwrap_or(0)), at_ms);
    };
    for now_ms in START_MS..=until_ms {
        for (_, queued) in arrivals.iter().filter(|(at_ms, _)| *at_ms == now_ms) {
            match outgoing.admit(queued.clone(), now_ms) {
                Admission::Now(queued) => send_now(&mut outgoing, queued, now_ms, &mut sent),
                admitted @ (Admission::Dropped(_) | Admission::Full(_) | Admission::Later { superseded: Some(_) }) => {
                    refused.push(admitted);
                }
                Admission::AfterCopy | Admission::Later { superseded: None } => {}
            }
        }
        while let Some(due) = outgoing.take_due(now_ms) {
            if let Due::Waiting(queued) = due {
                send_now(&mut outgoing, queued, now_ms, &mut sent);
            }
        }
    }
    (sent, refused)
}

#[test]
fn two_steps_inside_the_spacing_go_out_as_two_in_their_order() {
    let arrivals = [
        (START_MS, queued(1, relative(3, LevelTransition::StepUp))),
        (START_MS + 1, queued(2, relative(3, LevelTransition::StepUp))),
        (START_MS + 2, queued(3, relative(3, LevelTransition::StepUp))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 200);
    let spaced = START_MS + RELATIVE_SPACING_MS;
    assert_eq!(sent, vec![(START_MS, 1), (spaced, 2), (spaced + RELATIVE_SPACING_MS, 3)]);
    assert!(refused.is_empty(), "no step superseded another: {refused:?}");
}

#[test]
fn a_relative_verb_never_overtakes_or_replaces_the_setpoint_before_it() {
    let arrivals = [
        (START_MS, queued(1, setpoint(3, 100))),
        (START_MS + 5, queued(2, setpoint(3, 150))),
        (START_MS + 6, queued(3, relative(3, LevelTransition::Up))),
        (START_MS + 7, queued(4, stop_fade(3))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 300);
    let after_setpoint = START_MS + 2 * TARGET_SPACING_MS;
    let expected = vec![(START_MS, 1), (START_MS + TARGET_SPACING_MS, 2), (after_setpoint - 50, 3), (after_setpoint, 4)];
    assert_eq!(sent, expected);
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn a_setpoint_after_steps_waits_behind_them_and_still_lands_last() {
    let arrivals = [
        (START_MS, queued(1, relative(3, LevelTransition::StepDown))),
        (START_MS + 1, queued(2, relative(3, LevelTransition::StepDown))),
        (START_MS + 2, queued(3, setpoint(3, 10))),
        (START_MS + 3, queued(4, setpoint(3, 20))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 300);
    let ids: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(ids, vec![1, 2, 4], "the later setpoint replaced only the setpoint waiting at the tail");
    assert_eq!(refused, vec![Admission::Later { superseded: Some(queued(3, setpoint(3, 10))) }]);
    assert_eq!(sent.last(), Some(&(START_MS + RELATIVE_SPACING_MS + TARGET_SPACING_MS, 4)));
}

#[test]
fn a_fifth_waiting_relative_verb_is_dropped_and_a_setpoint_behind_them_is_not() {
    let mut arrivals: Vec<(u32, Queued)> =
        (1..=6).map(|id| (START_MS + u32::try_from(id).unwrap_or(0), queued(id, relative(3, LevelTransition::Up)))).collect();
    arrivals.push((START_MS + 10, queued(9, setpoint(3, 200))));
    let (sent, refused) = drive(&arrivals, START_MS + 600);
    let ids: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5, 9], "four wait behind the one on the wire, and the setpoint is the last word");
    assert_eq!(refused, vec![Admission::Dropped(queued(6, relative(3, LevelTransition::Up)))]);
}

#[test]
fn a_key_held_at_the_fastest_repeat_never_waits() {
    let repeat_ms = 100;
    let arrivals: Vec<(u32, Queued)> =
        (0..10).map(|n| (START_MS + n * repeat_ms, queued(u64::from(n), relative(3, LevelTransition::Up)))).collect();
    let (sent, refused) = drive(&arrivals, START_MS + 10 * repeat_ms);
    let expected: Vec<(u32, u64)> = arrivals.iter().map(|(at_ms, queued)| (*at_ms, queued.correlation_id)).collect();
    assert_eq!(sent, expected, "each repeat goes out when it is given");
    assert!(refused.is_empty());
    assert!(RELATIVE_SPACING_MS < repeat_ms && RELATIVE_SPACING_MS > COPY_GAP_MS);
}

#[test]
fn copies_of_two_targets_never_cross_while_relative_verbs_wait() {
    let arrivals = [
        (START_MS, queued(1, relative(3, LevelTransition::StepUp))),
        (START_MS + 1, queued(2, relative(4, LevelTransition::StepUp))),
        (START_MS + 2, queued(3, relative(3, LevelTransition::StepUp))),
    ];
    let (sent, _) = drive(&arrivals, START_MS + 200);
    assert_eq!(sent, vec![(START_MS, 1), (START_MS + COPY_GAP_MS, 2), (START_MS + RELATIVE_SPACING_MS, 3)]);
}

fn on_all(verb: LightVerb) -> ClusterCommandCommand {
    ClusterCommandCommand { action: Action::Light { target: Target::All, verb }, source: SourceClass::Operator, hold_hcl: true }
}

fn level(level: u8) -> LightVerb {
    LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: Some(level), color: None })
}

#[test]
fn a_setpoint_to_all_lamps_never_leaves_before_a_step_on_a_channel_given_before_it() {
    let arrivals = [
        (START_MS, queued(1, relative(3, LevelTransition::StepUp))),
        (START_MS + 10, queued(2, relative(3, LevelTransition::StepUp))),
        (START_MS + 15, queued(3, on_all(level(200)))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 300);
    let order: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(order, vec![1, 2, 3], "the setpoint to all lamps is the last word: {sent:?}");
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn a_step_to_all_lamps_never_overtakes_a_setpoint_on_a_channel_given_before_it() {
    let arrivals = [
        (START_MS, queued(1, setpoint(3, 100))),
        (START_MS + 10, queued(2, setpoint(3, 150))),
        (START_MS + 30, queued(3, on_all(LightVerb::Transition(LevelTransition::StepUp)))),
    ];
    let (sent, _) = drive(&arrivals, START_MS + 300);
    let order: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(order, vec![1, 2, 3], "the step lands on the setpoint before it: {sent:?}");
}

#[test]
fn a_channel_waits_for_nothing_on_another_channel_and_a_later_setpoint_never_replaces_across_all_lamps() {
    let arrivals = [
        (START_MS, queued(1, setpoint(3, 100))),
        (START_MS + 10, queued(2, setpoint(3, 150))),
        (START_MS + 30, queued(3, setpoint(4, 90))),
        (START_MS + 35, queued(4, on_all(level(10)))),
        (START_MS + 40, queued(5, setpoint(3, 200))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 600);
    assert_eq!(sent.get(1), Some(&(START_MS + 30, 3)), "another channel left at once: {sent:?}");
    let order: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(order, vec![1, 3, 2, 4, 5], "each lands in the order it was given: {sent:?}");
    assert!(refused.is_empty(), "nothing given before all lamps is replaced after it: {refused:?}");
}

fn forwarded_as(correlation_id: u64, request: ClusterCommandCommand) -> Queued {
    Queued { correlation_id, request, forwarded: true }
}

#[test]
fn a_forwarded_setpoint_is_never_folded_over_one_waiting_before_a_step_to_all_lamps() {
    let on_warm = LightSetpoint { power: PowerState::On, level: Some(100), color: warm_white() };
    let colour_only = LightSetpoint { power: PowerState::Unknown, level: None, color: warm_white() };
    let arrivals = [
        (START_MS, queued(1, setpoint(3, 50))),
        (START_MS + 5, forwarded_as(2, on_channel(3, on_warm, SourceClass::Hcl))),
        (START_MS + 10, queued(3, on_all(LightVerb::Transition(LevelTransition::StepUp)))),
        (START_MS + 15, forwarded_as(4, on_channel(3, colour_only, SourceClass::Hcl))),
    ];
    let (sent, refused) = drive(&arrivals, START_MS + 600);
    let order: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert_eq!(order, vec![1, 2, 3, 4], "the step lands on the level given before it: {sent:?}");
    assert!(refused.is_empty(), "the level is not carried past the step: {refused:?}");
}

#[test]
fn a_full_line_crossed_by_all_lamps_still_takes_the_last_setpoint_as_its_last_word() {
    let arrivals: Vec<(u32, Queued)> = (0..24_u8)
        .map(|n| {
            let request = if n % 2 == 0 { setpoint(3, n) } else { on_all(level(n)) };
            (START_MS + u32::from(n), queued(u64::from(n) + 1, request))
        })
        .collect();
    let (sent, refused) = drive(&arrivals, START_MS + 4000);
    let order: Vec<u64> = sent.iter().map(|(_, id)| *id).collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "never out of order: {order:?}");
    assert_eq!(order.last(), Some(&24), "the last command given is the last word: {order:?}");
    assert!(order.contains(&23), "the last setpoint of the channel still lands: {order:?}");
    assert!(refused.iter().all(|admitted| matches!(admitted, Admission::Later { superseded: Some(_) })), "{refused:?}");
}
