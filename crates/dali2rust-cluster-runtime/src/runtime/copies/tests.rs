use std::sync::Mutex;

use dali2rust_contracts::msg::{ClusterBirthMark, FixedText32};
use dali2rust_domain::registry::{ClusterPartKeys, ClusterPartView, ClusterRecordNumbers, ClusterRecordView};

use super::*;
use crate::info::{ClusterInfoReadPort, InfoHeldView};
use crate::runtime::shared::{ClusterPresenceReadPort, IgnoredView, NodeView, OwnStatus};

const OWN: u8 = 5;
const HOME: u8 = 1;
const NUMBER: u8 = 20;
const OWN_NUMBER: u8 = 83;
const KIND: ClusterRecordKind = ClusterRecordKind::Channel;
const MAC: [u8; 6] = [2, 0, 0, 0, 0, 1];
const TIMING: PresenceTiming = PresenceTiming::every_ms(100);
const NOW_MS: u32 = 100_000;
const HELD_AGE_MS: u32 = 10;
const SETTLED_FOR_MS: u32 = 1_000;

fn mark(generation: u32, nonce: u32) -> ClusterBirthMark {
    ClusterBirthMark { generation, nonce }
}

fn record(number: u8, generation: u32) -> ClusterRecordRef {
    ClusterRecordRef { number, mark: mark(generation, 7) }
}

struct World {
    home: Option<(ClusterBirthMark, u32)>,
    judged: bool,
    parts: Vec<(u8, ClusterRecordRef)>,
    copies: Mutex<Vec<(ClusterRecordRef, u32)>>,
}

fn view(kind: ClusterRecordKind, record: ClusterRecordRef, revision: u32) -> ClusterRecordView {
    let home_node = cluster_node_of(record.number);
    ClusterRecordView { kind, number: record.number, home_node, mark: record.mark, name: "Hall".into(), revision }
}

impl ClusterPresenceReadPort for World {
    fn own_status(&self) -> OwnStatus {
        unreachable!("copies read no own status")
    }

    fn heard_node(&self, node: u8) -> Option<NodeView> {
        (node == HOME).then(|| NodeView {
            node,
            mac: MAC,
            source: [0; 4],
            installation: FixedText32::new(),
            wire: node,
            wire_protection: false,
            ha_face: None,
            live: true,
            age_ms: 0,
            sequence_gaps: 0,
            split: false,
            settled_for_ms: self.judged.then_some(SETTLED_FOR_MS),
        })
    }

    fn ignored_board(&self, _index: usize) -> Option<IgnoredView> {
        None
    }
}

impl ClusterInfoReadPort for World {
    fn info_held(&self, node: u8) -> Option<InfoHeldView> {
        let (channels, scenes, channel_parts, scene_parts, skipped) = (0, 0, 0, 0, 0);
        (node == HOME)
            .then_some(InfoHeldView { node, mac: MAC, set: 1, age_ms: HELD_AGE_MS, skipped, channels, scenes, channel_parts, scene_parts })
    }

    fn info_record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let (mark, revision) = self.home.filter(|_| node == HOME && number == NUMBER)?;
        Some(view(kind, ClusterRecordRef { number, mark }, revision))
    }

    fn info_record_numbers(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        ClusterRecordNumbers::default()
    }

    fn info_part(&self, _node: u8, _kind: ClusterRecordKind, _adapter: u8, _local: u8) -> Option<ClusterPartView> {
        None
    }

    fn info_part_keys(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterPartKeys {
        ClusterPartKeys::default()
    }
}

impl ClusterPartReadPort for World {
    fn cluster_part(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        let record = self.parts.iter().find(|(at, _)| (*at, 0) == (adapter, local)).map(|(_, record)| *record)?;
        Some(ClusterPartView { kind, adapter, local, record, send: true, accept: true, revision: 1 })
    }

    fn cluster_part_keys(&self, kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        if kind == KIND {
            self.parts.iter().for_each(|(adapter, _)| keys.insert(*adapter, 0));
        }
        keys
    }
}

impl ClusterCopyReadPort for World {
    fn cluster_copy(&self, kind: ClusterRecordKind, record: ClusterRecordRef) -> Option<ClusterRecordView> {
        let copies = self.copies.lock().unwrap();
        let revision = copies.iter().find(|(held, _)| *held == record).map(|(_, revision)| *revision)?;
        Some(view(kind, record, revision))
    }

    fn cluster_copy_at(&self, kind: ClusterRecordKind, index: usize) -> Option<ClusterRecordView> {
        let copies = self.copies.lock().unwrap();
        copies.get(index).map(|(record, revision)| view(kind, *record, *revision))
    }
}

fn sent_ago(earlier_ms: u32, last_ms: u32) -> Announced {
    Announced { sets_sent: [Some(NOW_MS - earlier_ms), Some(NOW_MS - last_ms)], now_ms: NOW_MS, timing: TIMING }
}

fn answered() -> Announced {
    sent_ago(SETTLED_FOR_MS, SETTLED_FOR_MS / 2)
}

fn actions_after(world: &World, announced: Announced) -> Vec<(u8, ClusterCopyAction)> {
    let reads = CopyReads { home: HomeReads { presence: world, info: world }, parts: world, copies: world, announced };
    copy_commands(OWN, reads).into_iter().map(|command| (command.record.number, command.action)).collect()
}

fn actions(world: &World) -> Vec<(u8, ClusterCopyAction)> {
    actions_after(world, answered())
}

fn store(revision: u32) -> ClusterCopyAction {
    ClusterCopyAction::Store { revision, name: fixed_text_64("Hall") }
}

fn base() -> World {
    World { home: None, judged: true, parts: Vec::new(), copies: Mutex::new(Vec::new()) }
}

#[test]
fn a_part_s_record_held_whole_by_its_home_is_copied_once_and_again_only_at_a_higher_revision() {
    let world = World { home: Some((mark(2, 7), 3)), parts: vec![(0, record(NUMBER, 2)), (1, record(NUMBER, 2))], ..base() };
    assert_eq!(actions(&world), [(NUMBER, store(3))], "two parts of one record make one copy");
    world.copies.lock().unwrap().push((record(NUMBER, 2), 3));
    assert!(actions(&world).is_empty(), "the same revision is not written again");
    world.copies.lock().unwrap()[0].1 = 2;
    assert_eq!(actions(&world), [(NUMBER, store(3))]);
    world.copies.lock().unwrap()[0].1 = 4;
    assert!(actions(&world).is_empty(), "a lower revision on the home never lowers a copy");
}

#[test]
fn a_home_holding_another_mark_at_the_number_writes_no_copy() {
    let world = World { home: Some((mark(2, 8), 9)), parts: vec![(0, record(NUMBER, 2))], ..base() };
    assert!(actions(&world).is_empty());
}

#[test]
fn a_copy_is_dropped_only_once_its_part_is_judged_an_orphan() {
    let held = || Mutex::new(vec![(record(NUMBER, 2), 3)]);
    let unsettled = World { judged: false, parts: vec![(0, record(NUMBER, 2))], copies: held(), ..base() };
    assert!(actions(&unsettled).is_empty(), "nothing is dropped before the home's full round");
    let older = World { home: Some((mark(1, 7), 1)), parts: vec![(0, record(NUMBER, 2))], copies: held(), ..base() };
    assert!(actions(&older).is_empty(), "an older generation at the number is not an orphan verdict");
    let gone = World { parts: vec![(0, record(NUMBER, 2))], copies: held(), ..base() };
    assert_eq!(actions(&gone), [(NUMBER, ClusterCopyAction::Drop)]);
    let reissued = World { home: Some((mark(3, 1), 1)), parts: vec![(0, record(NUMBER, 2))], copies: held(), ..base() };
    assert_eq!(actions(&reissued), [(NUMBER, ClusterCopyAction::Drop)]);
}

#[test]
fn a_copy_its_home_holds_nothing_for_is_dropped_only_once_two_sets_carrying_it_reached_that_home() {
    let world = World { parts: vec![(0, record(NUMBER, 2))], copies: Mutex::new(vec![(record(NUMBER, 2), 3)]), ..base() };
    let holding_ms = SETTLED_FOR_MS + TIMING.settled_after_ms();
    let answer_ms = TIMING.info_coalesce_ms() + 2 * TIMING.pulse_period_ms();
    let drop = [(NUMBER, ClusterCopyAction::Drop)];
    assert_eq!(actions_after(&world, sent_ago(holding_ms - 1, HELD_AGE_MS + answer_ms)), drop);
    let once = Announced { sets_sent: [None, Some(NOW_MS - 500)], ..answered() };
    assert!(actions_after(&world, once).is_empty(), "one set may have been lost");
    assert!(actions_after(&world, sent_ago(holding_ms, 500)).is_empty(), "a set before this home was heard proves nothing");
    let unanswered = sent_ago(holding_ms - 1, HELD_AGE_MS + answer_ms - 1);
    assert!(actions_after(&world, unanswered).is_empty(), "the home had no time to take it back");
    let reissued = World { home: Some((mark(3, 1), 1)), ..world };
    assert_eq!(actions_after(&reissued, Announced { sets_sent: [None; 2], ..answered() }), drop, "a newer generation needs no answer");
}

#[test]
fn a_home_showing_revision_zero_writes_no_copy() {
    let world = World { home: Some((mark(2, 7), 0)), parts: vec![(0, record(NUMBER, 2))], ..base() };
    assert!(actions(&world).is_empty());
}

#[test]
fn a_part_of_this_node_s_own_block_is_never_copied() {
    let world = World { home: Some((mark(2, 7), 3)), parts: vec![(0, record(OWN_NUMBER, 2))], ..base() };
    assert!(actions(&world).is_empty());
}
