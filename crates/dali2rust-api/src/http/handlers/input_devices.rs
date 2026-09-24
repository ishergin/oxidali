use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    Dali103CommissionCommand, Dali103IdentifyCommand, Dali103InstanceConfigureCommand,
    Dali103ScanCommand, InputDeviceMetadataUpdateCommand, InputDeviceNotesUpdateCommand,
    OperationType,
};
use serde_json::Value;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{accepted_operation_response, json_err, json_stream_dto};
use crate::http::handlers::operation_dispatch::publish_begin_then_semantic_command_pair;
use crate::http::input_device_state::InputDeviceHttpState;
use crate::http::types::HttpResponse;

fn path_u8(params: &std::collections::HashMap<String, String>, key: &str) -> Option<u8> {
    params.get(key)?.parse::<u8>().ok()
}

pub const PATCH_EVENT_SCHEME: u16 = 1 << 0;
pub const PATCH_EVENT_FILTER: u16 = 1 << 1;
pub const PATCH_EVENT_PRIORITY: u16 = 1 << 2;
pub const PATCH_INSTANCE_GROUP_0: u16 = 1 << 3;
pub const TIMER_PATCH_BITS: [u16; 4] = [1 << 6, 1 << 7, 1 << 8, 1 << 9];
pub const PATCH_INSTANCE_ENABLED: u16 = 1 << 10;

pub const FB_PATCH_TIMING: u8 = 1 << 0;
pub const FB_PATCH_ACTIVE_BRIGHTNESS: u8 = 1 << 1;
pub const FB_PATCH_ACTIVE_COLOUR: u8 = 1 << 2;
pub const FB_PATCH_INACTIVE_BRIGHTNESS: u8 = 1 << 3;
pub const FB_PATCH_INACTIVE_COLOUR: u8 = 1 << 4;

const PATCH_NAME: u8 = 1 << 0;
const PATCH_HA_EXPOSE: u8 = 1 << 1;
const PATCH_CLEAR_NAME: u8 = 1 << 2;
const PATCH_FORGET: u8 = 1 << 3;

// IEC 62386-103 §9.4.1
const RESERVED_EVENT_PRIORITY: u64 = 2;

pub struct InputDeviceListHandler {
    state: Arc<dyn InputDeviceHttpState>,
}

impl InputDeviceListHandler {
    pub fn new(state: Arc<dyn InputDeviceHttpState>) -> Self {
        Self { state }
    }
}

impl ApiHandler for InputDeviceListHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        params: &std::collections::HashMap<String, String>,
    ) -> HttpResponse {
        let Some(adapter_id) = path_u8(params, "adapter_id") else {
            return json_err(400, "invalid_resource_id");
        };
        json_stream_dto(serde_json::json!({
            "input_devices": self.state.list(adapter_id),
        }))
    }
}

pub struct InputDeviceGetHandler {
    state: Arc<dyn InputDeviceHttpState>,
    wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
}

impl InputDeviceGetHandler {
    pub fn new(
        state: Arc<dyn InputDeviceHttpState>,
        wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
    ) -> Self {
        Self { state, wall }
    }
}

impl ApiHandler for InputDeviceGetHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        params: &std::collections::HashMap<String, String>,
    ) -> HttpResponse {
        let (Some(adapter_id), Some(short_address)) =
            (path_u8(params, "adapter_id"), path_u8(params, "short_address"))
        else {
            return json_err(400, "invalid_resource_id");
        };
        match self.state.detail(adapter_id, short_address) {
            Some(mut dto) => {
                dto.now_ms = self.wall.unix_millis();
                json_stream_dto(dto)
            }
            None => json_err(404, "input_device_not_found"),
        }
    }
}

pub struct InputDeviceActionHandler {
    publisher: BusPublisher,
    bus_id: BusId,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    state: Arc<dyn InputDeviceHttpState>,
    action: InputDeviceAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputDeviceAction {
    Scan,
    Commission,
    Identify,
    ConfigureInstance,
    ConfigureFeedback,
    PatchMetadata,
    Forget,
}

impl InputDeviceActionHandler {
    pub fn new(
        publisher: BusPublisher,
        bus_id: BusId,
        correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
        state: Arc<dyn InputDeviceHttpState>,
        action: InputDeviceAction,
    ) -> Self {
        Self {
            publisher,
            bus_id,
            correlation,
            state,
            action,
        }
    }

    fn operation(&self, key: String, semantic: dali2rust_contracts::msg::CommandEnvelope,
                 operation_type: OperationType) -> HttpResponse {
        let workflow = semantic.meta.correlation_id;
        if let Err(response) = publish_begin_then_semantic_command_pair(
            &self.publisher,
            self.bus_id,
            workflow,
            &key,
            operation_type,
            semantic,
        ) {
            return response;
        }
        accepted_operation_response(key, operation_type)
    }
}

impl ApiHandler for InputDeviceActionHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        body: &[u8],
        params: &std::collections::HashMap<String, String>,
    ) -> HttpResponse {
        let Some(adapter_id) = path_u8(params, "adapter_id") else {
            return json_err(400, "invalid_resource_id");
        };
        match self.action {
            InputDeviceAction::Scan => self.scan(adapter_id),
            InputDeviceAction::Commission => self.commission(adapter_id, body),
            InputDeviceAction::Identify => self.per_device(adapter_id, params, body),
            InputDeviceAction::ConfigureInstance => self.configure(adapter_id, params, body),
            InputDeviceAction::ConfigureFeedback => {
                self.configure_feedback(adapter_id, params, body)
            }
            InputDeviceAction::PatchMetadata => self.patch_metadata(adapter_id, params, body),
            InputDeviceAction::Forget => self.forget(adapter_id, params),
        }
    }
}

impl InputDeviceActionHandler {
    fn correlation(&self) -> u64 {
        self.correlation.next_id()
    }

    fn scan(&self, adapter_id: u8) -> HttpResponse {
        let corr = self.correlation();
        let semantic = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            Dali103ScanCommand {
                registry_adapter_id: adapter_id,
            },
        );
        self.operation(
            format!("inp-scan-{adapter_id}-{corr}"),
            semantic,
            OperationType::Discovery,
        )
    }

    fn commission(&self, adapter_id: u8, body: &[u8]) -> HttpResponse {
        let include_addressed = match serde_json::from_slice::<Value>(body) {
            Ok(json) => json
                .get("include_addressed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            Err(_) if body.is_empty() => false,
            Err(_) => return json_err(400, "invalid_json"),
        };
        let corr = self.correlation();
        let semantic = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            Dali103CommissionCommand {
                registry_adapter_id: adapter_id,
                include_addressed,
            },
        );
        self.operation(
            format!("inp-comm-{adapter_id}-{corr}"),
            semantic,
            OperationType::CommissioningAddressChange,
        )
    }

    fn per_device(
        &self,
        adapter_id: u8,
        params: &std::collections::HashMap<String, String>,
        _body: &[u8],
    ) -> HttpResponse {
        let Some(short_address) = path_u8(params, "short_address") else {
            return json_err(400, "invalid_resource_id");
        };
        if self.state.detail(adapter_id, short_address).is_none() {
            return json_err(404, "input_device_not_found");
        }
        let corr = self.correlation();
        let semantic = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            Dali103IdentifyCommand {
                registry_adapter_id: adapter_id,
                short_address,
            },
        );
        self.operation(
            format!("inp-id-{adapter_id}-{short_address}-{corr}"),
            semantic,
            OperationType::CommissioningIdentify,
        )
    }

    fn configure(
        &self,
        adapter_id: u8,
        params: &std::collections::HashMap<String, String>,
        body: &[u8],
    ) -> HttpResponse {
        let (Some(short_address), Some(instance_number)) = (
            path_u8(params, "short_address"),
            path_u8(params, "instance_number"),
        ) else {
            return json_err(400, "invalid_resource_id");
        };
        let Some(device) = self.state.detail(adapter_id, short_address) else {
            return json_err(404, "input_device_not_found");
        };
        if instance_number >= device.summary.instance_count {
            return json_err(404, "instance_not_found");
        }
        let cmd = match parse_instance_patch(body, adapter_id, short_address, instance_number) {
            Ok(cmd) => cmd,
            Err(response) => return response,
        };
        let corr = self.correlation();
        let semantic = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            cmd,
        );
        self.operation(
            format!("inp-cfg-{adapter_id}-{short_address}-{instance_number}"),
            semantic,
            OperationType::ConfigWrite,
        )
    }

    fn configure_feedback(
        &self,
        adapter_id: u8,
        params: &std::collections::HashMap<String, String>,
        body: &[u8],
    ) -> HttpResponse {
        let (Some(short_address), Some(instance_number)) = (
            path_u8(params, "short_address"),
            path_u8(params, "instance_number"),
        ) else {
            return json_err(400, "invalid_resource_id");
        };
        let Some(device) = self.state.detail(adapter_id, short_address) else {
            return json_err(404, "input_device_not_found");
        };
        if instance_number >= device.summary.instance_count {
            return json_err(404, "instance_not_found");
        }
        let opcode_map = match feedback_dialect(&device, instance_number) {
            Ok(map) => map,
            Err(response) => return response,
        };
        let cmd =
            match parse_feedback_patch(body, adapter_id, short_address, instance_number, opcode_map)
            {
                Ok(cmd) => cmd,
                Err(response) => return response,
            };
        let corr = self.correlation();
        let semantic = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            cmd,
        );
        self.operation(
            format!("inp-fb-{adapter_id}-{short_address}-{instance_number}"),
            semantic,
            OperationType::ConfigWrite,
        )
    }

    fn patch_metadata(
        &self,
        adapter_id: u8,
        params: &std::collections::HashMap<String, String>,
        body: &[u8],
    ) -> HttpResponse {
        let Some(short_address) = path_u8(params, "short_address") else {
            return json_err(400, "invalid_resource_id");
        };
        if self.state.detail(adapter_id, short_address).is_none() {
            return json_err(404, "input_device_not_found");
        }
        let (meta, notes) = match parse_metadata_patch(body, adapter_id, short_address) {
            Ok(parsed) => parsed,
            Err(response) => return response,
        };
        let before = self.state.revision();
        if meta.patch_mask != 0 {
            self.publish(meta);
        }
        if let Some(notes) = notes {
            self.publish(notes);
        }
        self.wait_for_revision(before);
        match self.state.detail(adapter_id, short_address) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "input_device_not_found"),
        }
    }

    fn forget(
        &self,
        adapter_id: u8,
        params: &std::collections::HashMap<String, String>,
    ) -> HttpResponse {
        let Some(short_address) = path_u8(params, "short_address") else {
            return json_err(400, "invalid_resource_id");
        };
        if self.state.detail(adapter_id, short_address).is_none() {
            return json_err(404, "input_device_not_found");
        }
        let before = self.state.revision();
        self.publish(dali2rust_contracts::msg::InputDeviceMetadataUpdateCommand {
            registry_adapter_id: adapter_id,
            short_address,
            patch_mask: PATCH_FORGET,
            name: dali2rust_contracts::msg::fixed_text_64(""),
            ha_expose: false,
        });
        self.wait_for_revision(before);
        HttpResponse::json(200, br#"{"forgotten":true}"#.to_vec())
    }

    fn publish<P>(&self, payload: P)
    where
        dali2rust_contracts::msg::BusCommandPayload: From<P>,
    {
        let env = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            self.correlation(),
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            payload,
        );
        let _ = self.publisher.try_publish(
            dali2rust_bus::BusChannel::Commands,
            dali2rust_bus::BusFrame::command(env),
        );
    }

    fn wait_for_revision(&self, before: u32) {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(METADATA_APPLY_BUDGET_MS);
        while std::time::Instant::now() < deadline {
            if self.state.revision() != before {
                return;
            }
            // sleep-ok: bounded apply-watch read-after-write poll (>= 10 ms step)
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

const METADATA_APPLY_BUDGET_MS: u64 = 200;

fn parse_instance_patch(
    body: &[u8],
    adapter_id: u8,
    short_address: u8,
    instance_number: u8,
) -> Result<Dali103InstanceConfigureCommand, HttpResponse> {
    let json: Value =
        serde_json::from_slice(body).map_err(|_| json_err(400, "invalid_json"))?;
    let mut cmd = Dali103InstanceConfigureCommand {
        registry_adapter_id: adapter_id,
        short_address,
        instance_number,
        patch_mask: 0,
        event_scheme: 0,
        event_filter: [0; 3],
        event_priority: 0,
        instance_groups: [None; 3],
        timer_multipliers: [None; 4],
        instance_enabled: false,
    };
    parse_scheme_and_priority(&json, &mut cmd)?;
    parse_filter_and_groups(&json, &mut cmd)?;
    parse_timers(&json, &mut cmd)?;
    if cmd.patch_mask == 0 {
        return Err(json_err(400, "empty_patch"));
    }
    Ok(cmd)
}

fn parse_timers(
    json: &Value,
    cmd: &mut Dali103InstanceConfigureCommand,
) -> Result<(), HttpResponse> {
    let Some(timers) = json.get("timers") else {
        return Ok(());
    };
    let timers = timers.as_object().ok_or_else(|| json_err(422, "invalid_value"))?;
    const SPECS: [(&str, usize, u64, u64, u64, bool); 4] = [
        ("t_short_ms", 0, 20, 1, 255, false),
        ("t_double_ms", 1, 20, 5, 100, true),
        ("t_repeat_ms", 2, 20, 5, 100, false),
        ("t_stuck_s", 3, 1, 5, 255, false),
    ];
    for (key, slot, unit, min_units, max_units, zero_ok) in SPECS {
        let Some(value) = timers.get(key) else { continue };
        let raw = value.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        let units = if raw == 0 && zero_ok {
            0
        } else {
            if raw % unit != 0 {
                return Err(json_err(422, "invalid_timer_range"));
            }
            let units = raw / unit;
            if !(min_units..=max_units).contains(&units) {
                return Err(json_err(422, "invalid_timer_range"));
            }
            units
        };
        cmd.timer_multipliers[slot] = Some(u8::try_from(units).unwrap_or(u8::MAX));
        cmd.patch_mask |= TIMER_PATCH_BITS[slot];
    }
    Ok(())
}

fn feedback_dialect(
    device: &crate::http::input_device_state::InputDeviceDto,
    instance_number: u8,
) -> Result<u8, HttpResponse> {
    let Some(instance) = device
        .instances
        .iter()
        .find(|i| i.instance_number == instance_number)
    else {
        return Ok(0);
    };
    match (instance.feedback.probed, instance.feedback.opcode_map) {
        (true, None) => Err(json_err(422, "feedback_not_supported")),
        (_, Some("ed1")) => Ok(2),
        (_, Some(_)) => Ok(1),
        (false, None) => Ok(0),
    }
}

fn parse_feedback_patch(
    body: &[u8],
    adapter_id: u8,
    short_address: u8,
    instance_number: u8,
    opcode_map: u8,
) -> Result<dali2rust_contracts::msg::Dali103FeedbackConfigureCommand, HttpResponse> {
    let json: Value = serde_json::from_slice(body).map_err(|_| json_err(400, "invalid_json"))?;
    let mut cmd = dali2rust_contracts::msg::Dali103FeedbackConfigureCommand {
        registry_adapter_id: adapter_id,
        short_address,
        instance_number,
        patch_mask: 0,
        timing: 0,
        active_brightness: 0,
        active_colour: 0,
        inactive_brightness: 0,
        inactive_colour: 0,
        opcode_map,
    };
    parse_feedback_fields(&json, &mut cmd)?;
    if cmd.patch_mask == 0 {
        return Err(json_err(400, "empty_patch"));
    }
    Ok(cmd)
}

fn parse_feedback_fields(
    json: &Value,
    cmd: &mut dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
) -> Result<(), HttpResponse> {
    const FIELDS: [(&str, u8, bool); 5] = [
        ("timing", FB_PATCH_TIMING, false),
        ("active_brightness", FB_PATCH_ACTIVE_BRIGHTNESS, false),
        ("active_colour", FB_PATCH_ACTIVE_COLOUR, true),
        ("inactive_brightness", FB_PATCH_INACTIVE_BRIGHTNESS, false),
        ("inactive_colour", FB_PATCH_INACTIVE_COLOUR, true),
    ];
    let slots = |cmd: &mut dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
                 key: &str,
                 value: u8| match key {
        "timing" => cmd.timing = value,
        "active_brightness" => cmd.active_brightness = value,
        "active_colour" => cmd.active_colour = value,
        "inactive_brightness" => cmd.inactive_brightness = value,
        _ => cmd.inactive_colour = value,
    };
    for (key, bit, is_colour) in FIELDS {
        let Some(value) = json.get(key) else { continue };
        let raw = value.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        let value = u8::try_from(raw).map_err(|_| json_err(422, "invalid_value"))?;
        if is_colour && !(FEEDBACK_COLOUR_RANGE).contains(&value) {
            return Err(json_err(422, "invalid_feedback_colour"));
        }
        slots(cmd, key, value);
        cmd.patch_mask |= bit;
    }
    Ok(())
}

// IEC 62386-332 §9.5.3
const FEEDBACK_COLOUR_RANGE: std::ops::RangeInclusive<u8> = 1..=63;

fn parse_scheme_and_priority(
    json: &Value,
    cmd: &mut Dali103InstanceConfigureCommand,
) -> Result<(), HttpResponse> {
    if let Some(scheme) = json.get("event_scheme") {
        let scheme = scheme.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        if scheme > 4 {
            return Err(json_err(422, "invalid_value"));
        }
        cmd.event_scheme = u8::try_from(scheme).unwrap_or(0);
        cmd.patch_mask |= PATCH_EVENT_SCHEME;
    }
    if let Some(priority) = json.get("event_priority") {
        let priority = priority.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        if !(2..=5).contains(&priority) || priority == RESERVED_EVENT_PRIORITY {
            return Err(json_err(422, "invalid_value"));
        }
        cmd.event_priority = u8::try_from(priority).unwrap_or(3);
        cmd.patch_mask |= PATCH_EVENT_PRIORITY;
    }
    if let Some(enabled) = json.get("enabled") {
        cmd.instance_enabled = enabled.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
        cmd.patch_mask |= PATCH_INSTANCE_ENABLED;
    }
    Ok(())
}

fn parse_filter_and_groups(
    json: &Value,
    cmd: &mut Dali103InstanceConfigureCommand,
) -> Result<(), HttpResponse> {
    if let Some(filter) = json.get("event_filter") {
        let bytes = filter.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
        if bytes.len() > 3 {
            return Err(json_err(422, "invalid_value"));
        }
        for (slot, byte) in bytes.iter().enumerate() {
            let value = byte.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
            cmd.event_filter[slot] =
                u8::try_from(value).map_err(|_| json_err(422, "invalid_value"))?;
        }
        cmd.patch_mask |= PATCH_EVENT_FILTER;
    }
    if let Some(groups) = json.get("instance_groups") {
        let groups = groups.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
        for (slot, group) in groups.iter().take(3).enumerate() {
            cmd.instance_groups[slot] = parse_group(group)?;
            cmd.patch_mask |= PATCH_INSTANCE_GROUP_0 << slot;
        }
    }
    Ok(())
}

fn parse_group(group: &Value) -> Result<Option<u8>, HttpResponse> {
    match group {
        Value::Null => Ok(None),
        other => {
            let value = other.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
            if value > 31 {
                return Err(json_err(422, "invalid_value"));
            }
            Ok(Some(u8::try_from(value).unwrap_or(0)))
        }
    }
}

fn parse_metadata_patch(
    body: &[u8],
    adapter_id: u8,
    short_address: u8,
) -> Result<(InputDeviceMetadataUpdateCommand, Option<InputDeviceNotesUpdateCommand>), HttpResponse>
{
    let json: Value =
        serde_json::from_slice(body).map_err(|_| json_err(400, "invalid_json"))?;
    let mut cmd = InputDeviceMetadataUpdateCommand {
        registry_adapter_id: adapter_id,
        short_address,
        patch_mask: 0,
        name: dali2rust_contracts::msg::fixed_text_64(""),
        ha_expose: true,
    };
    match json.get("name") {
        Some(Value::Null) => cmd.patch_mask |= PATCH_CLEAR_NAME,
        Some(Value::String(name)) => {
            cmd.name = dali2rust_contracts::msg::fixed_text_64(name);
            cmd.patch_mask |= PATCH_NAME;
        }
        Some(_) => return Err(json_err(422, "invalid_value")),
        None => {}
    }
    if let Some(expose) = json.get("ha_expose") {
        cmd.ha_expose = expose.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
        cmd.patch_mask |= PATCH_HA_EXPOSE;
    }
    let notes = match json.get("notes") {
        Some(Value::Null) => Some(InputDeviceNotesUpdateCommand {
            registry_adapter_id: adapter_id,
            short_address,
            notes: dali2rust_contracts::msg::fixed_text_48(""),
        }),
        Some(Value::String(text)) => Some(InputDeviceNotesUpdateCommand {
            registry_adapter_id: adapter_id,
            short_address,
            notes: dali2rust_contracts::msg::fixed_text_48(text),
        }),
        Some(_) => return Err(json_err(422, "invalid_value")),
        None => None,
    };
    if cmd.patch_mask == 0 && notes.is_none() {
        return Err(json_err(400, "empty_patch"));
    }
    Ok((cmd, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T>(result: Result<T, HttpResponse>) -> T {
        match result {
            Ok(value) => value,
            Err(_) => panic!("parser refused a body the test considers valid"),
        }
    }

    #[test]
    fn event_priority_two_is_refused_at_ingress() {
        let body = br#"{"event_priority":2}"#;
        let outcome = parse_instance_patch(body, 0, 3, 0);
        assert!(outcome.is_err(), "priority 2 must not reach the wire");

        let ok = parse_instance_patch(br#"{"event_priority":3}"#, 0, 3, 0);
        assert!(ok.is_ok());
    }

    #[test]
    fn an_event_scheme_above_four_is_refused() {
        assert!(parse_instance_patch(br#"{"event_scheme":5}"#, 0, 3, 0).is_err());
        assert!(parse_instance_patch(br#"{"event_scheme":2}"#, 0, 3, 0).is_ok());
    }

    #[test]
    fn an_instance_group_above_thirty_one_is_refused() {
        assert!(parse_instance_patch(br#"{"instance_groups":[32]}"#, 0, 3, 0).is_err());
        let parsed = ok(parse_instance_patch(
            br#"{"instance_groups":[4,null,null]}"#,
            0,
            3,
            0,
        ));
        assert_eq!(parsed.instance_groups[0], Some(4));
        assert_eq!(parsed.instance_groups[1], None);
    }

    #[test]
    fn an_empty_patch_is_a_client_error_not_a_no_op_write() {
        assert!(parse_instance_patch(b"{}", 0, 3, 0).is_err());
        assert!(parse_metadata_patch(b"{}", 0, 3).is_err());
    }

    #[test]
    fn notes_ride_their_own_command_because_of_the_wire_budget() {
        let (meta, notes) = ok(parse_metadata_patch(
            br#"{"name":"panel","notes":"hall"}"#,
            0,
            3,
        ));
        assert_eq!(meta.patch_mask & PATCH_NAME, PATCH_NAME);
        let notes = notes.expect("notes command");
        assert_eq!(notes.notes.as_str(), "hall");
    }

    #[test]
    fn a_null_name_clears_rather_than_sets_empty() {
        let (meta, _) = ok(parse_metadata_patch(br#"{"name":null}"#, 0, 3));
        assert_eq!(meta.patch_mask & PATCH_CLEAR_NAME, PATCH_CLEAR_NAME);
        assert_eq!(meta.patch_mask & PATCH_NAME, 0);
    }
}
