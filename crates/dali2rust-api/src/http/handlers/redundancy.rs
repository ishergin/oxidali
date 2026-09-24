use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::Dali103HandoverCommand;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{json_err, json_stream_dto, require_get};
use crate::http::redundancy_settings_state::RedundancySettingsHttpState;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::redundancy_state::RedundancyHttpState;
use crate::http::types::HttpResponse;

declare_handler_shell! {
    RedundancyGetHandler {
        state: Arc<dyn RedundancyHttpState>,
    }
}

impl ApiHandler for RedundancyGetHandler {
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
        json_stream_dto(self.state.redundancy_state_dto())
    }
}

declare_handler_shell!(RedundancySwitchoverHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn RedundancyHttpState>,
    settings: Arc<dyn RedundancySettingsHttpState>,
    bus_id: BusId,
    registry_adapter_id: u8,
    timeout_ms: u64,
});

pub struct SwitchoverData {
    peer_short_address: u8,
}

impl crate::http::handlers::common::MutatingHandler for RedundancySwitchoverHandler {
    type Validated = SwitchoverData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<SwitchoverData, HttpResponse> {
        let state = self.state.redundancy_state_dto();
        if !state.active {
            return Err(json_err(409, "not_the_active_controller"));
        }
        let peer = self
            .settings
            .redundancy_settings_dto()
            .peer_device_short_address
            .ok_or_else(|| json_err(422, "peer_not_configured"))?;
        Ok(SwitchoverData {
            peer_short_address: peer,
        })
    }

    fn execute(&self, data: SwitchoverData) -> Result<(), HttpResponse> {
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            Dali103HandoverCommand {
                registry_adapter_id: self.registry_adapter_id,
                peer_short_address: data.peer_short_address,
            },
        );
        crate::http::dispatcher::dispatch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.redundancy_state_dto())
    }
}
