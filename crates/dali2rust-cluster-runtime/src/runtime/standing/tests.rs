use dali2rust_contracts::msg::{ClusterBirthMark, FixedText32};
use dali2rust_domain::registry::{ClusterPartKeys, ClusterPartView, ClusterRecordNumbers, ClusterRecordView};

use super::*;
use crate::info::InfoHeldView;
use crate::runtime::shared::{IgnoredView, NodeView, OwnStatus};

const HOME: u8 = 1;
const NUMBER: u8 = 20;
const MAC: [u8; 6] = [2, 0, 0, 0, 0, 1];
const KIND: ClusterRecordKind = ClusterRecordKind::Channel;

fn mark(generation: u32, nonce: u32) -> ClusterBirthMark {
    ClusterBirthMark { generation, nonce }
}

fn part_of(generation: u32, nonce: u32) -> ClusterRecordRef {
    ClusterRecordRef { number: NUMBER, mark: mark(generation, nonce) }
}

#[derive(Clone, Copy)]
struct Home {
    live: bool,
    split: bool,
    mac: [u8; 6],
    settled_for_ms: Option<u32>,
    set_age_ms: u32,
    held: Option<ClusterBirthMark>,
}

const SETTLED: Home =
    Home { live: true, split: false, mac: MAC, settled_for_ms: Some(1_000), set_age_ms: 10, held: None };

impl ClusterPresenceReadPort for Home {
    fn own_status(&self) -> OwnStatus {
        unreachable!("the verdict reads no own status")
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
            live: self.live,
            age_ms: 0,
            sequence_gaps: 0,
            split: self.split,
            settled_for_ms: self.settled_for_ms,
        })
    }

    fn ignored_board(&self, _index: usize) -> Option<IgnoredView> {
        None
    }
}

impl ClusterInfoReadPort for Home {
    fn info_held(&self, node: u8) -> Option<InfoHeldView> {
        let (channels, scenes, channel_parts, scene_parts) = (0, 0, 0, 0);
        let (set, skipped, mac, age_ms) = (1, 0, self.mac, self.set_age_ms);
        (node == HOME).then_some(InfoHeldView { node, mac, set, age_ms, skipped, channels, scenes, channel_parts, scene_parts })
    }

    fn info_record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let mark = self.held.filter(|_| node == HOME && number == NUMBER)?;
        Some(ClusterRecordView { kind, number, home_node: node, mark, name: "Hall".into(), revision: 1 })
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

fn standing(home: Home, part: ClusterRecordRef) -> RecordStanding {
    HomeReads { presence: &home, info: &home }.standing(KIND, part)
}

#[test]
fn a_part_is_live_while_its_home_s_set_holds_its_mark() {
    let held = Home { held: Some(mark(2, 7)), live: false, settled_for_ms: None, ..SETTLED };
    assert_eq!(standing(held, part_of(2, 7)), RecordStanding::Live);
}

#[test]
fn a_part_is_an_orphan_only_when_its_settled_home_holds_nothing_there_or_a_newer_generation() {
    assert_eq!(standing(SETTLED, part_of(2, 7)), RecordStanding::Orphan);
    assert_eq!(standing(Home { held: Some(mark(3, 1)), ..SETTLED }, part_of(2, 7)), RecordStanding::Orphan);
    assert_eq!(standing(Home { held: Some(mark(2, 8)), ..SETTLED }, part_of(2, 7)), RecordStanding::Unknown);
    assert_eq!(standing(Home { held: Some(mark(1, 7)), ..SETTLED }, part_of(2, 7)), RecordStanding::Unknown);
}

#[test]
fn no_orphan_is_judged_from_a_home_that_is_stale_split_replaced_or_not_yet_settled() {
    let cases = [
        ("stale", Home { live: false, ..SETTLED }),
        ("split", Home { split: true, ..SETTLED }),
        ("another board's set", Home { mac: [9; 6], ..SETTLED }),
        ("still in its round", Home { settled_for_ms: None, ..SETTLED }),
        ("a set heard before it settled", Home { set_age_ms: 1_000, ..SETTLED }),
    ];
    for (what, home) in cases {
        assert_eq!(standing(home, part_of(2, 7)), RecordStanding::Unknown, "{what}");
    }
}

struct Own(Option<ClusterBirthMark>);

impl ClusterRecordReadPort for Own {
    fn cluster_record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let mark = self.0?;
        Some(ClusterRecordView { kind, number, home_node: 0, mark, name: "Own".into(), revision: 1 })
    }

    fn cluster_record_numbers(&self, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        ClusterRecordNumbers::default()
    }

    fn cluster_generation(&self, _kind: ClusterRecordKind, _number: u8) -> u32 {
        self.0.map_or(0, |mark| mark.generation)
    }
}

#[test]
fn a_part_of_this_node_s_record_is_judged_only_once_the_block_is_settled() {
    let part = part_of(2, 7);
    assert_eq!(own_standing(&Own(Some(mark(2, 7))), BlockState::Listening, KIND, part), RecordStanding::Live);
    assert_eq!(own_standing(&Own(None), BlockState::Listening, KIND, part), RecordStanding::Unknown);
    assert_eq!(own_standing(&Own(None), BlockState::Settled, KIND, part), RecordStanding::Orphan);
    assert_eq!(own_standing(&Own(Some(mark(3, 1))), BlockState::Settled, KIND, part), RecordStanding::Orphan);
    assert_eq!(own_standing(&Own(Some(mark(2, 8))), BlockState::Settled, KIND, part), RecordStanding::Unknown);
}
