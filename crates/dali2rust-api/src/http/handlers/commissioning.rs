use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{CommissioningStep, InitialiseScope, OperationType};
use dali2rust_domain::dali::pres::special::QueryShortAddressAnswer;
use dali2rust_domain::registry::OperationReadPort;
use serde::Deserialize;

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{
    accepted_operation_response, json_err, parse_adapter_id, parse_json_body,
    parse_typed_body, reject_if_commissioning_active, MutatingHandler,
};
use crate::http::handlers::operation_dispatch::publish_begin_then_semantic_command_pair;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::physical_device_state::PhysicalDeviceHttpState;
use crate::http::types::HttpResponse;

const MAX_SHORT_ADDRESS: u8 = 63;
// IEC 62386-102 §9.14.3.2
#[derive(Deserialize, serde::Serialize)]
pub struct CommissioningIdentifyRequest {
    pub short_address: u8,
}

fn accept_commissioning_workflow(
    publisher: &BusPublisher,
    correlation: &CorrelationIdAllocator,
    bus_id: BusId,
    op_key: String,
    operation_type: OperationType,
    payload: impl Into<dali2rust_contracts::msg::BusCommandPayload>,
) -> Result<HttpResponse, HttpResponse> {
    let workflow = correlation.next_id();
    let command = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow,
        bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        payload,
    );
    publish_begin_then_semantic_command_pair(
        publisher,
        bus_id,
        workflow,
        &op_key,
        operation_type,
        command,
    )?;
    Ok(accepted_operation_response(op_key, operation_type))
}

fn validate_identify_keys(body: &[u8]) -> Result<(), HttpResponse> {
    let value = parse_json_body(body)?;
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    if obj.keys().any(|key| key.as_str() != "short_address") {
        return Err(json_err(422, "unsupported_field"));
    }
    Ok(())
}

declare_handler_shell!(CommissioningIdentifyHandler {
    publisher: BusPublisher,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    state: Arc<dyn PhysicalDeviceHttpState>,
    operations: Arc<dyn OperationReadPort>,
});

impl MutatingHandler for CommissioningIdentifyHandler {
    type Validated = (u8, u8);
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Self::Validated, HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        reject_if_commissioning_active(self.operations.as_ref(), adapter_id)?;
        validate_identify_keys(body)?;
        let req: CommissioningIdentifyRequest = parse_typed_body(body)?;

        if req.short_address > MAX_SHORT_ADDRESS {
            return Err(json_err(422, "invalid_value"));
        }
        if !self
            .state
            .physical_device_exists(adapter_id, req.short_address)
        {
            return Err(json_err(404, "not_found"));
        }
        Ok((adapter_id, req.short_address))
    }

    fn execute(&self, validated: Self::Validated) -> Result<Self::Executed, HttpResponse> {
        let (adapter_id, short_address) = validated;
        let op_serial = self.correlation.next_id();
        let op_key = format!("comm-ident-{adapter_id}-{short_address}-{op_serial}");
        let payload = dali2rust_contracts::msg::DaliIdentifyDeviceCommand {
            registry_adapter_id: adapter_id,
            short_address,
            operation_key: dali2rust_contracts::msg::fixed_text_32(&op_key),
        };
        accept_commissioning_workflow(
            &self.publisher,
            &self.correlation,
            self.bus_id,
            op_key,
            OperationType::CommissioningIdentify,
            payload,
        )
    }

    fn respond(&self, executed: Self::Executed) -> HttpResponse {
        executed
    }
}

#[allow(dead_code, reason = "asserts the blanket ApiHandler impl covers this handler")]
fn _assert_api_handler(h: &CommissioningIdentifyHandler) -> &dyn ApiHandler {
    h
}

#[derive(Deserialize, serde::Serialize)]
pub struct CommissioningAddressChangeRequest {
    pub short_address: u8,
    pub new_short_address: u8,
    #[serde(default)]
    pub verify_after_program: Option<bool>,
}

declare_handler_shell!(CommissioningAddressChangeHandler {
    publisher: BusPublisher,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    state: Arc<dyn PhysicalDeviceHttpState>,
    operations: Arc<dyn OperationReadPort>,
});

impl MutatingHandler for CommissioningAddressChangeHandler {
    type Validated = (u8, u8, u8, bool);
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Self::Validated, HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        reject_if_commissioning_active(self.operations.as_ref(), adapter_id)?;
        let req: CommissioningAddressChangeRequest = parse_typed_body(body)?;

        if req.short_address > MAX_SHORT_ADDRESS || req.new_short_address > MAX_SHORT_ADDRESS {
            return Err(json_err(422, "invalid_value"));
        }
        if req.short_address == req.new_short_address {
            return Err(json_err(422, "invalid_value"));
        }
        if !self
            .state
            .physical_device_exists(adapter_id, req.short_address)
        {
            return Err(json_err(404, "not_found"));
        }
        if self
            .state
            .physical_device_exists(adapter_id, req.new_short_address)
        {
            return Err(json_err(409, "conflict"));
        }
        Ok((
            adapter_id,
            req.short_address,
            req.new_short_address,
            req.verify_after_program.unwrap_or(true),
        ))
    }

    fn execute(&self, validated: Self::Validated) -> Result<Self::Executed, HttpResponse> {
        let (adapter_id, short_address, new_short_address, verify_after_program) = validated;
        let op_serial = self.correlation.next_id();
        let op_key = format!("comm-addr-{adapter_id}-{short_address}-{new_short_address}-{op_serial}");
        let payload = dali2rust_contracts::msg::DaliAddressingCommand {
            registry_adapter_id: adapter_id,
            short_address,
            new_short_address,
            verify_after_program,
            operation_key: dali2rust_contracts::msg::fixed_text_32(&op_key),
        };
        accept_commissioning_workflow(
            &self.publisher,
            &self.correlation,
            self.bus_id,
            op_key,
            OperationType::CommissioningAddressChange,
            payload,
        )
    }

    fn respond(&self, executed: Self::Executed) -> HttpResponse {
        executed
    }
}

fn parse_step(step: &str) -> Option<CommissioningStep> {
    Some(match step {
        "initialise" => CommissioningStep::Initialise,
        "randomise" => CommissioningStep::Randomise,
        "search-address" => CommissioningStep::SearchAddress,
        "compare" => CommissioningStep::Compare,
        "program-short-address" => CommissioningStep::ProgramShortAddress,
        "verify-short-address" => CommissioningStep::VerifyShortAddress,
        "query-short-address" => CommissioningStep::QueryShortAddress,
        "withdraw" => CommissioningStep::Withdraw,
        "terminate" => CommissioningStep::Terminate,
        _ => return None,
    })
}

fn parse_scope(scope: &str) -> Option<InitialiseScope> {
    Some(match scope {
        "all" => InitialiseScope::All,
        "unaddressed" => InitialiseScope::Unaddressed,
        "short" => InitialiseScope::Short,
        _ => return None,
    })
}

const MAX_SEARCH_ADDRESS: u32 = 0x00FF_FFFF;

#[derive(Deserialize, Default)]
pub struct CommissioningStepRequest {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub short_address: Option<u8>,
    #[serde(default)]
    pub search_address: Option<u32>,
}

declare_handler_shell! {
    CommissioningStepHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    adapter_count: u8,
    operations: Arc<dyn OperationReadPort>,
    timeout_ms: u64,
    }
}

impl CommissioningStepHandler {

    fn typed_result(step: CommissioningStep, confirmation: &[u8]) -> Vec<u8> {
        let base: serde_json::Value =
            serde_json::from_slice(confirmation).unwrap_or(serde_json::Value::Null);
        let success = base.get("success").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let backward = base.get("backward_frame").and_then(serde_json::Value::as_u64);
        let violation = base
            .get("backward_violation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let mut out = serde_json::Map::new();
        out.insert("success".into(), success.into());
        if let Some(b) = backward {
            out.insert("backward_frame".into(), b.into());
        }
        Self::carry_named_cause(&base, &mut out);
        Self::insert_step_reading(step, success, backward, violation, &mut out);
        serde_json::to_vec(&serde_json::Value::Object(out)).unwrap_or_default()
    }

    fn carry_named_cause(base: &serde_json::Value, out: &mut serde_json::Map<String, serde_json::Value>) {
        for key in ["error_code", "message"] {
            if let Some(value) = base.get(key).filter(|v| !v.is_null()) {
                out.insert(key.into(), value.clone());
            }
        }
    }

    fn insert_step_reading(
        step: CommissioningStep,
        success: bool,
        backward: Option<u64>,
        violation: bool,
        out: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        match step {
            // IEC 62386-101 §8.2.5
            CommissioningStep::Compare | CommissioningStep::VerifyShortAddress => {
                out.insert(
                    "match".into(),
                    (success && (violation || backward.unwrap_or(0) != 0)).into(),
                );
            }
            CommissioningStep::QueryShortAddress => {
                let answer = if success {
                    QueryShortAddressAnswer::decode(
                        backward.and_then(|raw| u8::try_from(raw).ok()),
                        violation,
                    )
                } else {
                    QueryShortAddressAnswer::None
                };
                out.insert(
                    "short_address".into(),
                    answer.address().map_or(serde_json::Value::Null, |s| s.into()),
                );
                out.insert("answer".into(), answer.wire_name().into());
            }
            _ => {}
        }
    }
}

pub struct ValidatedStep {
    adapter_id: u8,
    step: CommissioningStep,
    scope: Option<InitialiseScope>,
    short_address: Option<u8>,
    search_address: Option<u32>,
}

fn resolve_scope(
    step: CommissioningStep,
    raw: Option<&str>,
) -> Result<Option<InitialiseScope>, HttpResponse> {
    match (step, raw) {
        (CommissioningStep::Initialise, Some(raw)) => {
            parse_scope(raw).map(Some).ok_or_else(|| json_err(422, "invalid_enum"))
        }
        (CommissioningStep::Initialise, None) => Ok(Some(InitialiseScope::All)),
        _ => Ok(None),
    }
}

fn check_step_params(
    step: CommissioningStep,
    scope: Option<InitialiseScope>,
    short_address: Option<u8>,
    search_address: Option<u32>,
) -> Result<(), HttpResponse> {
    if short_address.is_some_and(|s| s > MAX_SHORT_ADDRESS)
        || search_address.is_some_and(|a| a > MAX_SEARCH_ADDRESS)
    {
        return Err(json_err(422, "invalid_value"));
    }
    let needs_short = matches!(
        step,
        CommissioningStep::ProgramShortAddress | CommissioningStep::VerifyShortAddress
    ) || scope == Some(InitialiseScope::Short);
    if needs_short && short_address.is_none() {
        return Err(json_err(422, "invalid_value"));
    }
    if step == CommissioningStep::SearchAddress && search_address.is_none() {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(())
}

impl MutatingHandler for CommissioningStepHandler {
    type Validated = ValidatedStep;
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Self::Validated, HttpResponse> {
        let adapter_id = parse_adapter_id(self.adapter_count, params)?;
        let step = params
            .get("step")
            .and_then(|s| parse_step(s))
            .ok_or_else(|| json_err(404, "not_found"))?;
        reject_if_commissioning_active(self.operations.as_ref(), adapter_id)?;

        let req: CommissioningStepRequest = if body.is_empty() {
            CommissioningStepRequest::default()
        } else {
            parse_typed_body(body)?
        };
        let scope = resolve_scope(step, req.scope.as_deref())?;
        check_step_params(step, scope, req.short_address, req.search_address)?;
        Ok(ValidatedStep {
            adapter_id,
            step,
            scope,
            short_address: req.short_address,
            search_address: req.search_address,
        })
    }

    fn execute(&self, validated: Self::Validated) -> Result<Self::Executed, HttpResponse> {
        let correlation_id = self.correlation.next_id();
        let wait = self
            .slots
            .try_register(correlation_id, crate::bus_codec::confirmation_to_json_body)
            .map_err(|()| json_err(503, "confirmation_slots_exhausted"))?;
        let frame = dali2rust_bus::BusFrame::command(dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::DaliCommissioningStepCommand {
                registry_adapter_id: validated.adapter_id,
                step: validated.step,
                scope: validated.scope,
                short_address: validated.short_address,
                search_address: validated.search_address,
            },
        ));
        if self
            .publisher
            .try_publish(dali2rust_bus::BusChannel::Commands, frame)
            != dali2rust_bus::PublishResult::Queued
        {
            self.slots.cancel(correlation_id);
            return Err(json_err(503, "commands_ingress_overload"));
        }
        let bytes = crate::http::dispatcher::recv_confirmation_bytes(
            wait,
            self.timeout_ms,
            &self.slots,
            correlation_id,
        )?;
        Ok(HttpResponse::json(
            200,
            Self::typed_result(validated.step, &bytes),
        ))
    }

    fn respond(&self, executed: Self::Executed) -> HttpResponse {
        executed
    }
}

#[derive(Deserialize, Default)]
pub struct RestoreScopeRequest {
    #[serde(default = "default_true")]
    pub metadata_and_overrides: bool,
    #[serde(default = "default_true")]
    pub attributes: bool,
    #[serde(default = "default_true")]
    pub groups: bool,
    #[serde(default = "default_true")]
    pub scenes: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct CommissioningReplacementRequest {
    pub failed_short_address: u8,
    pub replacement_short_address: u8,
    #[serde(default = "default_restore")]
    pub restore: RestoreScopeRequest,
}

fn default_restore() -> RestoreScopeRequest {
    RestoreScopeRequest {
        metadata_and_overrides: true,
        attributes: true,
        groups: true,
        scenes: true,
    }
}

declare_handler_shell!(CommissioningReplacementHandler {
    publisher: BusPublisher,
    correlation: Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    state: Arc<dyn PhysicalDeviceHttpState>,
    operations: Arc<dyn OperationReadPort>,
});

pub struct ValidatedReplacement {
    adapter_id: u8,
    failed: u8,
    replacement: u8,
    restore: RestoreScopeRequest,
}

impl MutatingHandler for CommissioningReplacementHandler {
    type Validated = ValidatedReplacement;
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Self::Validated, HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        reject_if_commissioning_active(self.operations.as_ref(), adapter_id)?;
        let req: CommissioningReplacementRequest = parse_typed_body(body)?;

        if req.failed_short_address > MAX_SHORT_ADDRESS
            || req.replacement_short_address > MAX_SHORT_ADDRESS
            || req.failed_short_address == req.replacement_short_address
        {
            return Err(json_err(422, "invalid_value"));
        }
        if !(req.restore.metadata_and_overrides
            || req.restore.attributes
            || req.restore.groups
            || req.restore.scenes)
        {
            return Err(json_err(422, "invalid_value"));
        }
        for short in [req.failed_short_address, req.replacement_short_address] {
            if !self.state.physical_device_exists(adapter_id, short) {
                return Err(json_err(404, "not_found"));
            }
        }
        Ok(ValidatedReplacement {
            adapter_id,
            failed: req.failed_short_address,
            replacement: req.replacement_short_address,
            restore: req.restore,
        })
    }

    fn execute(&self, v: Self::Validated) -> Result<Self::Executed, HttpResponse> {
        let op_serial = self.correlation.next_id();
        let op_key = format!(
            "comm-repl-{}-{}-{}-{op_serial}",
            v.adapter_id, v.failed, v.replacement
        );
        let payload = dali2rust_contracts::msg::DaliReplaceDeviceCommand {
            registry_adapter_id: v.adapter_id,
            failed_short_address: v.failed,
            replacement_short_address: v.replacement,
            restore_metadata_and_overrides: v.restore.metadata_and_overrides,
            restore_attributes: v.restore.attributes,
            restore_groups: v.restore.groups,
            restore_scenes: v.restore.scenes,
            operation_key: dali2rust_contracts::msg::fixed_text_32(&op_key),
        };
        accept_commissioning_workflow(
            &self.publisher,
            &self.correlation,
            self.bus_id,
            op_key,
            OperationType::CommissioningReplaceDevice,
            payload,
        )
    }

    fn respond(&self, executed: Self::Executed) -> HttpResponse {
        executed
    }
}
