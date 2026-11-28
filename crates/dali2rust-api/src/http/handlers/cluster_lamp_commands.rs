use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_contracts::msg::{ClusterAction, ClusterLampAddress, ClusterTarget};
use dali2rust_domain::registry::{CLUSTER_NODE_COUNT, VIRTUAL_LAMP_COUNT};

use crate::http::cluster_info_state::ClusterInfoSource;
use crate::http::cluster_lamps_state::{ClusterLampAddressDto, ClusterLampDto, ClusterLampsHttpState};
use crate::http::handler::ApiHandler;
use crate::http::handlers::cluster_commands::{light_verb, publish_cluster_command, ClusterCommandDeps};
use crate::http::target_state_request::{parse_target_request, GroupTargetRequest, TargetStateBody};
use crate::http::types::HttpResponse;

use super::common::{
    gate_refusal, json_err, json_err_with_message, parse_resource_id_param, parse_typed_body, vl_cap_supports_color_mode,
};

const METHOD: &str = "PUT";
const NODE_PARAM: &str = "node";
const LAMP_PARAM: &str = "lamp";
const OWN_ADAPTER_PARAM: &str = "adapter_id";
const OWN_LAMP_PARAM: &str = "lamp_id";
const OWN_ADAPTER: &str = "0";
const LAST_NODE: u16 = CLUSTER_NODE_COUNT as u16 - 1;
const LAST_LAMP: u16 = VIRTUAL_LAMP_COUNT as u16 - 1;
const CLUSTER_LAMP_STALE: &str = "cluster_lamp_stale";
const CLUSTER_LAMP_UNBOUND: &str = "cluster_lamp_unbound";

pub struct ClusterLampTargetStateHandler {
    deps: Arc<ClusterCommandDeps>,
    lamps: Arc<dyn ClusterLampsHttpState>,
    own: Box<dyn ApiHandler>,
}

impl ClusterLampTargetStateHandler {
    pub fn new(deps: Arc<ClusterCommandDeps>, lamps: Arc<dyn ClusterLampsHttpState>, own: Box<dyn ApiHandler>) -> Self {
        Self { deps, lamps, own }
    }
}

impl ApiHandler for ClusterLampTargetStateHandler {
    fn handle_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != METHOD {
            return HttpResponse::method_not_allowed();
        }
        let answer = lamp_address(params).and_then(|(node, lamp)| {
            if node == self.lamps.own_node() {
                return Ok(self.own.handle_request(method, path, body, &own_params(lamp)));
            }
            self.remote(node, lamp, body)
        });
        answer.unwrap_or_else(|refusal| refusal)
    }
}

impl ClusterLampTargetStateHandler {
    fn remote(&self, node: u8, lamp: u8, body: &[u8]) -> Result<HttpResponse, HttpResponse> {
        if let Some(cause) = self.lamps.command_refusal() {
            return Err(gate_refusal(cause));
        }
        let dto = self.lamps.lamp(node, lamp).ok_or_else(|| json_err(404, "not_found"))?;
        fresh(&dto)?;
        let body = parse_typed_body::<TargetStateBody>(body)?;
        let request = parse_target_request(&body, |mode| vl_cap_supports_color_mode(&dto.lamp, mode))?;
        bound(&dto)?;
        let ClusterLampAddressDto { node, adapter, lamp } = dto.cluster_address;
        let target = ClusterTarget::Lamp(ClusterLampAddress { node, adapter, lamp });
        let verb = light_verb(GroupTargetRequest::Target(request));
        publish_cluster_command(&self.deps, ClusterAction::Light { target, verb })
    }
}

fn lamp_address(params: &HashMap<String, String>) -> Result<(u8, u8), HttpResponse> {
    let node = parse_resource_id_param(params, NODE_PARAM, LAST_NODE)?;
    Ok((node, parse_resource_id_param(params, LAMP_PARAM, LAST_LAMP)?))
}

fn own_params(lamp: u8) -> HashMap<String, String> {
    HashMap::from([(OWN_ADAPTER_PARAM.to_owned(), OWN_ADAPTER.to_owned()), (OWN_LAMP_PARAM.to_owned(), lamp.to_string())])
}

fn fresh(dto: &ClusterLampDto) -> Result<(), HttpResponse> {
    let stale = dto.source != ClusterInfoSource::Cached;
    if stale { Err(json_err_with_message(409, "conflict", CLUSTER_LAMP_STALE)) } else { Ok(()) }
}

fn bound(dto: &ClusterLampDto) -> Result<(), HttpResponse> {
    if dto.bound { Ok(()) } else { Err(json_err_with_message(409, "conflict", CLUSTER_LAMP_UNBOUND)) }
}
