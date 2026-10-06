use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    Dali103CommissionCommand, Dali103IdentifyCommand, Dali103InstanceConfigureCommand,
    Dali103FeedbackConfigureCommand, Dali103ScanCommand, FeedbackPatchField,
    InputDeviceMetadataUpdateCommand, InputDeviceNotesUpdateCommand, InstancePatchField,
    OperationBeginCommand, OperationType,
};
use serde_json::Value;

use crate::http::handler::ApiHandler;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::publish_batch_and_wait_for_success;
use crate::http::handlers::common::{
    accepted_operation_response, json_err, json_err_with_message, json_stream_dto,
    check_body_keys, parse_adapter_id, parse_body_object, parse_resource_id_param,
    MAX_SHORT_ADDRESS,
};
use crate::http::handlers::operation_dispatch::publish_begin_then_semantic_command;
use crate::http::input_device_state::{InputDeviceDto, InputDeviceHttpState};
use crate::http::types::HttpResponse;

type PathParams = std::collections::HashMap<String, String>;

fn path_u8(params: &PathParams, key: &str) -> Option<u8> {
    params.get(key)?.parse::<u8>().ok()
}

fn device_path(params: &PathParams) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, "short_address", u16::from(MAX_SHORT_ADDRESS))
}

fn adapter_path(state: &dyn InputDeviceHttpState, params: &PathParams) -> Result<u8, HttpResponse> {
    parse_adapter_id(state.adapter_count(), params)
}

const PATCH_NAME: u8 = InputDeviceMetadataUpdateCommand::PATCH_NAME;
const PATCH_HA_EXPOSE: u8 = InputDeviceMetadataUpdateCommand::PATCH_HA_EXPOSE;
const PATCH_CLEAR_NAME: u8 = InputDeviceMetadataUpdateCommand::PATCH_CLEAR_NAME;

// IEC 62386-103 §9.4.1
const RESERVED_EVENT_PRIORITY: u64 = 2;

pub struct InputDeviceListHandler {
    state: Arc<dyn InputDeviceHttpState>,
    wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
}

impl InputDeviceListHandler {
    pub fn new(
        state: Arc<dyn InputDeviceHttpState>,
        wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
    ) -> Self {
        Self { state, wall }
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
        let adapter_id = match adapter_path(self.state.as_ref(), params) {
            Ok(adapter_id) => adapter_id,
            Err(response) => return response,
        };
        json_stream_dto(serde_json::json!({
            "now_ms": self.wall.unix_millis(),
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
        let path = adapter_path(self.state.as_ref(), params)
            .and_then(|adapter_id| Ok((adapter_id, device_path(params)?)));
        let (adapter_id, short_address) = match path {
            Ok(path) => path,
            Err(response) => return response,
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

pub struct InputDeviceBus {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub slots: Arc<PendingConfirmationSlots>,
    pub timeout_ms: u64,
    pub wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
}

pub struct InputDeviceActionHandler {
    publisher: BusPublisher,
    bus_id: BusId,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    slots: Arc<PendingConfirmationSlots>,
    timeout_ms: u64,
    wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
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
        bus: InputDeviceBus,
        state: Arc<dyn InputDeviceHttpState>,
        action: InputDeviceAction,
    ) -> Self {
        Self {
            publisher: bus.publisher,
            bus_id: bus.bus_id,
            correlation: bus.correlation,
            slots: bus.slots,
            timeout_ms: bus.timeout_ms,
            wall: bus.wall,
            state,
            action,
        }
    }

    fn operation(&self, key: String, semantic: dali2rust_contracts::msg::CommandEnvelope,
                 operation_type: OperationType) -> HttpResponse {
        let begin = OperationBeginCommand::with_defaults(&key, operation_type);
        self.begin_then(key, semantic, begin)
    }

    fn wire_config_write(&self, key: String, semantic: dali2rust_contracts::msg::CommandEnvelope) -> HttpResponse {
        let begin = OperationBeginCommand::wire_config_write(&key);
        self.begin_then(key, semantic, begin)
    }

    fn begin_then(&self, key: String, semantic: dali2rust_contracts::msg::CommandEnvelope,
                  begin: OperationBeginCommand) -> HttpResponse {
        let workflow = semantic.meta.correlation_id;
        let operation_type = begin.operation_type;
        if let Err(response) =
            publish_begin_then_semantic_command(&self.publisher, self.bus_id, workflow, begin, semantic)
        {
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
        params: &PathParams,
    ) -> HttpResponse {
        self.act(params, body).unwrap_or_else(|refusal| refusal)
    }
}

type Answer = Result<HttpResponse, HttpResponse>;

impl InputDeviceActionHandler {
    fn act(&self, params: &PathParams, body: &[u8]) -> Answer {
        let adapter_id = adapter_path(self.state.as_ref(), params)?;
        match self.action {
            InputDeviceAction::Scan => self.scan(adapter_id, body),
            InputDeviceAction::Commission => self.commission(adapter_id, body),
            InputDeviceAction::Identify => self.identify(adapter_id, params, body),
            InputDeviceAction::ConfigureInstance => self.configure(adapter_id, params, body),
            InputDeviceAction::ConfigureFeedback => {
                self.configure_feedback(adapter_id, params, body)
            }
            InputDeviceAction::PatchMetadata => self.patch_metadata(adapter_id, params, body),
            InputDeviceAction::Forget => self.forget(adapter_id, params),
        }
    }

    fn envelope<P>(&self, payload: P) -> (u64, dali2rust_contracts::msg::CommandEnvelope)
    where
        dali2rust_contracts::msg::BusCommandPayload: From<P>,
    {
        let correlation_id = self.correlation.next_id();
        let envelope = dali2rust_contracts::bus::command_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            payload,
        );
        (correlation_id, envelope)
    }

    fn device(
        &self,
        adapter_id: u8,
        params: &PathParams,
    ) -> Result<(u8, InputDeviceDto), HttpResponse> {
        let short_address = device_path(params)?;
        Ok((short_address, self.record(adapter_id, short_address)?))
    }

    fn record(&self, adapter_id: u8, short_address: u8) -> Result<InputDeviceDto, HttpResponse> {
        self.state
            .detail(adapter_id, short_address)
            .ok_or_else(|| json_err(404, "input_device_not_found"))
    }

    fn instance(
        &self,
        adapter_id: u8,
        params: &PathParams,
    ) -> Result<(u8, u8, InputDeviceDto), HttpResponse> {
        let short_address = device_path(params)?;
        let instance_number =
            path_u8(params, "instance_number").ok_or_else(|| json_err(400, "invalid_resource_id"))?;
        let device = self.record(adapter_id, short_address)?;
        if instance_number >= device.summary.instance_count {
            return Err(json_err(404, "instance_not_found"));
        }
        Ok((short_address, instance_number, device))
    }

    fn scan(&self, adapter_id: u8, body: &[u8]) -> Answer {
        parse_body_object(body, &[], &[])?;
        let (corr, semantic) = self.envelope(Dali103ScanCommand {
            registry_adapter_id: adapter_id,
        });
        let key = format!("inp-scan-{adapter_id}-{corr}");
        Ok(self.operation(key, semantic, OperationType::Discovery))
    }

    fn commission(&self, adapter_id: u8, body: &[u8]) -> Answer {
        let include_addressed = parse_commission_body(body)?;
        let (corr, semantic) = self.envelope(Dali103CommissionCommand {
            registry_adapter_id: adapter_id,
            include_addressed,
        });
        let key = format!("inp-comm-{adapter_id}-{corr}");
        Ok(self.operation(key, semantic, OperationType::CommissioningAddressChange))
    }

    fn identify(&self, adapter_id: u8, params: &PathParams, body: &[u8]) -> Answer {
        let (short_address, _) = self.device(adapter_id, params)?;
        parse_body_object(body, &[], &[])?;
        let (corr, semantic) = self.envelope(Dali103IdentifyCommand {
            registry_adapter_id: adapter_id,
            short_address,
        });
        let key = format!("inp-id-{adapter_id}-{short_address}-{corr}");
        Ok(self.operation(key, semantic, OperationType::CommissioningIdentify))
    }

    fn configure(&self, adapter_id: u8, params: &PathParams, body: &[u8]) -> Answer {
        let (short_address, instance_number, _) = self.instance(adapter_id, params)?;
        let cmd = parse_instance_patch(body, adapter_id, short_address, instance_number)?;
        let (corr, semantic) = self.envelope(cmd);
        let key = format!("inp-cfg-{adapter_id}-{short_address}-{instance_number}-{corr}");
        Ok(self.wire_config_write(key, semantic))
    }

    fn configure_feedback(&self, adapter_id: u8, params: &PathParams, body: &[u8]) -> Answer {
        let (short_address, instance_number, device) = self.instance(adapter_id, params)?;
        let mut cmd = parse_feedback_patch(body, adapter_id, short_address, instance_number)?;
        cmd.opcode_map = feedback_dialect(&device, instance_number)?;
        let (corr, semantic) = self.envelope(cmd);
        let key = format!("inp-fb-{adapter_id}-{short_address}-{instance_number}-{corr}");
        Ok(self.wire_config_write(key, semantic))
    }

    fn patch_metadata(&self, adapter_id: u8, params: &PathParams, body: &[u8]) -> Answer {
        let (short_address, _) = self.device(adapter_id, params)?;
        let (meta, notes) = parse_metadata_patch(body, adapter_id, short_address)?;
        let mut batch = Vec::with_capacity(2);
        if meta.patch_mask != 0 {
            batch.push(self.frame(meta));
        }
        if let Some(notes) = notes {
            batch.push(self.frame(notes));
        }
        self.publish_confirmed(batch)?;
        let (_, mut dto) = self.device(adapter_id, params)?;
        dto.now_ms = self.wall.unix_millis();
        Ok(json_stream_dto(dto))
    }

    fn forget(&self, adapter_id: u8, params: &PathParams) -> Answer {
        let (short_address, _) = self.device(adapter_id, params)?;
        let forget = self.frame(dali2rust_contracts::msg::InputDeviceMetadataUpdateCommand {
            registry_adapter_id: adapter_id,
            short_address,
            patch_mask: InputDeviceMetadataUpdateCommand::PATCH_FORGET,
            name: dali2rust_contracts::msg::fixed_text_64(""),
            ha_expose: false,
        });
        self.publish_confirmed(vec![forget])?;
        Ok(HttpResponse::json(200, br#"{"forgotten":true}"#.to_vec()))
    }

    fn frame<P>(&self, payload: P) -> (u64, dali2rust_bus::BusFrame)
    where
        dali2rust_contracts::msg::BusCommandPayload: From<P>,
    {
        let (correlation_id, envelope) = self.envelope(payload);
        (correlation_id, dali2rust_bus::BusFrame::command(envelope))
    }

    fn publish_confirmed(&self, batch: Vec<(u64, dali2rust_bus::BusFrame)>) -> Result<(), HttpResponse> {
        publish_batch_and_wait_for_success(&self.publisher, &self.slots, self.timeout_ms, batch)
    }
}

fn parse_commission_body(body: &[u8]) -> Result<bool, HttpResponse> {
    let json = parse_body_object(body, &["include_addressed"], &[])?;
    match json.get("include_addressed") {
        None => Ok(false),
        Some(value) => value.as_bool().ok_or_else(|| json_err(422, "invalid_value")),
    }
}

const INSTANCE_PATCH_KEYS: [&str; 6] = [
    "event_scheme",
    "event_priority",
    "enabled",
    "event_filter",
    "instance_groups",
    "timers",
];
const INSTANCE_GROUP_SLOTS: usize = 3;
const INSTANCE_READ_ONLY_KEYS: &[&str] = &[
    "instance_number",
    "instance_type",
    "instance_type_name",
    "instance_status",
    "resolution",
    "event_scheme_confirmed",
    "manual_config_active",
    "feedback",
    "runtime",
];
const FEEDBACK_READ_ONLY_KEYS: &[&str] =
    &["probed", "present", "opcode_map", "capability", "colour_capability"];
const DEVICE_READ_ONLY_KEYS: &[&str] = &[
    "adapter_id",
    "short_address",
    "present",
    "instance_count",
    "first_instance_type",
    "last_seen_ms",
    "last_event_at_ms",
    "device_capabilities",
    "device_status",
    "version_number",
    "now_ms",
    "nvm_settling_until_ms",
    "instances",
];

fn parse_instance_patch(
    body: &[u8],
    adapter_id: u8,
    short_address: u8,
    instance_number: u8,
) -> Result<Dali103InstanceConfigureCommand, HttpResponse> {
    let json = Value::Object(parse_body_object(body, &INSTANCE_PATCH_KEYS, INSTANCE_READ_ONLY_KEYS)?);
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
    check_body_keys(timers, &SPECS.map(|(key, ..)| key), &[])?;
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
        cmd.patch(InstancePatchField::TIMERS[slot]);
    }
    Ok(())
}

fn feedback_dialect(
    device: &InputDeviceDto,
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
) -> Result<Dali103FeedbackConfigureCommand, HttpResponse> {
    let writable = FeedbackPatchField::ALL.map(feedback_key);
    let json = Value::Object(parse_body_object(body, &writable, FEEDBACK_READ_ONLY_KEYS)?);
    let mut cmd = Dali103FeedbackConfigureCommand {
        registry_adapter_id: adapter_id,
        short_address,
        instance_number,
        patch_mask: 0,
        timing: 0,
        active_brightness: 0,
        active_colour: 0,
        inactive_brightness: 0,
        inactive_colour: 0,
        opcode_map: 0,
    };
    parse_feedback_fields(&json, &mut cmd)?;
    if cmd.patch_mask == 0 {
        return Err(json_err(400, "empty_patch"));
    }
    Ok(cmd)
}

fn parse_feedback_fields(
    json: &Value,
    cmd: &mut Dali103FeedbackConfigureCommand,
) -> Result<(), HttpResponse> {
    for field in FeedbackPatchField::ALL {
        let Some(value) = json.get(feedback_key(field)) else { continue };
        let raw = value.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        let value = u8::try_from(raw).map_err(|_| json_err(422, "invalid_value"))?;
        let is_colour = matches!(
            field,
            FeedbackPatchField::ActiveColour | FeedbackPatchField::InactiveColour
        );
        if is_colour && !FEEDBACK_COLOUR_RANGE.contains(&value) {
            return Err(json_err(422, "invalid_feedback_colour"));
        }
        cmd.patch(field, value);
    }
    Ok(())
}

const fn feedback_key(field: FeedbackPatchField) -> &'static str {
    match field {
        FeedbackPatchField::Timing => "timing",
        FeedbackPatchField::ActiveBrightness => "active_brightness",
        FeedbackPatchField::ActiveColour => "active_colour",
        FeedbackPatchField::InactiveBrightness => "inactive_brightness",
        FeedbackPatchField::InactiveColour => "inactive_colour",
    }
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
        cmd.patch(InstancePatchField::EventScheme);
    }
    if let Some(priority) = json.get("event_priority") {
        let priority = priority.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
        if !(2..=5).contains(&priority) || priority == RESERVED_EVENT_PRIORITY {
            return Err(json_err(422, "invalid_value"));
        }
        cmd.event_priority = u8::try_from(priority).unwrap_or(3);
        cmd.patch(InstancePatchField::EventPriority);
    }
    if let Some(enabled) = json.get("enabled") {
        cmd.instance_enabled = enabled.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
        cmd.patch(InstancePatchField::InstanceEnabled);
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
        cmd.patch(InstancePatchField::EventFilter);
    }
    if let Some(groups) = json.get("instance_groups") {
        let groups = groups.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
        if groups.len() > INSTANCE_GROUP_SLOTS {
            return Err(json_err(422, "invalid_value"));
        }
        for (slot, group) in groups.iter().enumerate() {
            cmd.instance_groups[slot] = parse_group(group)?;
            cmd.patch(InstancePatchField::INSTANCE_GROUPS[slot]);
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

const NAME_MAX_BYTES: usize = 64;
const NOTES_MAX_BYTES: usize = 48;

fn within_bytes(field: &str, text: &str, cap: usize) -> Result<(), HttpResponse> {
    if text.len() <= cap {
        return Ok(());
    }
    let message = format!("{field} is {} bytes; the limit is {cap}", text.len());
    Err(json_err_with_message(422, "invalid_value", &message))
}

fn parse_notes_patch(
    json: &Value,
    adapter_id: u8,
    short_address: u8,
) -> Result<Option<InputDeviceNotesUpdateCommand>, HttpResponse> {
    let text = match json.get("notes") {
        None => return Ok(None),
        Some(Value::Null) => "",
        Some(Value::String(text)) => {
            within_bytes("notes", text, NOTES_MAX_BYTES)?;
            text.as_str()
        }
        Some(_) => return Err(json_err(422, "invalid_value")),
    };
    Ok(Some(InputDeviceNotesUpdateCommand {
        registry_adapter_id: adapter_id,
        short_address,
        notes: dali2rust_contracts::msg::fixed_text_48(text),
    }))
}

fn parse_metadata_patch(
    body: &[u8],
    adapter_id: u8,
    short_address: u8,
) -> Result<(InputDeviceMetadataUpdateCommand, Option<InputDeviceNotesUpdateCommand>), HttpResponse>
{
    let writable = ["name", "ha_expose", "notes"];
    let json = Value::Object(parse_body_object(body, &writable, DEVICE_READ_ONLY_KEYS)?);
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
            within_bytes("name", name, NAME_MAX_BYTES)?;
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
    let notes = parse_notes_patch(&json, adapter_id, short_address)?;
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
