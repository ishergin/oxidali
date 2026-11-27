use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{ClusterForgetNodeCommand, Origin};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::CLUSTER_NODE_COUNT;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::cluster_status_state::{write_cluster_status, ClusterStatusHttpState};
use crate::http::dispatcher::{dispatch_and_wait_for_success, CorrelationIdAllocator};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{parse_resource_id_param, require_get, MutatingHandler};

const NODE_PARAM: &str = "node";

const LAST_NODE: u16 = CLUSTER_NODE_COUNT as u16 - 1;

declare_handler_shell! {
    ClusterStatusGetHandler {
        state: Arc<dyn ClusterStatusHttpState>,
    }
}

impl ApiHandler for ClusterStatusGetHandler {
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
        HttpResponse::json_stream(200, Box::new(move |out| write_cluster_status(state.as_ref(), out)))
    }
}

declare_handler_shell!(ClusterNodeForgetHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    timeout_ms: u64,
});

impl MutatingHandler for ClusterNodeForgetHandler {
    type Validated = u8;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "DELETE"
    }

    fn validate(&self, params: &HashMap<String, String>, _body: &[u8]) -> Result<u8, HttpResponse> {
        parse_resource_id_param(params, NODE_PARAM, LAST_NODE)
    }

    fn execute(&self, node: u8) -> Result<(), HttpResponse> {
        let correlation_id = self.correlation.next_id();
        let forget = ClusterForgetNodeCommand { node };
        let command = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(Origin::Api),
            forget,
        );
        let frame = BusFrame::command(command);
        dispatch_and_wait_for_success(&self.publisher, &self.slots, correlation_id, self.timeout_ms, frame)
    }

    fn respond(&self, (): ()) -> HttpResponse {
        HttpResponse::no_content()
    }
}
