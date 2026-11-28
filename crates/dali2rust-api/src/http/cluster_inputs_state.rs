use std::io::{self, Write};

use dali2rust_domain::registry::{CLUSTER_INPUT_DEVICES, CLUSTER_INPUT_INSTANCES, CLUSTER_NODE_COUNT, INPUT_INSTANCE_GROUPS};
use serde::Serialize;

use crate::http::cluster_info_state::{write_object_then, ClusterInfoSource};
use crate::http::handlers::common::write_json_array_items;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInputOwnerDto {
    pub controller_id: String,
    pub discovery_prefix: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInputsNodeDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    pub set: Option<u32>,
    pub age_ms: Option<u32>,
    pub device_count: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInputDeviceDto {
    pub node: u8,
    pub source: ClusterInfoSource,
    pub adapter_id: u8,
    pub short_address: u8,
    pub name: Option<String>,
    pub present: bool,
    pub instance_count: u8,
    pub ha_entity: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInputRuntimeDto {
    pub last_event_info: Option<u16>,
    pub input_value: Option<u16>,
    pub age_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterInputInstanceDto {
    pub instance_number: u8,
    pub instance_type: Option<u8>,
    pub instance_type_name: Option<&'static str>,
    pub instance_groups: [Option<u8>; INPUT_INSTANCE_GROUPS],
    pub runtime: ClusterInputRuntimeDto,
}

pub trait ClusterInputsHttpState: Send + Sync {
    fn inputs_node(&self, node: u8) -> Option<ClusterInputsNodeDto>;

    fn input_owner(&self, node: u8) -> Option<ClusterInputOwnerDto>;

    fn input_device(&self, node: u8, index: usize) -> Option<ClusterInputDeviceDto>;

    fn input_instance(&self, node: u8, device_index: usize, index: usize) -> Option<ClusterInputInstanceDto>;
}

pub fn write_cluster_inputs(state: &dyn ClusterInputsHttpState, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(br#"{"nodes":["#)?;
    let nodes = (0..CLUSTER_NODE_COUNT).filter_map(|node| state.inputs_node(node));
    write_json_array_items(out, nodes)?;
    out.write_all(b"]}")
}

pub fn write_node_inputs(state: &dyn ClusterInputsHttpState, node: &ClusterInputsNodeDto, out: &mut dyn Write) -> io::Result<()> {
    write_object_then(out, node, "owner")?;
    serde_json::to_writer(&mut *out, &state.input_owner(node.node))?;
    out.write_all(br#","devices":["#)?;
    let devices = (0..CLUSTER_INPUT_DEVICES).map_while(|index| state.input_device(node.node, index).map(|device| (index, device)));
    for (index, device) in devices {
        if index > 0 {
            out.write_all(b",")?;
        }
        write_device(state, node.node, (index, &device), out)?;
    }
    out.write_all(b"]}")
}

fn write_device(
    state: &dyn ClusterInputsHttpState,
    node: u8,
    (index, device): (usize, &ClusterInputDeviceDto),
    out: &mut dyn Write,
) -> io::Result<()> {
    write_object_then(out, device, "instances")?;
    out.write_all(b"[")?;
    let instances = (0..CLUSTER_INPUT_INSTANCES).map_while(|at| state.input_instance(node, index, at));
    write_json_array_items(out, instances)?;
    out.write_all(b"]}")
}
