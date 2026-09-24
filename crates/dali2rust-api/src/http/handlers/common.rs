use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_contracts::msg::{ColorMode, DeviceType, OperationType, PowerState};
use dali2rust_domain::registry::OperationReadPort;

use crate::http::physical_device_state::CapabilityFlagsDto;
use crate::http::types::HttpResponse;
use crate::http::virtual_lamp_state::VirtualLampDto;

pub fn config_write_commit_entry(
    correlation_id: u64,
    bus_id: dali2rust_bus::BusId,
    resource: dali2rust_contracts::msg::ConfigWriteResource,
    adapter_id: u8,
    scene_id: Option<u8>,
    chunks: u8,
) -> dali2rust_bus::BusFrame {
    let envelope = dali2rust_contracts::bus::command_envelope(
        crate::bus_codec::SOURCE_ID_UNSPECIFIED,
        correlation_id,
        bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::ConfigWriteCommitCommand {
            resource,
            adapter_id,
            scene_id,
            chunks,
        },
    );
    dali2rust_bus::BusFrame::command(envelope)
}

pub fn publish_matrix_series(
    publisher: &dali2rust_bus::BusPublisher,
    bus_id: dali2rust_bus::BusId,
    workflow: u64,
    resource: dali2rust_contracts::msg::ConfigWriteResource,
    adapter_id: u8,
    scene_id: Option<u8>,
    mut frames: Vec<dali2rust_bus::BusFrame>,
) -> Result<String, HttpResponse> {
    let chunks = u8::try_from(frames.len()).map_err(|_| json_err(422, "invalid_value"))?;
    frames.push(config_write_commit_entry(
        workflow, bus_id, resource, adapter_id, scene_id, chunks,
    ));
    let op_key = match scene_id {
        Some(scene) => format!("cfg-scn-{adapter_id}-{scene}-{workflow}"),
        None => format!("cfg-grp-{adapter_id}-{workflow}"),
    };
    crate::http::handlers::operation_dispatch::publish_begin_then_chunk_series(
        publisher, bus_id, workflow, &op_key, frames,
    )?;
    Ok(op_key)
}

pub const APPLY_WATCH_BUDGET_MS: u64 = 200;
const APPLY_WATCH_POLL: Duration = Duration::from_millis(10);

pub fn serialize_or_log(val: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(val).unwrap_or_else(|e| {
        log::error!("serialize_or_log: failed to serialize response: {e}");
        b"{}".to_vec()
    })
}

pub fn json_stream_dto<T: serde::Serialize + Send + 'static>(dto: T) -> HttpResponse {
    HttpResponse::json_stream(
        200,
        Box::new(move |w: &mut dyn std::io::Write| {
            serde_json::to_writer(w, &dto)
                .map_err(|e| std::io::Error::other(format!("json serialization: {e}")))
        }),
    )
}

pub(crate) fn write_json_array_items<T: serde::Serialize>(
    w: &mut dyn std::io::Write,
    items: impl IntoIterator<Item = T>,
) -> std::io::Result<()> {
    for (idx, item) in items.into_iter().enumerate() {
        if idx > 0 {
            w.write_all(b",")?;
        }
        serde_json::to_writer(&mut *w, &item)?;
    }
    Ok(())
}

pub fn json_err(status: u16, code: &str) -> HttpResponse {
    HttpResponse::json(
        status,
        serde_json::json!({ "error": code })
            .to_string()
            .into_bytes(),
    )
}

pub fn json_err_with_message(status: u16, code: &str, message: &str) -> HttpResponse {
    HttpResponse::json(
        status,
        serde_json::json!({ "error": code, "message": message })
            .to_string()
            .into_bytes(),
    )
}

pub fn accepted_operation_response(
    operation_id: String,
    operation_type: OperationType,
) -> HttpResponse {
    HttpResponse::json(
        202,
        serialize_or_log(&serde_json::json!({
            "operation_id": operation_id,
            "type": operation_type.rest_name(),
            "status": "accepted",
        })),
    )
}

pub fn accepted_config_write_response(operation_id: String, schedule_id: &str) -> HttpResponse {
    HttpResponse::json(
        202,
        serialize_or_log(&serde_json::json!({
            "operation_id": operation_id,
            "type": OperationType::ConfigWrite.rest_name(),
            "status": "accepted",
            "schedule_id": schedule_id,
        })),
    )
}

pub fn accepted_correlation_response(correlation_id: u64) -> HttpResponse {
    HttpResponse::json(
        202,
        serialize_or_log(&serde_json::json!({
            "correlation_id": correlation_id,
            "status": "accepted",
        })),
    )
}

pub fn parse_resource_id_param(
    params: &HashMap<String, String>,
    key: &str,
    max_valid: u16,
) -> Result<u8, HttpResponse> {
    let Some(raw) = params.get(key) else {
        return Err(json_err(400, "missing_resource_id"));
    };
    let Ok(id) = raw.parse::<u16>() else {
        return Err(json_err(400, "invalid_resource_id"));
    };
    if id > max_valid {
        return Err(json_err(400, "invalid_resource_id"));
    }
    Ok(id as u8)
}

pub fn ensure_confirmation_success(body: &[u8]) -> Result<(), HttpResponse> {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Err(json_err(503, "execution_failed"));
    };
    if parsed
        .get("success")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let code = parsed
        .get("error_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let coarse = parsed
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or("execution_failed");
    let message = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if code == "conflict" && message == CONTROLLER_PASSIVE {
        return Err(standby_refusal());
    }
    Err(match code {
        "superseded" => json_err(409, "superseded"),
        "vl_unbound" => json_err(422, "vl_unbound"),
        "confirmation_timeout" => json_err(504, "confirmation_timeout"),
        "conflict" => json_err(409, "conflict"),
        "not_found" => json_err(404, "not_found"),
        "invalid_value" => json_err(422, "invalid_value"),
        "invalid_resource_id" => json_err(400, "invalid_resource_id"),
        "formatter_failed" => json_err(503, "formatter_failed"),
        _ => {
            if coarse == "timeout" {
                json_err(504, "confirmation_timeout")
            } else {
                json_err(503, coarse)
            }
        }
    })
}

const CONTROLLER_PASSIVE: &str = "controller_passive";

const STANDBY_RETRY_AFTER: &[(&str, &str)] = &[
    ("Retry-After", "1"),
    (crate::http::role::ROLE_HEADER, "standby"),
];

#[must_use]
pub fn standby_refusal_headers() -> &'static [(&'static str, &'static str)] {
    STANDBY_RETRY_AFTER
}

fn standby_refusal() -> HttpResponse {
    let mut response = json_err(409, "controller_standby");
    response.extra_headers = STANDBY_RETRY_AFTER;
    response
}

pub fn parse_device_type(s: &str) -> Option<DeviceType> {
    match s {
        "unknown" => Some(DeviceType::Unknown),
        "dt8_color" => Some(DeviceType::Dt8Color),
        "dt6_led" => Some(DeviceType::Dt6Led),
        _ => None,
    }
}

pub fn parse_color_mode(s: &str) -> Option<ColorMode> {
    match s {
        "brightness" => Some(ColorMode::Brightness),
        "cct" => Some(ColorMode::Cct),
        "xy" => Some(ColorMode::Xy),
        "rgb" => Some(ColorMode::Rgb),
        "rgbwaf" => Some(ColorMode::Rgbwaf),
        "unknown" => Some(ColorMode::Unknown),
        _ => None,
    }
}

pub fn parse_power(s: &str) -> Option<PowerState> {
    match s {
        "on" => Some(PowerState::On),
        "off" => Some(PowerState::Off),
        "unknown" => Some(PowerState::Unknown),
        _ => None,
    }
}

pub fn cap_supports_color_mode(cap: &CapabilityFlagsDto, cm: ColorMode) -> bool {
    dali2rust_domain::registry::capability_supports_color_mode(cap.into(), cm)
}

pub fn cap_accepts_color_mode(cap: &CapabilityFlagsDto, cm: ColorMode) -> bool {
    dali2rust_domain::registry::capability_accepts_color_mode(cap.into(), cm)
}

pub fn vl_cap_supports_color_mode(vl: &VirtualLampDto, cm: ColorMode) -> bool {
    cap_supports_color_mode(&vl.capabilities, cm)
}

pub fn pd_cap_supports_color_mode(caps: &CapabilityFlagsDto, cm: ColorMode) -> bool {
    cap_supports_color_mode(caps, cm)
}

const MAX_JSON_BODY_DEPTH: usize = 8;

fn json_depth_exceeds(body: &[u8], max_depth: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &b in body {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max_depth {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

pub fn parse_typed_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, HttpResponse> {
    let body_slice = if body.is_empty() {
        "{}".as_bytes()
    } else {
        body
    };
    if json_depth_exceeds(body_slice, MAX_JSON_BODY_DEPTH) {
        return Err(json_err(400, "invalid_json"));
    }
    serde_json::from_slice(body_slice).map_err(|_| json_err(400, "invalid_json"))
}

pub fn parse_json_body(body: &[u8]) -> Result<serde_json::Value, HttpResponse> {
    parse_typed_body(body)
}

pub fn parse_physical_short(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    let Some(ss) = params.get("short") else {
        return Err(json_err(400, "missing_short_address"));
    };
    let sa = ss
        .parse::<u16>()
        .map_err(|_| json_err(400, "invalid_resource_id"))?;
    if sa > 63 {
        return Err(json_err(400, "invalid_resource_id"));
    }
    Ok(sa as u8)
}

pub fn parse_adapter_route_id(
    adapter_count: u8,
    params: &HashMap<String, String>,
) -> Result<u8, HttpResponse> {
    let Some(id_str) = params.get("id") else {
        return Err(json_err(400, "missing_adapter_id"));
    };
    let Ok(requested) = id_str.parse::<u16>() else {
        return Err(json_err(400, "invalid_resource_id"));
    };
    if requested >= u16::from(adapter_count) {
        return Err(json_err(404, "not_found"));
    }
    Ok(requested as u8)
}

pub fn publish_and_await_apply<F>(
    publisher: &dali2rust_bus::BusPublisher,
    slots: &Arc<crate::confirmation_bridge::PendingConfirmationSlots>,
    correlation_id: u64,
    timeout_ms: u64,
    frame: dali2rust_bus::BusFrame,
    load: F,
) -> Result<(), HttpResponse>
where
    F: Fn() -> u32,
{
    let before = load();
    crate::http::dispatcher::dispatch_and_wait_for_success(
        publisher,
        slots,
        correlation_id,
        timeout_ms,
        frame,
    )?;
    wait_apply_counter(load, before, APPLY_WATCH_BUDGET_MS.min(timeout_ms))
}

pub fn wait_apply_counter<F>(mut load: F, before: u32, budget_ms: u64) -> Result<(), HttpResponse>
where
    F: FnMut() -> u32,
{
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    loop {
        if load() > before {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(json_err(504, "apply_watch_timeout"));
        }
        // sleep-ok: bounded apply-watch read-after-write poll (>= 10 ms step)
        std::thread::sleep(APPLY_WATCH_POLL);
    }
}

pub fn parse_adapter_id(
    adapter_count: u8,
    params: &HashMap<String, String>,
) -> Result<u8, HttpResponse> {
    let Some(id_str) = params.get("adapter_id") else {
        return Err(json_err(400, "missing_adapter_id"));
    };
    let Ok(aid) = id_str.parse::<u16>() else {
        return Err(json_err(400, "invalid_resource_id"));
    };
    if aid >= u16::from(adapter_count) {
        return Err(json_err(404, "not_found"));
    }
    Ok(aid as u8)
}

pub const COMMISSIONING_OPERATION_TYPES: [OperationType; 3] = [
    OperationType::CommissioningIdentify,
    OperationType::CommissioningAddressChange,
    OperationType::CommissioningReplaceDevice,
];

pub fn reject_if_commissioning_active(
    operations: &dyn OperationReadPort,
    adapter_id: u8,
) -> Result<(), HttpResponse> {
    if COMMISSIONING_OPERATION_TYPES
        .iter()
        .any(|op_type| operations.has_active_operation(*op_type, adapter_id))
    {
        return Err(json_err(409, "conflict"));
    }
    Ok(())
}

pub fn reject_if_apply_active(
    operations: &dyn OperationReadPort,
    operation_type: OperationType,
    adapter_id: u8,
) -> Result<(), HttpResponse> {
    if operations.has_active_operation(operation_type, adapter_id) {
        return Err(json_err(409, "conflict"));
    }
    Ok(())
}

pub fn publish_apply_execute(
    publisher: &dali2rust_bus::BusPublisher,
    operation_type: OperationType,
    operation_id: String,
    execute: dali2rust_contracts::msg::CommandEnvelope,
) -> Result<HttpResponse, HttpResponse> {
    if publisher.try_publish(
        dali2rust_bus::BusChannel::Commands,
        dali2rust_bus::BusFrame::command(execute),
    ) != dali2rust_bus::PublishResult::Queued
    {
        return Err(json_err(503, "commands_ingress_overload"));
    }
    Ok(accepted_operation_response(operation_id, operation_type))
}

#[derive(Clone, Copy)]
pub enum MatrixWriteMode {
    Patch,
    Replace,
}

impl MatrixWriteMode {
    pub fn full_replace(self) -> bool {
        matches!(self, Self::Replace)
    }

    pub fn expected_method(self) -> &'static str {
        match self {
            Self::Patch => "PATCH",
            Self::Replace => "PUT",
        }
    }
}

pub fn require_get(method: &str) -> Result<(), HttpResponse> {
    if method != "GET" {
        return Err(HttpResponse::method_not_allowed());
    }
    Ok(())
}

pub fn parse_bool_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, HttpResponse> {
    match obj.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_bool()
            .map(Some)
            .ok_or_else(|| json_err(422, "invalid_value")),
    }
}

pub fn get_adapter_id(
    method: &str,
    adapter_count: u8,
    params: &HashMap<String, String>,
) -> Result<u8, HttpResponse> {
    require_get(method)?;
    parse_adapter_id(adapter_count, params)
}

pub trait MutatingHandler: Send + Sync {
    type Validated;
    type Executed;

    fn expected_method(&self) -> &'static str;
    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Self::Validated, HttpResponse>;
    fn execute(&self, validated: Self::Validated) -> Result<Self::Executed, HttpResponse>;
    fn respond(&self, executed: Self::Executed) -> HttpResponse;
}

impl<H: MutatingHandler> crate::http::handler::ApiHandler for H {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != self.expected_method() {
            return HttpResponse::method_not_allowed();
        }
        let validated = match self.validate(params, body) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let executed = match self.execute(validated) {
            Ok(e) => e,
            Err(resp) => return resp,
        };
        self.respond(executed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(depth: usize) -> Vec<u8> {
        let mut s = String::from("{\"a\":");
        s.push_str(&"[".repeat(depth - 1));
        s.push('1');
        s.push_str(&"]".repeat(depth - 1));
        s.push('}');
        s.into_bytes()
    }

    #[test]
    fn depth_guard_accepts_product_depths() {
        assert!(!json_depth_exceeds(&nested(MAX_JSON_BODY_DEPTH), MAX_JSON_BODY_DEPTH));
        assert!(parse_json_body(&nested(MAX_JSON_BODY_DEPTH)).is_ok());
    }

    #[test]
    fn depth_guard_rejects_over_deep_bodies() {
        assert!(json_depth_exceeds(&nested(MAX_JSON_BODY_DEPTH + 1), MAX_JSON_BODY_DEPTH));
        let resp = parse_json_body(&nested(MAX_JSON_BODY_DEPTH + 1)).unwrap_err();
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn depth_guard_ignores_braces_inside_strings() {
        let body = br#"{"a":"[[[[[[[[[[[[[[[[[[[["}"#;
        assert!(!json_depth_exceeds(body, MAX_JSON_BODY_DEPTH));
        assert!(parse_json_body(body).is_ok());
    }

    #[test]
    fn depth_guard_handles_escaped_quotes_in_strings() {
        let body = br#"{"a":"x\"[[[[[[[[[[\"y"}"#;
        assert!(!json_depth_exceeds(body, MAX_JSON_BODY_DEPTH));
        assert!(parse_json_body(body).is_ok());
    }
}
