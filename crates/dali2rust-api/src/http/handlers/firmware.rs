use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::commands::FirmwareUpdateBeginCommand;
use dali2rust_contracts::msg::{fixed_text_96, OperationType};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::firmware_state::FirmwareHttpState;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{
    accepted_operation_response, json_err, json_stream_dto, parse_json_body, require_get,
};
use crate::http::handlers::operation_dispatch::publish_begin_then_semantic_command_pair;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

const OPERATION_KEY: &str = "firmware-update";

const ALLOWED_SCHEMES: [&str; 2] = ["http://", "https://"];

declare_handler_shell! {
    FirmwareGetHandler {
        state: Arc<dyn FirmwareHttpState>,
    }
}

impl ApiHandler for FirmwareGetHandler {
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
        json_stream_dto(self.state.firmware_dto())
    }
}

declare_handler_shell! {
    FirmwareUpdateHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn FirmwareHttpState>,
        bus_id: BusId,
    }
}

fn parse_url(body: &[u8]) -> Result<String, HttpResponse> {
    let value = parse_json_body(body)?;
    let Some(object) = value.as_object() else {
        return Err(json_err(400, "invalid_body"));
    };
    if object.keys().any(|key| key.as_str() != "url") {
        return Err(json_err(422, "unsupported_field"));
    }
    let Some(url) = object.get("url").and_then(|v| v.as_str()) else {
        return Err(json_err(400, "missing_url"));
    };
    if !ALLOWED_SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
        return Err(json_err(422, "unsupported_scheme"));
    }
    if url.len() > FirmwareUpdateBeginCommand::URL_MAX_BYTES {
        return Err(json_err(422, "url_too_long"));
    }
    Ok(url.to_string())
}

impl FirmwareUpdateHandler {
    fn accept(&self, url: &str) -> Result<HttpResponse, HttpResponse> {
        let state = self.state.firmware_dto();
        if !state.ota_capable {
            return Err(json_err(409, "ota_unsupported"));
        }
        if !matches!(state.update.state, "idle" | "failed") {
            return Err(json_err(409, "update_in_progress"));
        }
        let workflow = self.correlation.next_id();
        let command = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            FirmwareUpdateBeginCommand {
                url: fixed_text_96(url),
            },
        );
        publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            OPERATION_KEY,
            OperationType::FirmwareUpdate,
            command,
        )?;
        Ok(accepted_operation_response(
            OPERATION_KEY.to_string(),
            OperationType::FirmwareUpdate,
        ))
    }
}

impl ApiHandler for FirmwareUpdateHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if !method.eq_ignore_ascii_case("POST") {
            return json_err(405, "method_not_allowed");
        }
        let url = match parse_url(body) {
            Ok(url) => url,
            Err(response) => return response,
        };
        match self.accept(&url) {
            Ok(response) | Err(response) => response,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_the_wire_would_truncate_is_refused_rather_than_clipped() {
        let long = "https://example.com/".to_string()
            + &"a".repeat(FirmwareUpdateBeginCommand::URL_MAX_BYTES);
        let body = serde_json::json!({ "url": long }).to_string();
        let Err(response) = parse_url(body.as_bytes()) else {
            panic!("an over-long URL must be refused, not truncated");
        };
        assert_eq!(response.status, 422);
    }

    #[test]
    fn only_http_and_https_are_fetched() {
        for url in ["file:///tmp/x.bin", "ftp://h/x.bin", "/x.bin"] {
            let body = serde_json::json!({ "url": url }).to_string();
            let Err(response) = parse_url(body.as_bytes()) else {
                panic!("{url} is not fetchable and must be refused");
            };
            assert_eq!(response.status, 422, "{url}");
        }
        let ok = serde_json::json!({ "url": "http://h/x.bin" }).to_string();
        assert_eq!(parse_url(ok.as_bytes()).ok(), Some("http://h/x.bin".to_string()));
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let body = serde_json::json!({ "url": "http://h/x.bin", "sha256": "…" }).to_string();
        let Err(response) = parse_url(body.as_bytes()) else {
            panic!("a field this route does not honour must be named, not ignored");
        };
        assert_eq!(response.status, 422);
    }
}
