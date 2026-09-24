use std::collections::HashMap;
use dali2rust_contracts::msg::{AdapterSettingsUpdateCommand};
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusPublisher};

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::adapter_state::{AdapterHttpState, AdapterSettingsApplyWatch, AdaptersListBody};
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_stream_dto, parse_adapter_route_id, parse_json_body,
    publish_and_await_apply,
};
use crate::http::handlers::common::{require_get};

declare_handler_shell! {
    AdaptersListHandler {
        state: Arc<dyn AdapterHttpState>,
    }
}

impl ApiHandler for AdaptersListHandler {
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
        let body = AdaptersListBody {
            adapters: self.state.list_adapter_dtos(),
        };
        json_stream_dto(body)
    }
}

declare_handler_shell! {
    AdapterGetHandler {
        state: Arc<dyn AdapterHttpState>,
    }
}

impl ApiHandler for AdapterGetHandler {
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
        let id = match parse_adapter_route_id(self.state.adapter_count(), params) {
            Ok(id) => id,
            Err(e) => return e,
        };
        let Some(dto) = self.state.adapter_dto(id) else {
            return json_err(404, "not_found");
        };
        json_stream_dto(dto)
    }
}

declare_handler_shell!(AdapterPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn AdapterHttpState>,
    apply_watch: Arc<dyn AdapterSettingsApplyWatch>,
    timeout_ms: u64,
});

pub struct AdapterPatchData {
    adapter_id: u8,
    patch_mask: u8,
    name: Option<String>,
    enabled: Option<bool>,
}

impl crate::http::handlers::common::MutatingHandler for AdapterPatchHandler {
    type Validated = AdapterPatchData;
    type Executed = u8;

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<AdapterPatchData, HttpResponse> {
        let adapter_id = parse_adapter_route_id(self.state.adapter_count(), params)?;
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        validate_adapter_patch_keys(obj)?;
        parse_adapter_patch_fields(adapter_id, obj)
    }

    fn execute(&self, data: AdapterPatchData) -> Result<u8, HttpResponse> {
        if data.patch_mask == 0 {
            return Ok(data.adapter_id);
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, u16::from(data.adapter_id), None, dali2rust_contracts::msg::AdapterSettingsUpdateCommand { patch_mask: data.patch_mask, name: dali2rust_contracts::msg::fixed_text_64((data.name.as_deref()).unwrap_or("")), enabled: data.enabled.unwrap_or(false) });
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.adapter_settings_applied_load(),
        )?;
        Ok(data.adapter_id)
    }

    fn respond(&self, adapter_id: u8) -> HttpResponse {
        let Some(dto) = self.state.adapter_dto(adapter_id) else {
            return json_err(404, "not_found");
        };
        json_stream_dto(dto)
    }
}

fn validate_adapter_patch_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), HttpResponse> {
    const RO: &[&str] = &["limits", "bus_status", "counters", "adapter_id"];
    for k in obj.keys() {
        if k == "name" || k == "enabled" {
            continue;
        }
        if RO.iter().any(|r| *r == k.as_str()) {
            return Err(json_err(422, "unsupported_field"));
        }
        return Err(json_err(400, "unknown_field"));
    }
    Ok(())
}

fn parse_adapter_patch_fields(
    adapter_id: u8,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<AdapterPatchData, HttpResponse> {
    let mut patch_mask: u8 = 0;
    let mut name: Option<String> = None;
    let mut enabled_val: Option<bool> = None;

    if let Some(nv) = obj.get("name") {
        let s = match nv.as_str() {
            Some(s) => s.to_string(),
            None => return Err(json_err(422, "invalid_value")),
        };
        if s.is_empty() || s.as_bytes().len() > 64 {
            return Err(json_err(422, "invalid_value"));
        }
        patch_mask |= AdapterSettingsUpdateCommand::PATCH_NAME;
        name = Some(s);
    }
    if let Some(ev) = obj.get("enabled") {
        let b = match ev.as_bool() {
            Some(b) => b,
            None => return Err(json_err(422, "invalid_value")),
        };
        patch_mask |= AdapterSettingsUpdateCommand::PATCH_ENABLED;
        enabled_val = Some(b);
    }

    Ok(AdapterPatchData {
        adapter_id,
        patch_mask,
        name,
        enabled: enabled_val,
    })
}
