use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    fixed_text_32, fixed_text_48, fixed_text_64, HomeAssistantControllerIdUpdateCommand,
    HomeAssistantCredentialsUpdateCommand, HomeAssistantSettingsUpdateCommand,
    HomeAssistantTopicsUpdateCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::home_assistant_settings_state::{
    HomeAssistantSettingsApplyWatch, HomeAssistantSettingsHttpState,
};
use crate::ha::topics::is_topic_safe;
use crate::http::types::HttpResponse;

use super::common::{
    accepted_operation_response, json_err, json_stream_dto, parse_bool_field, parse_json_body,
    require_get,
    wait_apply_counter, APPLY_WATCH_BUDGET_MS,
};
use super::operation_dispatch::publish_begin_then_semantic_command_pair;

const MAX_BROKER_HOST: usize = HomeAssistantSettingsUpdateCommand::BROKER_HOST_MAX_BYTES;
const MAX_BROKER_USERNAME: usize =
    HomeAssistantCredentialsUpdateCommand::BROKER_USERNAME_MAX_BYTES;
const MAX_BROKER_PASSWORD: usize =
    HomeAssistantCredentialsUpdateCommand::BROKER_PASSWORD_MAX_BYTES;
const MAX_PREFIX: usize = HomeAssistantTopicsUpdateCommand::PREFIX_MAX_BYTES;
const MAX_CONTROLLER_ID: usize = HomeAssistantControllerIdUpdateCommand::CONTROLLER_ID_MAX_BYTES;
const MAX_PUBLISH_QOS: u8 = 1;

const READ_ONLY_KEYS: &[&str] = &["broker_password_set", "broker_url_view"];

const WRITABLE_KEYS: &[&str] = &[
    "enabled",
    "broker_host",
    "broker_port",
    "broker_username",
    "broker_password",
    "discovery_prefix",
    "state_topic_prefix",
    "controller_id",
    "publish_qos",
    "retain_state",
    "retain_discovery",
    "expose_input_devices",
];

declare_handler_shell! {
    HomeAssistantSettingsGetHandler {
        state: Arc<dyn HomeAssistantSettingsHttpState>,
    }
}

impl ApiHandler for HomeAssistantSettingsGetHandler {
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
        json_stream_dto(self.state.home_assistant_settings_dto())
    }
}

declare_handler_shell!(HomeAssistantSettingsPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn HomeAssistantSettingsHttpState>,
    apply_watch: Arc<dyn HomeAssistantSettingsApplyWatch>,
    bus_id: BusId,
    timeout_ms: u64,
});

#[derive(Default)]
pub struct HomeAssistantPatchData {
    settings_mask: u8,
    enabled: bool,
    broker_host: String,
    broker_port: u16,
    publish_qos: u8,
    retain_state: bool,
    retain_discovery: bool,
    expose_input_devices: bool,
    credentials_mask: u8,
    broker_username: String,
    broker_password: String,
    topics_mask: u8,
    discovery_prefix: String,
    state_topic_prefix: String,
    controller_id: Option<String>,
}

impl HomeAssistantSettingsPatchHandler {
    fn frame(
        &self,
        payload: impl Into<dali2rust_contracts::msg::BusCommandPayload>,
    ) -> (u64, BusFrame) {
        let corr = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            None,
            payload,
        );
        (corr, BusFrame::command(cmd))
    }

    fn settings_frame(&self, d: &HomeAssistantPatchData) -> Option<(u64, BusFrame)> {
        (d.settings_mask != 0).then(|| {
            self.frame(HomeAssistantSettingsUpdateCommand {
                patch_mask: d.settings_mask,
                enabled: d.enabled,
                broker_host: fixed_text_64(&d.broker_host),
                broker_port: d.broker_port,
                publish_qos: d.publish_qos,
                retain_state: d.retain_state,
                retain_discovery: d.retain_discovery,
                expose_input_devices: d.expose_input_devices,
            })
        })
    }

    fn credentials_frame(&self, d: &HomeAssistantPatchData) -> Option<(u64, BusFrame)> {
        (d.credentials_mask != 0).then(|| {
            self.frame(HomeAssistantCredentialsUpdateCommand {
                patch_mask: d.credentials_mask,
                broker_username: fixed_text_32(&d.broker_username),
                broker_password: fixed_text_48(&d.broker_password),
            })
        })
    }

    fn topics_frame(&self, d: &HomeAssistantPatchData) -> Option<(u64, BusFrame)> {
        (d.topics_mask != 0).then(|| {
            self.frame(HomeAssistantTopicsUpdateCommand {
                patch_mask: d.topics_mask,
                discovery_prefix: fixed_text_32(&d.discovery_prefix),
                state_topic_prefix: fixed_text_32(&d.state_topic_prefix),
            })
        })
    }

    fn controller_id_frame(&self, d: &HomeAssistantPatchData) -> Option<(u64, BusFrame)> {
        d.controller_id.as_ref().map(|id| {
            self.frame(HomeAssistantControllerIdUpdateCommand {
                controller_id: fixed_text_32(id),
            })
        })
    }

    fn build_batch(&self, d: &HomeAssistantPatchData) -> Vec<(u64, BusFrame)> {
        [
            self.settings_frame(d),
            self.credentials_frame(d),
            self.topics_frame(d),
            self.controller_id_frame(d),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

impl crate::http::handlers::common::MutatingHandler for HomeAssistantSettingsPatchHandler {
    type Validated = HomeAssistantPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<HomeAssistantPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        validate_ha_patch_keys(obj)?;
        let mut data = HomeAssistantPatchData::default();
        parse_ha_broker_fields(obj, &mut data)?;
        parse_ha_credentials_fields(obj, &mut data)?;
        parse_ha_topic_fields(obj, &mut data)?;
        Ok(data)
    }

    fn execute(&self, data: HomeAssistantPatchData) -> Result<(), HttpResponse> {
        let batch = self.build_batch(&data);
        if batch.is_empty() {
            return Ok(());
        }
        let published = batch.len() as u32;
        let before = self.apply_watch.home_assistant_settings_applied_load();
        crate::http::dispatcher::publish_batch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            self.timeout_ms,
            batch,
        )?;
        let watch = Arc::clone(&self.apply_watch);
        wait_apply_counter(
            move || watch.home_assistant_settings_applied_load(),
            before.saturating_add(published.saturating_sub(1)),
            APPLY_WATCH_BUDGET_MS.min(self.timeout_ms),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.home_assistant_settings_dto())
    }
}

fn validate_ha_patch_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), HttpResponse> {
    for k in obj.keys() {
        if WRITABLE_KEYS.contains(&k.as_str()) {
            continue;
        }
        if READ_ONLY_KEYS.contains(&k.as_str()) {
            return Err(json_err(422, "unsupported_field"));
        }
        return Err(json_err(400, "unknown_field"));
    }
    Ok(())
}

fn parse_ha_broker_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut HomeAssistantPatchData,
) -> Result<(), HttpResponse> {
    if let Some(v) = parse_bool_field(obj, "enabled")? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_ENABLED;
        data.enabled = v;
    }
    if let Some(v) = parse_text_field(obj, "broker_host", MAX_BROKER_HOST, true)? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_BROKER_HOST;
        data.broker_host = v;
    }
    if let Some(v) = parse_port_field(obj)? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_BROKER_PORT;
        data.broker_port = v;
    }
    if let Some(v) = parse_qos_field(obj)? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_PUBLISH_QOS;
        data.publish_qos = v;
    }
    if let Some(v) = parse_bool_field(obj, "retain_state")? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_RETAIN_STATE;
        data.retain_state = v;
    }
    if let Some(v) = parse_bool_field(obj, "retain_discovery")? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_RETAIN_DISCOVERY;
        data.retain_discovery = v;
    }
    if let Some(v) = parse_bool_field(obj, "expose_input_devices")? {
        data.settings_mask |= HomeAssistantSettingsUpdateCommand::PATCH_EXPOSE_INPUT_DEVICES;
        data.expose_input_devices = v;
    }
    Ok(())
}

fn parse_ha_credentials_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut HomeAssistantPatchData,
) -> Result<(), HttpResponse> {
    if let Some(v) = parse_text_field(obj, "broker_username", MAX_BROKER_USERNAME, true)? {
        data.credentials_mask |= HomeAssistantCredentialsUpdateCommand::PATCH_BROKER_USERNAME;
        data.broker_username = v;
    }
    if let Some(v) = parse_text_field(obj, "broker_password", MAX_BROKER_PASSWORD, true)? {
        data.credentials_mask |= HomeAssistantCredentialsUpdateCommand::PATCH_BROKER_PASSWORD;
        data.broker_password = v;
    }
    Ok(())
}

fn parse_ha_topic_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
    data: &mut HomeAssistantPatchData,
) -> Result<(), HttpResponse> {
    if let Some(v) = parse_text_field(obj, "discovery_prefix", MAX_PREFIX, false)? {
        data.topics_mask |= HomeAssistantTopicsUpdateCommand::PATCH_DISCOVERY_PREFIX;
        data.discovery_prefix = v;
    }
    if let Some(v) = parse_text_field(obj, "state_topic_prefix", MAX_PREFIX, false)? {
        data.topics_mask |= HomeAssistantTopicsUpdateCommand::PATCH_STATE_TOPIC_PREFIX;
        data.state_topic_prefix = v;
    }
    if let Some(v) = parse_text_field(obj, "controller_id", MAX_CONTROLLER_ID, false)? {
        if !is_topic_safe(&v) {
            return Err(json_err(422, "invalid_value"));
        }
        data.controller_id = Some(v);
    }
    Ok(())
}

fn parse_text_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<Option<String>, HttpResponse> {
    let Some(v) = obj.get(key) else {
        return Ok(None);
    };
    let s = v.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
    if s.len() > max_bytes || (!allow_empty && s.is_empty()) {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(Some(s.to_string()))
}

fn parse_port_field(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<u16>, HttpResponse> {
    let Some(v) = obj.get("broker_port") else {
        return Ok(None);
    };
    let n = v
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    Ok(Some(n))
}

fn parse_qos_field(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<u8>, HttpResponse> {
    let Some(v) = obj.get("publish_qos") else {
        return Ok(None);
    };
    let n = v
        .as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| *n <= MAX_PUBLISH_QOS)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    Ok(Some(n))
}

declare_handler_shell! {
    HomeAssistantDiscoveryPublishHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
    }
}

impl ApiHandler for HomeAssistantDiscoveryPublishHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != "POST" {
            return json_err(405, "method_not_allowed");
        }
        let op_serial = self.correlation.next_id();
        let op_key = format!("ha-disc-{op_serial}");
        let workflow = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::HomeAssistantDiscoveryPublishCommand {},
        );
        if let Err(e) = publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            &op_key,
            dali2rust_contracts::msg::OperationType::HaDiscoveryPublish,
            cmd,
        ) {
            return e;
        }
        accepted_operation_response(
            op_key,
            dali2rust_contracts::msg::OperationType::HaDiscoveryPublish,
        )
    }
}
