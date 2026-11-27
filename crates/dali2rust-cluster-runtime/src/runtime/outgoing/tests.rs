use dali2rust_contracts::msg::{ClusterCommandCommand, LightSetpoint, PowerState};

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
    Queued { correlation_id, request }
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
    assert!(placed.slots.iter().all(Option::is_none));
}
