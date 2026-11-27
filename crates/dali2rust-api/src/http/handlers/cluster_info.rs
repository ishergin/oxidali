use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::CLUSTER_NODE_COUNT;

use crate::http::cluster_info_state::{write_cluster_info, ClusterInfoHttpState, ClusterView};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::cluster_parts::list_key;
use super::common::{json_err, parse_resource_id_param, require_get, write_json_array_items};

const NODE_PARAM: &str = "node";

const LAST_NODE: u16 = CLUSTER_NODE_COUNT as u16 - 1;

declare_handler_shell! {
    ClusterInfoGetHandler {
        state: Arc<dyn ClusterInfoHttpState>,
    }
}

impl ApiHandler for ClusterInfoGetHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        let state = Arc::clone(&self.state);
        HttpResponse::json_stream(200, Box::new(move |out| write_cluster_info(state.as_ref(), out)))
    }
}

declare_handler_shell! {
    ClusterNodePartsHandler {
        state: Arc<dyn ClusterInfoHttpState>,
        kind: ClusterRecordKind,
    }
}

impl ApiHandler for ClusterNodePartsHandler {
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
        match parse_resource_id_param(params, NODE_PARAM, LAST_NODE) {
            Ok(node) => self.node_parts(node),
            Err(refusal) => refusal,
        }
    }
}

impl ClusterNodePartsHandler {
    fn node_parts(&self, node: u8) -> HttpResponse {
        if ClusterView::read(self.state.as_ref()).source(node).is_none() {
            return json_err(404, "not_found");
        }
        let (kind, state) = (self.kind, Arc::clone(&self.state));
        HttpResponse::json_stream(
            200,
            Box::new(move |w: &mut dyn Write| {
                write!(w, "{{\"node\":{node},\"{}\":[", list_key(kind))?;
                write_json_array_items(w, ClusterView::read(state.as_ref()).node_parts(node, kind))?;
                w.write_all(b"]}")
            }),
        )
    }
}
