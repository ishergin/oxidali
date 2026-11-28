use std::io::{self, Write};

use dali2rust_contracts::msg::{ClusterBirthMark, ClusterRecordKind, ClusterRecordRef};
use dali2rust_domain::registry::{
    ClusterPartKeys, ClusterRecordNumbers, CLUSTER_NODE_COUNT, CLUSTER_NUMBERS_PER_NODE, MAX_CLUSTER_COPIES_PER_KIND,
};
use serde::Serialize;

use crate::http::cluster_parts_state::{ClusterPartDto, ClusterPartRecordDto};
use crate::http::handlers::common::write_json_array_items;
use crate::http::cluster_records_state::ClusterRecordDto;

#[cfg(test)]
mod tests;

const NODES: usize = CLUSTER_NODE_COUNT as usize;

pub const CLUSTER_BLOCK_UNSETTLED: &str = "cluster_block_unsettled";

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
}

pub trait ClusterInfoHttpState: Send + Sync {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto>;

    fn record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto>;

    fn record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers;

    fn part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto>;

    fn part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys;

    fn copy_at(&self, kind: ClusterRecordKind, index: usize) -> Option<ClusterRecordDto>;

    fn link(&self, kind: ClusterRecordKind, record: ClusterRecordRef) -> ClusterPartLink;

    fn wire(&self, node: u8) -> Option<u8>;

    fn record_write_refusal(&self) -> Option<&'static str>;
}

#[derive(Clone, Copy)]
pub struct ClusterView<'a> {
    state: &'a dyn ClusterInfoHttpState,
    sources: [Option<ClusterInfoSource>; NODES],
}

impl<'a> ClusterView<'a> {
    pub fn read(state: &'a dyn ClusterInfoHttpState) -> Self {
        let mut sources = [None; NODES];
        for (node, source) in (0..CLUSTER_NODE_COUNT).zip(sources.iter_mut()) {
            *source = state.node_info(node).map(|info| info.source);
        }
        Self { state, sources }
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
        let number = record.number;
        let all = self.known_nodes().flat_map(move |(node, _)| self.node_parts(node, kind));
        all.filter(move |held| held.part.record.number == number)
    }

    pub fn node_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        self.held_parts(node, kind).map(move |held| {
            let link = self.state.link(kind, record_ref(held.part.record));
            ClusterNodePartDto { link, wire_fault: self.wire_fault(kind, &held), ..held }
        })
    }

    fn held_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let source = self.source(node);
        let keys = source.map(|_| self.state.part_keys(node, kind)).unwrap_or_default();
        keys.iter().filter_map(move |(adapter, local)| {
            let part = self.state.part(node, kind, adapter, local)?;
            Some(ClusterNodePartDto { node, source: source?, part, link: ClusterPartLink::Unknown, wire_fault: None })
        })
    }

    fn wire_fault(&self, kind: ClusterRecordKind, held: &ClusterNodePartDto) -> Option<ClusterWireFaultDto> {
        let wire = self.state.wire(held.node)?;
        let (adapter, local) = (held.part.adapter, held.part.group.or(held.part.scene)?);
        let neighbours = self.known_nodes().filter(|(node, source)| {
            *node != held.node && matches!(source, ClusterInfoSource::Own | ClusterInfoSource::Cached)
        });
        neighbours.filter(|(node, _)| self.state.wire(*node) == Some(wire)).find_map(|(node, _)| {
            let holds = match self.state.part(node, kind, adapter, local) {
                None => ClusterWireHolding::Nothing,
                Some(other) if other.record.number != held.part.record.number => ClusterWireHolding::OtherRecord,
                Some(other) if other.record.birth_mark != held.part.record.birth_mark => ClusterWireHolding::OtherRecord,
                Some(_) => return None,
            };
            Some(ClusterWireFaultDto { node, holds })
        })
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
