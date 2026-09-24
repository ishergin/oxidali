use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{DaliAttributeGroup, PollerSettingsUpdateCommand};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::poller_settings_state::{PollerSettingsApplyWatch, PollerSettingsHttpState};
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_stream_dto, parse_bool_field, parse_json_body, publish_and_await_apply,
};
use crate::http::handlers::common::require_get;

const ALLOWED_KEYS: &[&str] = &[
    "enabled",
    "interval_ms",
    "attribute_groups_default",
    "include_dt8_color",
    "include_energy",
    "include_diagnostics",
    "skip_unbound_virtual_lamps",
];
const MIN_INTERVAL_MS: u32 = 200;
const MAX_INTERVAL_MS: u32 = 3_600_000;

declare_handler_shell! {
    PollerSettingsGetHandler {
        state: Arc<dyn PollerSettingsHttpState>,
    }
}

impl ApiHandler for PollerSettingsGetHandler {
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
        json_stream_dto(self.state.poller_settings_dto())
    }
}

declare_handler_shell!(PollerSettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn PollerSettingsHttpState>,
    apply_watch: Arc<dyn PollerSettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
});

#[derive(Default)]
pub struct PollerSettingsPatchData {
    patch_mask: u8,
    enabled: bool,
    interval_ms: u32,
    attribute_groups_mask: u8,
    include_dt8_color: bool,
    include_energy: bool,
    include_diagnostics: bool,
    skip_unbound_virtual_lamps: bool,
}

impl crate::http::handlers::common::MutatingHandler for PollerSettingsPatchHandler {
    type Validated = PollerSettingsPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<PollerSettingsPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        validate_poller_settings_patch_keys(obj)?;
        parse_poller_settings_patch_fields(obj)
    }

    fn execute(&self, data: PollerSettingsPatchData) -> Result<(), HttpResponse> {
        if data.patch_mask == 0 {
            return Ok(());
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, None, PollerSettingsUpdateCommand { patch_mask: data.patch_mask, enabled: data.enabled, interval_ms: data.interval_ms, attribute_groups_mask: data.attribute_groups_mask, include_dt8_color: data.include_dt8_color, skip_unbound_virtual_lamps: data.skip_unbound_virtual_lamps, include_energy: data.include_energy, include_diagnostics: data.include_diagnostics });
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.poller_settings_applied_load(),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.poller_settings_dto())
    }
}

fn validate_poller_settings_patch_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), HttpResponse> {
    for k in obj.keys() {
        if !ALLOWED_KEYS.contains(&k.as_str()) {
            return Err(json_err(400, "unknown_field"));
        }
    }
    Ok(())
}

fn parse_poller_settings_patch_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<PollerSettingsPatchData, HttpResponse> {
    let mut data = PollerSettingsPatchData::default();
    if let Some(v) = parse_bool_field(obj, "enabled")? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_ENABLED;
        data.enabled = v;
    }
    if let Some(v) = parse_u32_range_field(obj, "interval_ms", MIN_INTERVAL_MS, MAX_INTERVAL_MS)? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_INTERVAL_MS;
        data.interval_ms = v;
    }
    if let Some(v) = parse_attribute_groups_field(obj)? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_ATTRIBUTE_GROUPS_MASK;
        data.attribute_groups_mask = v;
    }
    if let Some(v) = parse_bool_field(obj, "include_dt8_color")? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_INCLUDE_DT8_COLOR;
        data.include_dt8_color = v;
    }
    if let Some(v) = parse_bool_field(obj, "skip_unbound_virtual_lamps")? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_SKIP_UNBOUND_VIRTUAL_LAMPS;
        data.skip_unbound_virtual_lamps = v;
    }
    if let Some(v) = parse_bool_field(obj, "include_energy")? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_INCLUDE_ENERGY;
        data.include_energy = v;
    }
    if let Some(v) = parse_bool_field(obj, "include_diagnostics")? {
        data.patch_mask |= PollerSettingsUpdateCommand::PATCH_INCLUDE_DIAGNOSTICS;
        data.include_diagnostics = v;
    }
    Ok(data)
}

fn parse_u32_range_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    lo: u32,
    hi: u32,
) -> Result<Option<u32>, HttpResponse> {
    let Some(v) = obj.get(key) else {
        return Ok(None);
    };
    let n = v
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if n < lo || n > hi {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(Some(n))
}

fn parse_attribute_groups_field(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<u8>, HttpResponse> {
    let Some(v) = obj.get("attribute_groups_default") else {
        return Ok(None);
    };
    let arr = v.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
    if arr.is_empty() {
        return Err(json_err(422, "invalid_value"));
    }
    let mut mask: u8 = 0;
    for el in arr {
        let s = el.as_str().ok_or_else(|| json_err(422, "invalid_enum"))?;
        let g = DaliAttributeGroup::parse(s).ok_or_else(|| json_err(422, "invalid_enum"))?;
        mask |= g.mask_bit();
    }
    Ok(Some(mask))
}
