use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::RedundancySettingsUpdateCommand;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::require_get;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::redundancy_settings_state::{
    RedundancySettingsApplyWatch, RedundancySettingsHttpState, ROLE_PRIMARY, ROLE_STANDBY,
};
use crate::http::types::HttpResponse;

use super::common::{
    json_err, json_stream_dto, parse_bool_field, parse_json_body, publish_and_await_apply,
};

const ALLOWED_KEYS: &[&str] = &[
    "enabled",
    "role",
    "probe_interval_ms",
    "takeover_after_missed",
    "boot_listen_ms",
    "peer_device_short_address",
    "peer_url",
];

const MIN_PROBE_INTERVAL_MS: u64 = 250;
const MAX_PROBE_INTERVAL_MS: u64 = 60_000;
const MIN_TAKEOVER_AFTER_MISSED: u64 = 1;
const MAX_TAKEOVER_AFTER_MISSED: u64 = 5;
const MAX_BOOT_LISTEN_MS: u64 = 120_000;
const MAX_DEVICE_SHORT_ADDRESS: u64 = 63;
const MAX_PEER_URL_LEN: usize = 64;

declare_handler_shell! {
    RedundancySettingsGetHandler {
        state: Arc<dyn RedundancySettingsHttpState>,
    }
}

impl ApiHandler for RedundancySettingsGetHandler {
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
        json_stream_dto(self.state.redundancy_settings_dto())
    }
}

declare_handler_shell!(RedundancySettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn RedundancySettingsHttpState>,
    apply_watch: Arc<dyn RedundancySettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
});

#[derive(Default)]
pub struct RedundancySettingsPatchData {
    patch_mask: u8,
    enabled: bool,
    standby_role: bool,
    probe_interval_ms: u32,
    takeover_after_missed: u8,
    boot_listen_ms: u32,
    peer_device_short_address: u8,
    peer_url: String,
}

fn parse_bounded(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, HttpResponse> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };
    let n = value
        .as_u64()
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if n < min || n > max {
        return Err(json_err(422, "out_of_range"));
    }
    Ok(Some(n))
}

impl crate::http::handlers::common::MutatingHandler for RedundancySettingsPatchHandler {
    type Validated = RedundancySettingsPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<RedundancySettingsPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        for key in obj.keys() {
            if !ALLOWED_KEYS.contains(&key.as_str()) {
                return Err(json_err(400, "unknown_field"));
            }
        }
        let mut data = RedundancySettingsPatchData::default();
        if let Some(value) = parse_bool_field(obj, "enabled")? {
            data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_ENABLED;
            data.enabled = value;
        }
        parse_role(obj, &mut data)?;
        parse_timings(obj, &mut data)?;
        parse_peer(obj, &mut data)?;
        parse_peer_url(obj, &mut data)?;
        Ok(data)
    }

    fn execute(&self, data: RedundancySettingsPatchData) -> Result<(), HttpResponse> {
        if data.patch_mask == 0 {
            return Ok(());
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            RedundancySettingsUpdateCommand {
                patch_mask: data.patch_mask,
                enabled: data.enabled,
                standby_role: data.standby_role,
                probe_interval_ms: data.probe_interval_ms,
                takeover_after_missed: data.takeover_after_missed,
                boot_listen_ms: data.boot_listen_ms,
                peer_device_short_address: data.peer_device_short_address,
                peer_url: dali2rust_contracts::msg::fixed_text_64(&data.peer_url),
            },
        );
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.redundancy_settings_applied_load(),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.redundancy_settings_dto())
    }
}

fn parse_role(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut RedundancySettingsPatchData,
) -> Result<(), HttpResponse> {
    let Some(value) = obj.get("role") else {
        return Ok(());
    };
    let text = value.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
    data.standby_role = match text {
        ROLE_STANDBY => true,
        ROLE_PRIMARY => false,
        _ => return Err(json_err(422, "invalid_value")),
    };
    data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_ROLE;
    Ok(())
}

fn parse_timings(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut RedundancySettingsPatchData,
) -> Result<(), HttpResponse> {
    if let Some(n) = parse_bounded(obj, "probe_interval_ms", MIN_PROBE_INTERVAL_MS, MAX_PROBE_INTERVAL_MS)? {
        data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_PROBE_INTERVAL_MS;
        data.probe_interval_ms = u32::try_from(n).unwrap_or(u32::MAX);
    }
    if let Some(n) = parse_bounded(
        obj,
        "takeover_after_missed",
        MIN_TAKEOVER_AFTER_MISSED,
        MAX_TAKEOVER_AFTER_MISSED,
    )? {
        data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_TAKEOVER_AFTER_MISSED;
        data.takeover_after_missed = u8::try_from(n).unwrap_or(u8::MAX);
    }
    if let Some(n) = parse_bounded(obj, "boot_listen_ms", 0, MAX_BOOT_LISTEN_MS)? {
        data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_BOOT_LISTEN_MS;
        data.boot_listen_ms = u32::try_from(n).unwrap_or(u32::MAX);
    }
    Ok(())
}

fn parse_peer_url(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut RedundancySettingsPatchData,
) -> Result<(), HttpResponse> {
    let Some(value) = obj.get("peer_url") else {
        return Ok(());
    };
    let text = value.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
    if text.len() > MAX_PEER_URL_LEN {
        return Err(json_err(422, "out_of_range"));
    }
    if !text.is_empty() && dali2rust_platform::http_fetch::authority_of(text).is_none() {
        return Err(json_err(422, "invalid_value"));
    }
    data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_PEER_URL;
    data.peer_url = text.to_string();
    Ok(())
}

fn parse_peer(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut RedundancySettingsPatchData,
) -> Result<(), HttpResponse> {
    let Some(value) = obj.get("peer_device_short_address") else {
        return Ok(());
    };
    data.patch_mask |= RedundancySettingsUpdateCommand::PATCH_PEER_SHORT_ADDRESS;
    data.peer_device_short_address = match value {
        serde_json::Value::Null => u8::MAX,
        other => {
            let n = other.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
            if n > MAX_DEVICE_SHORT_ADDRESS {
                return Err(json_err(422, "out_of_range"));
            }
            u8::try_from(n).unwrap_or(u8::MAX)
        }
    };
    Ok(())
}
