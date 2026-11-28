use std::io::{self, Write};

use dali2rust_contracts::msg::ClusterRecordRef;
use dali2rust_domain::registry::{ChannelPartLit, ChannelStateView, CLUSTER_NODE_COUNT, VIRTUAL_LAMP_COUNT};
use serde::Serialize;

use crate::ha::topics::HaTopics;
use crate::http::cluster_info_state::ClusterInfoSource;
use crate::http::handlers::common::write_json_array_items;
use crate::http::virtual_lamp_state::VirtualLampDto;

const LIGHT_COMPONENT: &str = "light";

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterLampAddressDto {
    pub node: u8,
    pub adapter: u8,
    pub lamp: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterLampHaDto {
    pub unique_id: String,
    pub discovery_topic: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterLampDto {
    pub cluster_address: ClusterLampAddressDto,
    pub source: ClusterInfoSource,
    #[serde(flatten)]
    pub lamp: VirtualLampDto,
    pub bound: bool,
    pub observation_age_ms: Option<u32>,
    pub home_assistant: Option<ClusterLampHaDto>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterLampsNodeDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    pub lamps: u8,
    pub state_set: Option<u32>,
    pub info_set: Option<u32>,
    pub age_ms: Option<u32>,
    pub sequence_gaps: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterChannelStateDto {
    pub on: Option<bool>,
    pub lit: u8,
    pub known: u8,
    pub members: u8,
    pub parts_unknown: u8,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterPartStateDto {
    pub on: Option<bool>,
    pub commanded: Option<bool>,
    pub lit: Option<u8>,
    pub known: Option<u8>,
    pub members: Option<u8>,
    pub fresh: bool,
}

pub trait ClusterLampsHttpState: Send + Sync {
    fn lamps_node(&self, node: u8) -> Option<ClusterLampsNodeDto>;

    fn lamp_ids(&self, node: u8) -> u64;

    fn lamp(&self, node: u8, lamp: u8) -> Option<ClusterLampDto>;

    fn part_lit(&self, node: u8, adapter: u8, group: u8, channel: ClusterRecordRef) -> ChannelPartLit;

    fn own_node(&self) -> u8;

    fn command_refusal(&self) -> Option<&'static str>;
}

pub fn ha_identity_dto(controller_id: &str, discovery_prefix: &str, adapter: u8, lamp: u8) -> ClusterLampHaDto {
    let topics = HaTopics::new(discovery_prefix, "", controller_id);
    let object_id = HaTopics::lamp_object_id(adapter, lamp);
    ClusterLampHaDto { unique_id: topics.unique_id(&object_id), discovery_topic: topics.discovery_topic(LIGHT_COMPONENT, &object_id) }
}

pub const fn channel_state_dto(view: ChannelStateView) -> ClusterChannelStateDto {
    let ChannelStateView { on, lit, known, members, parts_unknown } = view;
    ClusterChannelStateDto { on, lit, known, members, parts_unknown }
}

pub fn part_state_dto(part: ChannelPartLit) -> ClusterPartStateDto {
    let on = dali2rust_domain::registry::channel_state([part]).on;
    let lit = part.lit.filter(|_| part.fresh);
    ClusterPartStateDto {
        on,
        commanded: lit.map(|lit| lit.commanded),
        lit: lit.map(|lit| lit.lit),
        known: lit.map(|lit| lit.known),
        members: lit.map(|lit| lit.members),
        fresh: part.fresh,
    }
}

fn lamp_numbers(ids: u64) -> impl Iterator<Item = u8> {
    (0..VIRTUAL_LAMP_COUNT).filter(move |lamp| ids & 1_u64.checked_shl(u32::from(*lamp)).unwrap_or(0) != 0)
}

pub fn write_cluster_lamps(state: &dyn ClusterLampsHttpState, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(br#"{"nodes":["#)?;
    write_json_array_items(out, (0..CLUSTER_NODE_COUNT).filter_map(|node| state.lamps_node(node)))?;
    out.write_all(b"]}")
}

pub fn write_node_lamps(state: &dyn ClusterLampsHttpState, node: ClusterLampsNodeDto, out: &mut dyn Write) -> io::Result<()> {
    write!(out, "{{\"node\":{},\"source\":", node.node)?;
    serde_json::to_writer(&mut *out, &node.source)?;
    out.write_all(br#","lamps":["#)?;
    let lamps = lamp_numbers(state.lamp_ids(node.node)).filter_map(|lamp| state.lamp(node.node, lamp));
    write_json_array_items(out, lamps)?;
    out.write_all(b"]}")
}
