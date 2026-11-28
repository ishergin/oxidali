use std::io::{self, Write};

use dali2rust_domain::registry::CLUSTER_NODE_COUNT;
use serde::Serialize;

pub const MAX_CLUSTER_STATUS_NODES: u8 = CLUSTER_NODE_COUNT;

pub const MAX_CLUSTER_STATUS_IGNORED: usize = 24;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterWireOwnerDto {
    pub node: u8,
    pub board: String,
    pub this_node: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterOwnDto {
    pub state: &'static str,
    pub node_number: u8,
    pub wire: u8,
    pub board: Option<String>,
    pub boot_epoch: Option<u32>,
    pub wire_owner: Option<ClusterWireOwnerDto>,
    pub split_pair: bool,
    pub wire_chain: bool,
    pub unprotected_wire: bool,
    pub number_held_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterNodeDto {
    pub node: u8,
    pub board: String,
    pub address: String,
    pub installation: String,
    pub wire: u8,
    pub wire_protection: bool,
    pub home_assistant_face: Option<bool>,
    pub heard: bool,
    pub age_ms: u32,
    pub sequence_gaps: u32,
    pub split: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterIgnoredDto {
    pub board: String,
    pub address: String,
    pub node: u8,
    pub reason: &'static str,
    pub cluster_name: Option<String>,
    pub age_ms: u32,
}

pub trait ClusterStatusHttpState: Send + Sync {
    fn own(&self) -> ClusterOwnDto;

    fn node(&self, node: u8) -> Option<ClusterNodeDto>;

    fn ignored(&self, index: usize) -> Option<ClusterIgnoredDto>;

    fn neighbour_names_wire(&self, node: u8, wire: u8) -> bool;
}

pub fn write_cluster_status(state: &dyn ClusterStatusHttpState, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(br#"{"own":"#)?;
    serde_json::to_writer(&mut *out, &state.own())?;
    out.write_all(br#","nodes":["#)?;
    let nodes = (0..MAX_CLUSTER_STATUS_NODES).filter_map(|node| state.node(node));
    write_children(out, nodes)?;
    out.write_all(br#"],"ignored":["#)?;
    let ignored = (0..MAX_CLUSTER_STATUS_IGNORED).map_while(|index| state.ignored(index));
    write_children(out, ignored)?;
    out.write_all(b"]}")
}

fn write_children<T: Serialize>(out: &mut dyn Write, children: impl Iterator<Item = T>) -> io::Result<()> {
    for (index, child) in children.enumerate() {
        if index > 0 {
            out.write_all(b",")?;
        }
        serde_json::to_writer(&mut *out, &child)?;
    }
    Ok(())
}
