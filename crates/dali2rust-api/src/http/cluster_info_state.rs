use std::io::{self, Write};

use dali2rust_contracts::msg::{ClusterBirthMark, ClusterRecordKind, ClusterRecordRef};
use dali2rust_domain::registry::{
    channel_state, ChannelPartLit, ClusterPartKeys, ClusterRecordConflict, ClusterRecordNumbers, CLUSTER_NODE_COUNT,
    CLUSTER_NUMBERS_PER_NODE, MAX_CLUSTER_COPIES_PER_KIND,
};
use serde::Serialize;

use crate::http::cluster_lamps_state::{channel_state_dto, part_state_dto, ClusterChannelStateDto, ClusterPartStateDto};
use crate::http::cluster_parts_state::{ClusterPartDto, ClusterPartRecordDto};
use crate::http::handlers::common::write_json_array_items;
use crate::http::cluster_records_state::ClusterRecordDto;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod channel_state_tests;

const NODES: usize = CLUSTER_NODE_COUNT as usize;

pub const CLUSTER_BLOCK_UNSETTLED: &str = ClusterRecordConflict::BlockUnsettled.message();

pub const CLUSTER_NODE_CONTESTED: &str = "cluster_node_contested";

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterInfoSource {
    Own,
    Cached,
    Stale,
    Copy,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterPartLink {
    Live,
    Orphan,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInfoNodeDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    pub set: Option<u32>,
    pub age_ms: Option<u32>,
    pub channels: u8,
    pub scenes: u8,
    pub channel_parts: u8,
    pub scene_parts: u8,
    pub skipped: u16,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterWireHolding {
    OtherRecord,
    Nothing,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterWireFaultDto {
    pub node: u8,
    pub holds: ClusterWireHolding,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClusterNodePartDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    #[serde(flatten)]
    pub part: ClusterPartDto,
    pub link: ClusterPartLink,
    pub wire_fault: Option<ClusterWireFaultDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ClusterPartStateDto>,
    #[serde(skip)]
    pub lit: Option<ChannelPartLit>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterRecordItemDto {
    #[serde(flatten)]
    pub record: ClusterRecordDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ClusterChannelStateDto>,
}

pub trait ClusterInfoHttpState: Send + Sync {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto>;

    fn record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto>;

    fn record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers;

    fn part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto>;

    fn part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys;

    fn copy_at(&self, _kind: ClusterRecordKind, _index: usize) -> Option<ClusterRecordDto> {
        None
    }

    fn link(&self, _kind: ClusterRecordKind, _record: ClusterRecordRef) -> ClusterPartLink {
        ClusterPartLink::Unknown
    }

    fn wire(&self, _node: u8) -> Option<u8> {
        None
    }

    fn record_write_block(&self) -> Result<u8, &'static str>;

    fn part_lit(&self, _node: u8, _adapter: u8, _group: u8, _channel: ClusterRecordRef) -> ChannelPartLit {
        ChannelPartLit::UNHEARD
    }
}

#[derive(Clone, Copy)]
pub struct ClusterView<'a> {
    state: &'a dyn ClusterInfoHttpState,
    sources: [Option<ClusterInfoSource>; NODES],
    wires: [Option<u8>; NODES],
}

impl<'a> ClusterView<'a> {
    pub fn read(state: &'a dyn ClusterInfoHttpState) -> Self {
        let mut sources = [None; NODES];
        let mut wires = [None; NODES];
        for ((node, source), wire) in (0..CLUSTER_NODE_COUNT).zip(sources.iter_mut()).zip(wires.iter_mut()) {
            *source = state.node_info(node).map(|info| info.source);
            *wire = state.wire(node);
        }
        Self { state, sources, wires }
    }

    fn wire(&self, node: u8) -> Option<u8> {
        self.wires.get(usize::from(node)).copied().flatten()
    }

    pub fn source(&self, node: u8) -> Option<ClusterInfoSource> {
        self.sources.get(usize::from(node)).copied().flatten()
    }

    fn known_nodes(&self) -> impl Iterator<Item = (u8, ClusterInfoSource)> + '_ {
        (0..CLUSTER_NODE_COUNT).filter_map(|node| self.source(node).map(|source| (node, source)))
    }

    pub fn records(&self, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterRecordDto> + '_ {
        let held = self.known_nodes().flat_map(move |(node, _)| {
            let numbers = self.state.record_numbers(node, kind).iter();
            numbers.filter_map(move |number| self.state.record(node, kind, number))
        });
        held.chain(self.copies(kind))
    }

    fn copies(&self, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterRecordDto> + '_ {
        let mut copies = (0..MAX_CLUSTER_COPIES_PER_KIND).map_while(move |index| self.state.copy_at(kind, index)).peekable();
        core::iter::from_fn(move || {
            let copy = copies.next()?;
            let newest = copies.peek().is_none_or(|next| next.number != copy.number);
            Some(newest.then_some(copy))
        })
        .flatten()
        .filter(|copy| self.source(copy.home_node).is_none())
    }

    pub fn record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        let home = home_node(number);
        if self.source(home).is_some() {
            return self.state.record(home, kind, number);
        }
        self.copies(kind).find(|copy| copy.number == number)
    }

    pub fn parts_of(&self, kind: ClusterRecordKind, record: &ClusterRecordDto) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        self.record_parts(kind, record).map(move |held| self.lit(kind, self.with_wire_fault(kind, held)))
    }

    fn record_parts(&self, kind: ClusterRecordKind, record: &ClusterRecordDto) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let number = record.number;
        let all = self.known_nodes().flat_map(move |(node, _)| self.held_parts(node, kind));
        all.filter(move |held| held.part.record.number == number).map(move |held| self.linked(kind, held))
    }

    pub fn node_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let linked = self.held_parts(node, kind).map(move |held| self.linked(kind, held));
        linked.map(move |held| self.lit(kind, self.with_wire_fault(kind, held)))
    }

    fn linked(&self, kind: ClusterRecordKind, held: ClusterNodePartDto) -> ClusterNodePartDto {
        ClusterNodePartDto { link: self.state.link(kind, record_ref(held.part.record)), ..held }
    }

    fn with_wire_fault(&self, kind: ClusterRecordKind, held: ClusterNodePartDto) -> ClusterNodePartDto {
        ClusterNodePartDto { wire_fault: self.wire_fault(kind, &held), ..held }
    }

    fn held_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let source = self.source(node);
        let keys = source.map(|_| self.state.part_keys(node, kind)).unwrap_or_default();
        keys.iter().filter_map(move |(adapter, local)| {
            let part = self.state.part(node, kind, adapter, local)?;
            let link = ClusterPartLink::Unknown;
            Some(ClusterNodePartDto { node, source: source?, part, link, wire_fault: None, state: None, lit: None })
        })
    }

    fn wire_fault(&self, kind: ClusterRecordKind, held: &ClusterNodePartDto) -> Option<ClusterWireFaultDto> {
        let wire = self.wire(held.node)?;
        let (adapter, local) = (held.part.adapter, held.part.group.or(held.part.scene)?);
        let neighbours = self.known_nodes().filter(|(node, source)| {
            *node != held.node && matches!(source, ClusterInfoSource::Own | ClusterInfoSource::Cached)
        });
        neighbours.filter(|(node, _)| self.wire(*node) == Some(wire)).find_map(|(node, _)| {
            let holds = match self.state.part(node, kind, adapter, local) {
                None => ClusterWireHolding::Nothing,
                Some(other) if other.record.number != held.part.record.number => ClusterWireHolding::OtherRecord,
                Some(other) if other.record.birth_mark != held.part.record.birth_mark => ClusterWireHolding::OtherRecord,
                Some(_) => return None,
            };
            Some(ClusterWireFaultDto { node, holds })
        })
    }

    fn lit(&self, kind: ClusterRecordKind, held: ClusterNodePartDto) -> ClusterNodePartDto {
        let lit = self.part_lit(held.node, kind, &held.part);
        ClusterNodePartDto { state: lit.map(part_state_dto), lit, ..held }
    }

    fn part_lit(&self, node: u8, kind: ClusterRecordKind, part: &ClusterPartDto) -> Option<ChannelPartLit> {
        let group = part.group.filter(|_| kind == ClusterRecordKind::Channel)?;
        Some(self.state.part_lit(node, part.adapter, group, record_ref(part.record)))
    }

    pub fn with_state(&self, kind: ClusterRecordKind, record: ClusterRecordDto) -> ClusterRecordItemDto {
        let state = (kind == ClusterRecordKind::Channel).then(|| self.channel_state(&record));
        ClusterRecordItemDto { record, state }
    }

    fn channel_state(&self, record: &ClusterRecordDto) -> ClusterChannelStateDto {
        let kind = ClusterRecordKind::Channel;
        let held = || self.record_parts(kind, record).filter(|held| held.link != ClusterPartLink::Orphan);
        let live = held().filter(|held| held.link == ClusterPartLink::Live);
        let nodes = live.fold(0_u32, |mask, held| mask | 1_u32.checked_shl(u32::from(held.node)).unwrap_or(0));
        let counted = held().filter(move |held| !self.behind_wire_owner(held.node, nodes));
        channel_state_dto(channel_state(counted.map(|held| self.counted_lit(kind, held))))
    }

    fn counted_lit(&self, kind: ClusterRecordKind, held: ClusterNodePartDto) -> ChannelPartLit {
        let live = held.link == ClusterPartLink::Live;
        live.then(|| self.lit(kind, held).lit).flatten().unwrap_or(ChannelPartLit::UNHEARD)
    }

    fn behind_wire_owner(&self, node: u8, nodes: u32) -> bool {
        let owner_has_part = |wire: u8| nodes & 1_u32.checked_shl(u32::from(wire)).unwrap_or(0) != 0;
        self.wire(node).is_some_and(|wire| wire != node && owner_has_part(wire))
    }
}

const fn record_ref(record: ClusterPartRecordDto) -> ClusterRecordRef {
    let mark = ClusterBirthMark { generation: record.birth_mark.generation, nonce: record.birth_mark.nonce };
    ClusterRecordRef { number: record.number, mark }
}

#[must_use]
pub const fn home_node(number: u8) -> u8 {
    number / CLUSTER_NUMBERS_PER_NODE
}

pub fn write_cluster_info(state: &dyn ClusterInfoHttpState, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(br#"{"nodes":["#)?;
    let nodes = (0..CLUSTER_NODE_COUNT).filter_map(|node| state.node_info(node));
    write_json_array_items(out, nodes)?;
    out.write_all(b"]}")
}

pub fn write_object_then<T: Serialize>(out: &mut dyn Write, object: &T, key: &str) -> io::Result<()> {
    let bytes = serde_json::to_vec(object)?;
    let open = bytes.strip_suffix(b"}").ok_or_else(|| io::Error::other("not a JSON object"))?;
    out.write_all(open)?;
    let separator: &[u8] = if open.len() > 1 { b"," } else { b"" };
    out.write_all(separator)?;
    serde_json::to_writer(&mut *out, key)?;
    out.write_all(b":")
}
