use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

use dali2rust_domain::registry::ClusterGroupLitView;

use super::*;
use crate::http::cluster_parts_state::ClusterPartRecordDto;
use crate::http::cluster_records_state::ClusterBirthMarkDto;

const NODES: u8 = 4;
const GROUPS: u8 = 16;
const MARK: ClusterBirthMarkDto = ClusterBirthMarkDto { generation: 1, nonce: 1 };
const UNPROVEN: ClusterBirthMarkDto = ClusterBirthMarkDto { generation: 2, nonce: 2 };
const RECORD: u8 = 3;
const WIRE_OWNER: u8 = 0;

#[derive(Default)]
struct Counted {
    part_lit_calls: AtomicU32,
    link_calls: AtomicU32,
    home_unheard: bool,
    part_calls: AtomicU32,
    shared_wire: bool,
    owner_unproven: bool,
}

impl ClusterInfoHttpState for Counted {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto> {
        (node < NODES).then_some(ClusterInfoNodeDto {
            node,
            source: ClusterInfoSource::Cached,
            set: Some(1),
            age_ms: Some(0),
            channels: GROUPS,
            scenes: 0,
            channel_parts: GROUPS,
            scene_parts: 0,
            skipped: 0,
        })
    }

    fn record(&self, node: u8, _kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        let source = ClusterInfoSource::Cached;
        (node == 0).then(|| ClusterRecordDto { number, home_node: 0, birth_mark: MARK, name: String::new(), revision: 1, source })
    }

    fn record_numbers(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        ClusterRecordNumbers::default()
    }

    fn part(&self, node: u8, _kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto> {
        self.part_calls.fetch_add(1, Relaxed);
        let birth_mark = if self.owner_unproven && node == WIRE_OWNER { UNPROVEN } else { MARK };
        let record = ClusterPartRecordDto { number: local, birth_mark };
        Some(ClusterPartDto { adapter, group: Some(local), scene: None, record, send: true, accept: true, revision: 1 })
    }

    fn part_keys(&self, _node: u8, _kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        (0..GROUPS).for_each(|group| keys.insert(0, group));
        keys
    }

    fn link(&self, _kind: ClusterRecordKind, record: ClusterRecordRef) -> ClusterPartLink {
        self.link_calls.fetch_add(1, Relaxed);
        if self.home_unheard {
            return ClusterPartLink::Unknown;
        }
        let mark = |dto: ClusterBirthMarkDto| ClusterBirthMark { generation: dto.generation, nonce: dto.nonce };
        match record.mark {
            live if live == mark(MARK) => ClusterPartLink::Live,
            unproven if unproven == mark(UNPROVEN) => ClusterPartLink::Unknown,
            _ => ClusterPartLink::Orphan,
        }
    }

    fn wire(&self, _node: u8) -> Option<u8> {
        self.shared_wire.then_some(WIRE_OWNER)
    }

    fn record_write_block(&self) -> Result<u8, &'static str> {
        Ok(0)
    }

    fn part_lit(&self, _node: u8, _adapter: u8, _group: u8, _channel: ClusterRecordRef) -> ChannelPartLit {
        self.part_lit_calls.fetch_add(1, Relaxed);
        ChannelPartLit::UNHEARD
    }
}

#[test]
fn a_channel_s_state_reads_the_lit_counts_of_its_own_parts_only_and_once() {
    let state = Counted::default();
    let view = ClusterView::read(&state);
    let record = view.record(ClusterRecordKind::Channel, RECORD).expect("the record");
    let item = view.with_state(ClusterRecordKind::Channel, record);
    assert_eq!(item.state.map(|state| state.parts_unknown), Some(NODES), "one part a node, each unheard");
    let calls = state.part_lit_calls.load(Relaxed);
    assert_eq!(calls, u32::from(NODES), "lit counts read for {calls} parts, the channel has {NODES}");
}

#[test]
fn a_stale_part_shows_its_counts_unknown_never_its_last_ones() {
    let heard = ClusterGroupLitView { commanded: true, lit: 2, known: 2, members: 2 };
    let fresh = part_state_dto(ChannelPartLit { lit: Some(heard), fresh: true });
    assert_eq!((fresh.on, fresh.lit, fresh.members), (Some(true), Some(2), Some(2)));
    let stale = part_state_dto(ChannelPartLit { lit: Some(heard), fresh: false });
    assert_eq!((stale.on, stale.commanded, stale.lit, stale.known, stale.members), (None, None, None, None, None));
    assert!(!stale.fresh);
}

#[test]
fn a_channel_s_state_links_only_its_own_parts() {
    let state = Counted::default();
    let view = ClusterView::read(&state);
    let record = view.record(ClusterRecordKind::Channel, RECORD).expect("the record");
    let _ = view.with_state(ClusterRecordKind::Channel, record);
    let calls = state.link_calls.load(Relaxed);
    let most = 2 * u32::from(NODES);
    assert!(calls <= most, "linked {calls} parts for a channel with {NODES}, at most {most}");
}

#[test]
fn a_channel_whose_home_is_unheard_counts_its_parts_unknown_never_none() {
    let state = Counted { home_unheard: true, ..Counted::default() };
    let view = ClusterView::read(&state);
    let record = view.record(ClusterRecordKind::Channel, RECORD).expect("the record");
    let channel = view.with_state(ClusterRecordKind::Channel, record).state.expect("a channel's state");
    assert_eq!((channel.on, channel.parts_unknown), (None, NODES), "every part held, none provable");
    assert_eq!(state.part_lit_calls.load(Relaxed), 0, "an unlinked part's counts are not read");
}

#[test]
fn a_channel_s_state_looks_for_no_wire_fault() {
    let state = Counted { shared_wire: true, ..Counted::default() };
    let view = ClusterView::read(&state);
    let record = view.record(ClusterRecordKind::Channel, RECORD).expect("the record");
    state.part_calls.store(0, Relaxed);
    let _ = view.with_state(ClusterRecordKind::Channel, record);
    let walked = 2 * u32::from(NODES) * u32::from(GROUPS);
    let calls = state.part_calls.load(Relaxed);
    assert!(calls <= walked, "{calls} part reads, the two walks over every node's parts take {walked}");
}

#[test]
fn a_wire_owner_s_unproven_part_does_not_hide_the_proven_parts_behind_it() {
    let state = Counted { shared_wire: true, owner_unproven: true, ..Counted::default() };
    let view = ClusterView::read(&state);
    let record = view.record(ClusterRecordKind::Channel, RECORD).expect("the record");
    let channel = view.with_state(ClusterRecordKind::Channel, record).state.expect("a channel's state");
    assert_eq!(channel.parts_unknown, NODES, "the owner's part and the three behind it, each unheard in the fake");
    assert_eq!(state.part_lit_calls.load(Relaxed), u32::from(NODES - 1), "the proven parts behind the owner are read");
}
