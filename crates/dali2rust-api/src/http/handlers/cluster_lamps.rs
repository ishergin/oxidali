use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_domain::registry::{CLUSTER_NODE_COUNT, VIRTUAL_LAMP_COUNT};

use crate::http::cluster_lamps_state::{write_cluster_lamps, write_node_lamps, ClusterLampsHttpState};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{json_err, json_stream_dto, parse_resource_id_param, require_get};

const NODE_PARAM: &str = "node";

const LAMP_PARAM: &str = "lamp";

const LAST_NODE: u16 = CLUSTER_NODE_COUNT as u16 - 1;

const LAST_LAMP: u16 = VIRTUAL_LAMP_COUNT as u16 - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterLampsRoute {
    Nodes,
    NodeLamps,
    NodeLamp,
}

declare_handler_shell! {
    ClusterLampsHandler {
        state: Arc<dyn ClusterLampsHttpState>,
        route: ClusterLampsRoute,
    }
}

impl ApiHandler for ClusterLampsHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        let answer = match self.route {
            ClusterLampsRoute::Nodes => Ok(self.nodes()),
            ClusterLampsRoute::NodeLamps => self.node_lamps(params),
            ClusterLampsRoute::NodeLamp => self.node_lamp(params),
        };
        answer.unwrap_or_else(|refusal| refusal)
    }
}

impl ClusterLampsHandler {
    fn nodes(&self) -> HttpResponse {
        let state = Arc::clone(&self.state);
        HttpResponse::json_stream(200, Box::new(move |out| write_cluster_lamps(state.as_ref(), out)))
    }

    fn node_lamps(&self, params: &HashMap<String, String>) -> Result<HttpResponse, HttpResponse> {
        let node = parse_resource_id_param(params, NODE_PARAM, LAST_NODE)?;
        let held = self.state.lamps_node(node).ok_or_else(|| json_err(404, "not_found"))?;
        let state = Arc::clone(&self.state);
        Ok(HttpResponse::json_stream(200, Box::new(move |out| write_node_lamps(state.as_ref(), held, out))))
    }

    fn node_lamp(&self, params: &HashMap<String, String>) -> Result<HttpResponse, HttpResponse> {
        let node = parse_resource_id_param(params, NODE_PARAM, LAST_NODE)?;
        let lamp = parse_resource_id_param(params, LAMP_PARAM, LAST_LAMP)?;
        let dto = self.state.lamp(node, lamp).ok_or_else(|| json_err(404, "not_found"))?;
        Ok(json_stream_dto(dto))
    }
}
