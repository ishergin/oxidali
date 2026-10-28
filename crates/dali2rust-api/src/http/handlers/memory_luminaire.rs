use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    DaliWriteMemoryLuminaireCommand, LuminaireNumbers, LuminaireTextWrite, OperationType, Origin,
};
use dali2rust_domain::dali::banks::luminaire_write::{
    check_writable, number_patch, text_patch, Bank1Layout, Bank1WriteRefusal, FieldKind,
    LuminaireField,
};
use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::banks::IdentityRead;
use serde_json::{Map, Value};

use super::common::{
    accepted_operation_response, json_err, json_err_with_message, parse_json_body, MutatingHandler,
};
use super::operation_dispatch::publish_begin_then_semantic_command_pair;
use super::physical_devices::parse_existing_physical_device;
use super::resource_surface::declare_handler_shell;
use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::physical_device_state::PhysicalDeviceHttpState;
use crate::http::types::HttpResponse;

declare_handler_shell!(
    PhysicalDeviceMemoryLuminaireHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        bus_id: BusId,
        state: Arc<dyn PhysicalDeviceHttpState>,
    }
);

pub struct LuminairePatch {
    numbers: LuminaireNumbers,
    text: Option<LuminaireTextWrite>,
}

type Fields = Vec<&'static LuminaireField>;

fn requested_fields(obj: &Map<String, Value>) -> Result<Fields, HttpResponse> {
    if obj.is_empty() {
        return Err(json_err(400, "empty_patch"));
    }
    obj.keys()
        .map(|key| LuminaireField::named(key).ok_or_else(|| json_err(400, "unknown_field")))
        .collect()
}

fn refused(refusal: Bank1WriteRefusal) -> HttpResponse {
    json_err_with_message(409, "conflict", refusal.cause())
}

// IEC 62386-102 §9.10.4
// DiiA 251 §9.2.3
fn gated_format(layout: &Bank1Layout, fields: &Fields) -> Result<LuminaireFormat, HttpResponse> {
    if fields.iter().any(|field| field.kind == FieldKind::LayoutId) {
        return Err(refused(Bank1WriteRefusal::FieldReadOnly));
    }
    let mut format = None;
    for field in fields {
        format = Some(check_writable(layout, field).map_err(refused)?);
    }
    format.ok_or_else(|| json_err(400, "empty_patch"))
}

// IEC 62386-102 §9.10.6, Table 9
fn gated_identity(identity: IdentityRead) -> Result<(), HttpResponse> {
    identity.known().map(|_| ()).map_err(|cause| json_err_with_message(409, "conflict", cause))
}

fn invalid(field: &LuminaireField) -> HttpResponse {
    json_err_with_message(422, "invalid_value", field.name)
}

fn number_value(field: &LuminaireField, value: &Value, format: LuminaireFormat) -> Result<u32, HttpResponse> {
    value
        .as_u64()
        .filter(|number| field.accepts_number(*number, format))
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| invalid(field))
}

fn text_value(field: &LuminaireField, value: &Value) -> Result<LuminaireTextWrite, HttpResponse> {
    value
        .as_str()
        .filter(|text| field.accepts_text(text))
        .and_then(|text| text_patch(field, text))
        .ok_or_else(|| invalid(field))
}

fn parse_patch(
    obj: &Map<String, Value>,
    fields: &Fields,
    format: LuminaireFormat,
) -> Result<LuminairePatch, HttpResponse> {
    let mut numbers = Vec::with_capacity(fields.len());
    let mut text = None;
    for field in fields {
        let value = obj.get(field.name).unwrap_or(&Value::Null);
        match field.kind {
            FieldKind::Number { patch, .. } => numbers.push((patch, number_value(field, value, format)?)),
            FieldKind::Text(_) if text.is_some() => return Err(invalid(field)),
            FieldKind::Text(_) => text = Some(text_value(field, value)?),
            FieldKind::LayoutId => return Err(refused(Bank1WriteRefusal::FieldReadOnly)),
        }
    }
    Ok(LuminairePatch { numbers: number_patch(&numbers), text })
}

impl MutatingHandler for PhysicalDeviceMemoryLuminaireHandler {
    type Validated = (u8, u8, LuminairePatch);
    type Executed = String;

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, LuminairePatch), HttpResponse> {
        let (aid, short) = parse_existing_physical_device(self.state.as_ref(), params)?;
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        let fields = requested_fields(obj)?;
        let layout = self.state.physical_device_bank1_layout(aid, short).unwrap_or_default();
        let format = gated_format(&layout, &fields)?;
        gated_identity(self.state.physical_device_identity(aid, short))?;
        Ok((aid, short, parse_patch(obj, &fields, format)?))
    }

    fn execute(&self, (aid, short, patch): (u8, u8, LuminairePatch)) -> Result<String, HttpResponse> {
        let op_key = format!("pd-memlum-{aid}-{short}-{}", self.correlation.next_id());
        let workflow = self.correlation.next_id();
        let command = DaliWriteMemoryLuminaireCommand {
            registry_adapter_id: aid,
            short_address: short,
            numbers: patch.numbers,
            text: patch.text,
        };
        let envelope = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            self.bus_id.0,
            Some(Origin::Api),
            command,
        );
        let kind = OperationType::AttributeWrite;
        publish_begin_then_semantic_command_pair(&self.publisher, self.bus_id, workflow, &op_key, kind, envelope)?;
        Ok(op_key)
    }

    fn respond(&self, op_key: String) -> HttpResponse {
        accepted_operation_response(op_key, OperationType::AttributeWrite)
    }
}
