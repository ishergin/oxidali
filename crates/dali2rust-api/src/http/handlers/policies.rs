use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    PoliciesUpdateCommand, PolicyException, PolicyExceptionList, MAX_POLICY_EXCEPTIONS,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::MAX_SHORT_ADDRESSES;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::CorrelationIdAllocator;
use crate::http::handler::ApiHandler;
use dali2rust_contracts::msg::OperationType;

use crate::http::handlers::common::{
    check_body_keys, json_err, json_stream_dto, parse_bool_field, parse_json_body,
    publish_and_await_apply, publish_apply_execute, reject_if_apply_active, require_get,
};
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::physical_device_state::PhysicalDeviceHttpState;
use crate::http::policies_state::{PoliciesApplyWatch, PoliciesHttpState};
use crate::http::types::HttpResponse;

const ALLOWED_KEYS: &[&str] = &[
    "system_failure_level",
    "power_on_level",
    "apply_on_discovery",
    "exceptions",
];

const EXCEPTION_KEYS: &[&str] = &["adapter_id", "short_address"];

const READ_ONLY_KEYS: &[&str] = &["manages_anything"];

const MAX_MANAGED_LEVEL: u64 = 254;

declare_handler_shell! {
    PoliciesGetHandler {
        state: Arc<dyn PoliciesHttpState>,
    }
}

impl ApiHandler for PoliciesGetHandler {
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
        json_stream_dto(self.state.policies_dto())
    }
}

declare_handler_shell!(PoliciesPatchHandler {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn PoliciesHttpState>,
    apply_watch: Arc<dyn PoliciesApplyWatch>,
    devices: Arc<dyn PhysicalDeviceHttpState>,
    bus_id: BusId,
    timeout_ms: u64,
});

#[derive(Default)]
pub struct PoliciesPatchData {
    patch_mask: u8,
    system_failure_level: u8,
    power_on_level: u8,
    apply_on_discovery: bool,
    exceptions: PolicyExceptionList,
}

fn parse_level(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u8>, HttpResponse> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(PoliciesUpdateCommand::UNMANAGED));
    }
    let n = value
        .as_u64()
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if n > MAX_MANAGED_LEVEL {
        return Err(json_err(422, "out_of_range"));
    }
    Ok(Some(u8::try_from(n).unwrap_or(PoliciesUpdateCommand::UNMANAGED)))
}

fn parse_exception_byte(
    entry: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u8, HttpResponse> {
    let value = entry.get(key).and_then(serde_json::Value::as_u64);
    value.and_then(|n| u8::try_from(n).ok()).ok_or_else(|| json_err(422, "invalid_value"))
}

fn parse_exception(value: &serde_json::Value) -> Result<PolicyException, HttpResponse> {
    let entry = value.as_object().ok_or_else(|| json_err(422, "invalid_value"))?;
    if entry.keys().any(|key| !EXCEPTION_KEYS.contains(&key.as_str())) {
        return Err(json_err(422, "invalid_value"));
    }
    let adapter_id = parse_exception_byte(entry, "adapter_id")?;
    let short_address = parse_exception_byte(entry, "short_address")?;
    if usize::from(short_address) >= MAX_SHORT_ADDRESSES {
        return Err(json_err(422, "out_of_range"));
    }
    Ok(PolicyException { adapter_id, short_address })
}

fn parse_exception_list(value: &serde_json::Value) -> Result<PolicyExceptionList, HttpResponse> {
    let entries = value.as_array().ok_or_else(|| json_err(422, "invalid_value"))?;
    if entries.len() > MAX_POLICY_EXCEPTIONS {
        return Err(json_err(422, "too_many_exceptions"));
    }
    let mut list = PolicyExceptionList::new();
    for entry in entries {
        let device = parse_exception(entry)?;
        if list.contains(&device) || list.push(device).is_err() {
            return Err(json_err(422, "duplicate_exception"));
        }
    }
    Ok(list)
}

fn parse_levels(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<PoliciesPatchData, HttpResponse> {
    let mut data = PoliciesPatchData::default();
    if let Some(level) = parse_level(obj, "system_failure_level")? {
        data.patch_mask |= PoliciesUpdateCommand::PATCH_SYSTEM_FAILURE_LEVEL;
        data.system_failure_level = level;
    }
    if let Some(level) = parse_level(obj, "power_on_level")? {
        data.patch_mask |= PoliciesUpdateCommand::PATCH_POWER_ON_LEVEL;
        data.power_on_level = level;
    }
    if let Some(value) = parse_bool_field(obj, "apply_on_discovery")? {
        data.patch_mask |= PoliciesUpdateCommand::PATCH_APPLY_ON_DISCOVERY;
        data.apply_on_discovery = value;
    }
    Ok(data)
}

impl PoliciesPatchHandler {
    fn known_exceptions(
        &self,
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<PolicyExceptionList>, HttpResponse> {
        let Some(value) = obj.get("exceptions") else {
            return Ok(None);
        };
        let list = parse_exception_list(value)?;
        let stored = self.state.policies_dto().exceptions;
        let known = |device: &PolicyException| {
            stored.contains(device)
                || self.devices.physical_device_exists(device.adapter_id, device.short_address)
        };
        if !list.iter().all(known) {
            return Err(json_err(422, "unknown_device"));
        }
        Ok(Some(list))
    }
}

impl crate::http::handlers::common::MutatingHandler for PoliciesPatchHandler {
    type Validated = PoliciesPatchData;
    type Executed = ();

    fn expected_method(&self) -> &'static str {
        "PATCH"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<PoliciesPatchData, HttpResponse> {
        let v = parse_json_body(body)?;
        let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
        check_body_keys(obj, ALLOWED_KEYS, READ_ONLY_KEYS)?;
        let mut data = parse_levels(obj)?;
        if let Some(exceptions) = self.known_exceptions(obj)? {
            data.patch_mask |= PoliciesUpdateCommand::PATCH_EXCEPTIONS;
            data.exceptions = exceptions;
        }
        Ok(data)
    }

    fn execute(&self, data: PoliciesPatchData) -> Result<(), HttpResponse> {
        if data.patch_mask == 0 {
            return Ok(());
        }
        let correlation_id = self.correlation.next_id();
        let cmd = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            None,
            PoliciesUpdateCommand {
                patch_mask: data.patch_mask,
                system_failure_level: data.system_failure_level,
                power_on_level: data.power_on_level,
                apply_on_discovery: data.apply_on_discovery,
                exceptions: data.exceptions,
            },
        );
        let watch = Arc::clone(&self.apply_watch);
        publish_and_await_apply(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(cmd),
            move || watch.policies_applied_load(),
        )
    }

    fn respond(&self, (): ()) -> HttpResponse {
        json_stream_dto(self.state.policies_dto())
    }
}

declare_handler_shell!(PoliciesApplyHandler {
    publisher: BusPublisher,
    correlation: Arc<CorrelationIdAllocator>,
    state: Arc<dyn PoliciesHttpState>,
    bus_id: BusId,
    registry_adapter_id: u8,
    operations: Arc<dyn dali2rust_domain::registry::OperationReadPort>,
});

impl crate::http::handlers::common::MutatingHandler for PoliciesApplyHandler {
    type Validated = ();
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        _params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(), HttpResponse> {
        if !self.state.policies_dto().manages_anything {
            return Err(json_err(409, "nothing_managed"));
        }
        reject_if_apply_active(
            self.operations.as_ref(),
            OperationType::PolicyApply,
            self.registry_adapter_id,
        )
    }

    fn execute(&self, (): ()) -> Result<HttpResponse, HttpResponse> {
        let workflow = self.correlation.next_id();
        let operation_id = format!("policy-apply-{}-{workflow}", self.registry_adapter_id);
        let execute = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::PolicyApplyExecuteCommand {
                registry_adapter_id: self.registry_adapter_id,
                operation_key: dali2rust_contracts::msg::fixed_text_32(&operation_id),
            },
        );
        publish_apply_execute(
            &self.publisher,
            self.bus_id,
            OperationType::PolicyApply,
            operation_id,
            execute,
        )
    }

    fn respond(&self, executed: HttpResponse) -> HttpResponse {
        executed
    }
}
