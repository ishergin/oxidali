use std::collections::{HashMap, HashSet};
use dali2rust_contracts::msg::{GroupMetadataUpdateCommand};
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    GroupMatrixDesiredRow, GroupMatrixDesiredRowList, LightSetpoint, OperationType,
    MAX_GROUP_MATRIX_ROWS_PER_COMMAND,
};
use serde_json::Value;

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::{
    dispatch_and_wait_for_success, CorrelationIdAllocator,
};
use crate::http::group_state::{
    GroupHttpState, GroupMembershipMatrixDto, GroupMetadataApplyWatch,
    GroupDto, GroupsListBody,
};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::{
    declare_handler_shell, declare_matrix_write_handler, declare_metadata_patch_handler,
    declare_read_handler,
};
use crate::http::handlers::common::{
    publish_matrix_series,
    accepted_correlation_response, accepted_operation_response, cap_accepts_color_mode, json_err,
    json_stream_dto, parse_adapter_id, parse_json_body, parse_resource_id_param, parse_typed_body,
    write_json_array_items,
};
use crate::http::target_state_request::TargetStateBody;
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{publish_apply_execute, reject_if_apply_active};
use dali2rust_domain::registry::OperationReadPort;

use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};

fn parse_group_id(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, "group_id", u16::from(GROUP_COUNT) - 1)
}

fn matrix_response(dto: GroupMembershipMatrixDto) -> HttpResponse {
    HttpResponse::json_stream(
        200,
        Box::new(move |w: &mut dyn std::io::Write| {
            write!(w, "{{\"adapter_id\":{},\"groups\":[", dto.adapter_id)?;
            write_json_array_items(w, dto.groups.iter())?;
            w.write_all(b"],\"rows\":[")?;
            write_json_array_items(w, dto.rows.iter())?;
            write!(w, "],\"dirty\":{}}}", dto.dirty)?;
            Ok(())
        }),
    )
}

fn list_response(adapter_id: u8, groups: Vec<GroupDto>) -> HttpResponse {
    json_stream_dto(GroupsListBody { adapter_id, groups })
}

declare_read_handler! {
    GroupsListHandler,
    state: GroupHttpState,
    respond: |state: &Arc<dyn GroupHttpState>, adapter_id| {
        list_response(adapter_id, state.list_group_dtos(adapter_id))
    },
}

declare_read_handler! {
    GroupGetHandler,
    state: GroupHttpState,
    id: parse_group_id,
    respond: |state: &Arc<dyn GroupHttpState>, adapter_id, group_id| {
        match state.group_dto(adapter_id, group_id) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_metadata_patch_handler! {
    GroupPatchHandler,
    state: GroupHttpState,
    watch: GroupMetadataApplyWatch,
    watch_load: group_metadata_applied_load,
    data: GroupPatchData,
    id: parse_group_id,
    parse: parse_group_patch_data,
    command: GroupMetadataUpdateCommand { group_id, ha_entity_enabled },
    origin: Some(dali2rust_contracts::msg::Origin::Api),
    echo: |state: &Arc<dyn GroupHttpState>, adapter_id, group_id| {
        match state.group_dto(adapter_id, group_id) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_read_handler! {
    GroupMembershipMatrixGetHandler,
    state: GroupHttpState,
    respond: |state: &Arc<dyn GroupHttpState>, adapter_id| {
        match state.group_membership_matrix_dto(adapter_id) {
            Some(dto) => matrix_response(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_matrix_write_handler! {
    GroupMembershipMatrixWriteHandler,
    state: GroupHttpState,
}

impl crate::http::handlers::common::MutatingHandler for GroupMembershipMatrixWriteHandler {
    type Validated = (u8, Vec<GroupMatrixDesiredRow>);
    type Executed = String;

    fn expected_method(&self) -> &'static str {
        self.mode.expected_method()
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, Vec<GroupMatrixDesiredRow>), HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        let value = parse_json_body(body)?;
        let rows = parse_matrix_rows(&value, self.mode.full_replace())?;
        Ok((adapter_id, rows))
    }

    fn execute(&self, args: (u8, Vec<GroupMatrixDesiredRow>)) -> Result<String, HttpResponse> {
        let (adapter_id, rows) = args;
        publish_group_matrix_rows(
            &self.publisher,
            &self.correlation,
            self.bus_id,
            adapter_id,
            &rows,
            self.mode.full_replace(),
        )
    }

    fn respond(&self, op_key: String) -> HttpResponse {
        accepted_operation_response(op_key, OperationType::ConfigWrite)
    }
}

declare_handler_shell! {
    GroupApplyHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn GroupHttpState>,
        operations: Arc<dyn OperationReadPort>,
        bus_id: BusId,
    }
}

impl crate::http::handlers::common::MutatingHandler for GroupApplyHandler {
    type Validated = (u8, bool);
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(u8, bool), HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        reject_if_apply_active(
            self.operations.as_ref(),
            OperationType::GroupApply,
            adapter_id,
        )?;
        let snapshot = self
            .state
            .group_apply_snapshot(adapter_id)
            .ok_or_else(|| json_err(404, "not_found"))?;
        let diff_is_empty = dali2rust_domain::registry::collect_group_apply_diff(&snapshot).is_empty();
        Ok((adapter_id, diff_is_empty))
    }

    fn execute(&self, args: (u8, bool)) -> Result<HttpResponse, HttpResponse> {
        let (adapter_id, diff_is_empty) = args;
        if diff_is_empty {
            let dto = self
                .state
                .group_membership_matrix_dto(adapter_id)
                .ok_or_else(|| json_err(404, "not_found"))?;
            return Ok(matrix_response(dto));
        }
        let workflow = self.correlation.next_id();
        let operation_id = format!("grp-apply-{adapter_id}-{workflow}");
        let execute = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::GroupApplyExecuteCommand { registry_adapter_id: adapter_id, operation_key: dali2rust_contracts::msg::fixed_text_32(&operation_id) });
        publish_apply_execute(
            &self.publisher,
            OperationType::GroupApply,
            operation_id,
            execute,
        )
    }

    fn respond(&self, executed: HttpResponse) -> HttpResponse {
        executed
    }
}

declare_handler_shell! {
    GroupTargetStateHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn GroupHttpState>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl GroupTargetStateHandler {
    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, LightSetpoint), HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        let group_id = parse_group_id(params)?;
        let group = self
            .state
            .group_dto(adapter_id, group_id)
            .ok_or_else(|| json_err(404, "not_found"))?;
        let request = parse_typed_body::<TargetStateBody>(body)?;
        let setpoint = crate::http::target_state_request::parse_light_setpoint(&request, |mode| {
            cap_accepts_color_mode(&group.capabilities_summary, mode)
        })?;
        Ok((adapter_id, group_id, setpoint))
    }

    fn execute(&self, args: (u8, u8, LightSetpoint)) -> Result<u64, HttpResponse> {
        let (adapter_id, group_id, setpoint) = args;
        let correlation_id = self.correlation.next_id();
        let command = dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliSetTargetStateCommand::for_group(adapter_id, group_id, &setpoint));
        dispatch_and_wait_for_success(
            &self.publisher,
            &self.slots,
            correlation_id,
            self.timeout_ms,
            BusFrame::command(command),
        )?;
        Ok(correlation_id)
    }
}

impl ApiHandler for GroupTargetStateHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != "PUT" {
            return HttpResponse::method_not_allowed();
        }
        let validated = match self.validate(params, body) {
            Ok(validated) => validated,
            Err(error) => return error,
        };
        match self.execute(validated) {
            Ok(correlation_id) => accepted_correlation_response(correlation_id),
            Err(error) => error,
        }
    }
}

fn parse_group_patch_data(
    object: &serde_json::Map<String, Value>,
) -> Result<GroupPatchData, HttpResponse> {
    let mut data = GroupPatchData {
        patch_mask: 0,
        name: None,
        ha_flag: None,
    };
    for key in object.keys() {
        match key.as_str() {
            "name" | "ha_entity_enabled" => {}
            "capabilities_summary" | "dirty" | "member_count_desired" | "member_count_applied"
            | "group_id" | "adapter_id" => return Err(json_err(422, "unsupported_field")),
            _ => return Err(json_err(400, "unknown_field")),
        }
    }
    if let Some(value) = object.get("name") {
        let name = value.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
        if name.is_empty() || name.as_bytes().len() > 64 {
            return Err(json_err(422, "invalid_value"));
        }
        data.patch_mask |= GroupMetadataUpdateCommand::PATCH_NAME;
        data.name = Some(name.to_string());
    }
    if let Some(value) = object.get("ha_entity_enabled") {
        data.patch_mask |= GroupMetadataUpdateCommand::PATCH_HA_ENTITY_ENABLED;
        data.ha_flag = Some(
            value
                .as_bool()
                .ok_or_else(|| json_err(422, "invalid_value"))?,
        );
    }
    Ok(data)
}

fn parse_matrix_rows(value: &Value, full_replace: bool) -> Result<Vec<GroupMatrixDesiredRow>, HttpResponse> {
    let object = value.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
    for key in object.keys() {
        if key != "rows" {
            return Err(matrix_root_field_error(key));
        }
    }
    let rows = object
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if !full_replace && rows.is_empty() {
        return Err(json_err(422, "invalid_value"));
    }
    if full_replace && rows.len() != usize::from(VIRTUAL_LAMP_COUNT) {
        return Err(json_err(422, "invalid_value"));
    }
    parse_matrix_row_array(rows, full_replace)
}

fn matrix_root_field_error(key: &str) -> HttpResponse {
    match key {
        "groups" | "dirty" => json_err(422, "unsupported_field"),
        _ => json_err(400, "unknown_field"),
    }
}

fn parse_matrix_row_array(
    rows: &[Value],
    full_replace: bool,
) -> Result<Vec<GroupMatrixDesiredRow>, HttpResponse> {
    let mut parsed = Vec::with_capacity(rows.len());
    let mut seen = HashSet::with_capacity(rows.len());
    for row in rows {
        let parsed_row = parse_matrix_row(row)?;
        if !seen.insert(parsed_row.virtual_lamp_id) {
            return Err(json_err(422, "invalid_value"));
        }
        parsed.push(parsed_row);
    }
    if full_replace && seen.len() != usize::from(VIRTUAL_LAMP_COUNT) {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(parsed)
}

fn parse_matrix_row(row: &Value) -> Result<GroupMatrixDesiredRow, HttpResponse> {
    let object = row.as_object().ok_or_else(|| json_err(422, "invalid_value"))?;
    for key in object.keys() {
        match key.as_str() {
            "virtual_lamp_id" | "desired" => {}
            "applied" | "name" => return Err(json_err(422, "unsupported_field")),
            _ => return Err(json_err(400, "unknown_field")),
        }
    }
    let virtual_lamp_id = parse_virtual_lamp_id(object.get("virtual_lamp_id"))?;
    let desired_groups_mask = parse_desired_mask(object.get("desired"))?;
    Ok(GroupMatrixDesiredRow {
        virtual_lamp_id,
        desired_groups_mask,
    })
}

fn parse_virtual_lamp_id(value: Option<&Value>) -> Result<u8, HttpResponse> {
    let lamp_id = value
        .and_then(Value::as_u64)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if lamp_id >= u64::from(VIRTUAL_LAMP_COUNT) {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(lamp_id as u8)
}

fn parse_desired_mask(value: Option<&Value>) -> Result<u16, HttpResponse> {
    let desired = value
        .and_then(Value::as_array)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if desired.len() != usize::from(GROUP_COUNT) {
        return Err(json_err(422, "invalid_value"));
    }
    let mut mask = 0u16;
    for (group_id, flag) in desired.iter().enumerate() {
        let enabled = flag.as_bool().ok_or_else(|| json_err(422, "invalid_value"))?;
        if enabled {
            mask |= 1u16 << group_id;
        }
    }
    Ok(mask)
}

fn publish_group_matrix_rows(
    publisher: &BusPublisher,
    correlation: &Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    adapter_id: u8,
    rows: &[GroupMatrixDesiredRow],
    full_replace: bool,
) -> Result<String, HttpResponse> {
    let workflow = correlation.next_id();
    let mut batch = Vec::new();
    for chunk_rows in rows.chunks(MAX_GROUP_MATRIX_ROWS_PER_COMMAND) {
        let mut bounded = GroupMatrixDesiredRowList::new();
        for row in chunk_rows {
            bounded
                .push(*row)
                .map_err(|_| json_err(503, "commands_ingress_overload"))?;
        }
        let command = if full_replace {
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::GroupMatrixDesiredReplaceCommand { adapter_id, rows: bounded })
        } else {
            dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, workflow, bus_id.0, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::GroupMatrixDesiredPatchCommand { adapter_id, rows: bounded })
        };
        batch.push(BusFrame::command(command));
    }
    publish_matrix_series(
        publisher,
        bus_id,
        workflow,
        dali2rust_contracts::msg::ConfigWriteResource::GroupMatrix,
        adapter_id,
        None,
        batch,
    )
}

