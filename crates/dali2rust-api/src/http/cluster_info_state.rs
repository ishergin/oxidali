use std::io::{self, Write};

use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{
    ClusterPartKeys, ClusterRecordNumbers, CLUSTER_NODE_COUNT, CLUSTER_NUMBERS_PER_NODE,
};
use serde::Serialize;

use crate::http::cluster_parts_state::ClusterPartDto;
use crate::http::handlers::common::write_json_array_items;
use crate::http::cluster_records_state::ClusterRecordDto;

const NODES: usize = CLUSTER_NODE_COUNT as usize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterInfoSource {
    Own,
    Cached,
    Stale,
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

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClusterNodePartDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    #[serde(flatten)]
    pub part: ClusterPartDto,
    pub link: ClusterPartLink,
}

pub trait ClusterInfoHttpState: Send + Sync {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto>;

    fn record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto>;

    fn record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers;

    fn part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto>;

    fn part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys;
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
        self.known_nodes().flat_map(move |(node, _)| {
            let numbers = self.state.record_numbers(node, kind).iter();
            numbers.filter_map(move |number| self.state.record(node, kind, number))
        })
    }

    pub fn record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        let home = home_node(number);
        self.source(home)?;
        self.state.record(home, kind, number)
    }

    pub fn parts_of(&self, kind: ClusterRecordKind, record: &ClusterRecordDto) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let (number, mark) = (record.number, record.birth_mark);
        let all = self.known_nodes().flat_map(move |(node, _)| self.held_parts(node, kind));
        all.filter(move |held| held.part.record.number == number).map(move |held| {
            let link = if held.part.record.birth_mark == mark { ClusterPartLink::Live } else { ClusterPartLink::Orphan };
            ClusterNodePartDto { link, ..held }
        })
    }

    pub fn node_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        self.held_parts(node, kind).map(move |held| ClusterNodePartDto { link: self.link(kind, &held.part), ..held })
    }

    fn held_parts(&self, node: u8, kind: ClusterRecordKind) -> impl Iterator<Item = ClusterNodePartDto> + '_ {
        let source = self.source(node);
        let keys = source.map(|_| self.state.part_keys(node, kind)).unwrap_or_default();
        keys.iter().filter_map(move |(adapter, local)| {
            let part = self.state.part(node, kind, adapter, local)?;
            Some(ClusterNodePartDto { node, source: source?, part, link: ClusterPartLink::Unknown })
        })
    }

    fn link(&self, kind: ClusterRecordKind, part: &ClusterPartDto) -> ClusterPartLink {
        let number = part.record.number;
        if self.source(home_node(number)).is_none() {
            return ClusterPartLink::Unknown;
        }
        match self.record(kind, number) {
            Some(record) if record.birth_mark == part.record.birth_mark => ClusterPartLink::Live,
            _ => ClusterPartLink::Orphan,
        }
    }
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
