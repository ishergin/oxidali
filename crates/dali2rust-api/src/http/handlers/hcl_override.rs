use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::BusFrame;
use dali2rust_contracts::msg::HclOverrideClearCommand;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::HclOverrideReadPort;

use crate::http::dispatcher::dispatch_and_wait_for_success;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{json_err, json_stream_dto};
use crate::http::handlers::hcl::HclBusContext;
use crate::http::handlers::hcl_validate::validate_schedule_id;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::hcl_state::{hcl_override_view_to_dto, HclScheduleHttpState};
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{require_get};

fn known_schedule_id(
    state: &Arc<dyn HclScheduleHttpState>,
    params: &HashMap<String, String>,
) -> Result<String, HttpResponse> {
    let Some(raw) = params.get("schedule_id") else {
        return Err(json_err(400, "missing_resource_id"));
    };
    validate_schedule_id(raw)?;
    if state.hcl_schedule_dto(raw).is_none() {
        return Err(json_err(404, "not_found"));
    }
    Ok(raw.clone())
}

declare_handler_shell! {
    HclOverrideGetHandler {
        state: Arc<dyn HclScheduleHttpState>,
        overrides: Arc<dyn HclOverrideReadPort>,
    }
}

impl ApiHandler for HclOverrideGetHandler {
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
        let schedule_id = match known_schedule_id(&self.state, params) {
            Ok(id) => id,
            Err(response) => return response,
        };
        json_stream_dto(hcl_override_view_to_dto(
            self.overrides.hcl_override_view(&schedule_id),
        ))
    }
}

declare_handler_shell! {
    HclOverrideDeleteHandler {
        state: Arc<dyn HclScheduleHttpState>,
        bus: HclBusContext,
    }
}

impl ApiHandler for HclOverrideDeleteHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let schedule_id = match known_schedule_id(&self.state, params) {
            Ok(id) => id,
            Err(response) => return response,
        };
        let correlation_id = self.bus.correlation.next_id();
        let command = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            HclOverrideClearCommand {
                schedule_id: dali2rust_contracts::msg::fixed_text_32(&schedule_id),
            },
        );
        if let Err(response) = dispatch_and_wait_for_success(
            &self.bus.publisher,
            &self.bus.slots,
            correlation_id,
            self.bus.timeout_ms,
            BusFrame::command(command),
        ) {
            return response;
        }
        HttpResponse::no_content()
    }
}
