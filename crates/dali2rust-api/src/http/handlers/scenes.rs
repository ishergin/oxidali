use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliSceneTargetState, OperationType, PowerState, SceneMatrixDesiredRow,
    SceneMatrixDesiredRowList, SceneMetadataUpdateCommand, MAX_SCENE_MATRIX_ROWS_PER_COMMAND,
};
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::Value;

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::{
    dispatch_and_wait_for_success, CorrelationIdAllocator,
};
use crate::http::handler::ApiHandler;
use crate::http::handlers::resource_surface::{
    declare_handler_shell, declare_matrix_write_handler, declare_metadata_patch_handler,
    declare_read_handler,
};
use crate::http::handlers::common::{
    publish_matrix_series,
    accepted_operation_response, cap_supports_color_mode, json_err, json_stream_dto, parse_adapter_id,
    parse_json_body, parse_resource_id_param, parse_typed_body, serialize_or_log,
    write_json_array_items,
};
use crate::http::lenient::{MaybeBool, MaybeObj, MaybeSeq, MaybeU64};
use crate::http::physical_device_state::CapabilityFlagsDto;
use crate::http::scene_state::{
    SceneDto, SceneHttpState, SceneMatrixDto, SceneMetadataApplyWatch,
    ScenesListBody,
};
use crate::http::target_state_request::{
    apply_setpoint_fields, SetpointFields, TARGET_STATE_RUNTIME_READONLY_KEYS,
};
use crate::http::types::HttpResponse;
use crate::http::handlers::common::{publish_apply_execute, reject_if_apply_active};
use dali2rust_domain::registry::OperationReadPort;

use dali2rust_domain::registry::{SCENE_COUNT, VIRTUAL_LAMP_COUNT};
const SCENE_DESIRED_SETPOINT_KEYS: &[&str] = &[
    "power",
    "level",
    "color_mode",
    "color_temperature_kelvin",
    "xy",
    "rgb",
    "rgbwaf",
];

fn parse_scene_id(params: &HashMap<String, String>) -> Result<u8, HttpResponse> {
    parse_resource_id_param(params, "scene_id", u16::from(SCENE_COUNT) - 1)
}

fn list_response(adapter_id: u8, scenes: Vec<SceneDto>) -> HttpResponse {
    json_stream_dto(ScenesListBody { adapter_id, scenes })
}

fn matrix_stream_response(dto: SceneMatrixDto) -> HttpResponse {
    HttpResponse::json_stream(
        200,
        Box::new(move |w: &mut dyn Write| {
            write!(
                w,
                "{{\"adapter_id\":{},\"scene_id\":{},\"rows\":[",
                dto.adapter_id, dto.scene_id
            )?;
            write_json_array_items(w, dto.rows.iter())?;
            w.write_all(b"]}")?;
            Ok(())
        }),
    )
}

declare_read_handler! {
    ScenesListHandler,
    state: SceneHttpState,
    respond: |state: &Arc<dyn SceneHttpState>, adapter_id| {
        list_response(adapter_id, state.list_scene_dtos(adapter_id))
    },
}

declare_read_handler! {
    SceneGetHandler,
    state: SceneHttpState,
    id: parse_scene_id,
    respond: |state: &Arc<dyn SceneHttpState>, adapter_id, scene_id| {
        match state.scene_dto(adapter_id, scene_id) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_read_handler! {
    SceneMatrixGetHandler,
    state: SceneHttpState,
    id: parse_scene_id,
    respond: |state: &Arc<dyn SceneHttpState>, adapter_id, scene_id| {
        match state.scene_matrix_dto(adapter_id, scene_id) {
            Some(dto) => matrix_stream_response(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_metadata_patch_handler! {
    ScenePatchHandler,
    state: SceneHttpState,
    watch: SceneMetadataApplyWatch,
    watch_load: scene_metadata_applied_load,
    data: ScenePatchData,
    id: parse_scene_id,
    parse: parse_scene_patch_data,
    command: SceneMetadataUpdateCommand { scene_id, ha_select_enabled },
    origin: Some(dali2rust_contracts::msg::Origin::Api),
    echo: |state: &Arc<dyn SceneHttpState>, adapter_id, scene_id| {
        match state.scene_dto(adapter_id, scene_id) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "not_found"),
        }
    },
}

declare_matrix_write_handler! {
    SceneMatrixWriteHandler,
    state: SceneHttpState,
}

impl SceneMatrixWriteHandler {
    fn row_capabilities(
        &self,
        adapter_id: u8,
        scene_id: u8,
    ) -> Result<HashMap<u8, CapabilityFlagsDto>, HttpResponse> {
        let dto = self
            .state
            .scene_matrix_dto(adapter_id, scene_id)
            .ok_or_else(|| json_err(404, "not_found"))?;
        Ok(dto
            .rows
            .into_iter()
            .map(|row| (row.virtual_lamp_id, row.capabilities))
            .collect())
    }
}

impl crate::http::handlers::common::MutatingHandler for SceneMatrixWriteHandler {
    type Validated = (u8, u8, Vec<SceneMatrixDesiredRow>);
    type Executed = String;

    fn expected_method(&self) -> &'static str {
        self.mode.expected_method()
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(u8, u8, Vec<SceneMatrixDesiredRow>), HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        let scene_id = parse_scene_id(params)?;
        let caps = self.row_capabilities(adapter_id, scene_id)?;
        let request = parse_typed_body::<SceneMatrixBody>(body)?;
        let rows = parse_scene_matrix_rows(&request, self.mode.full_replace(), &caps)?;
        Ok((adapter_id, scene_id, rows))
    }

    fn execute(&self, args: (u8, u8, Vec<SceneMatrixDesiredRow>)) -> Result<String, HttpResponse> {
        let (adapter_id, scene_id, rows) = args;
        publish_scene_matrix_rows(
            &self.publisher,
            &self.correlation,
            self.bus_id,
            adapter_id,
            scene_id,
            &rows,
            self.mode.full_replace(),
        )
    }

    fn respond(&self, op_key: String) -> HttpResponse {
        accepted_operation_response(op_key, OperationType::ConfigWrite)
    }
}

declare_handler_shell! {
    SceneApplyHandler {
        publisher: BusPublisher,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn SceneHttpState>,
        operations: Arc<dyn OperationReadPort>,
        bus_id: BusId,
    }
}

impl crate::http::handlers::common::MutatingHandler for SceneApplyHandler {
    type Validated = (u8, u8, bool);
    type Executed = HttpResponse;

    fn expected_method(&self) -> &'static str {
        "POST"
    }

    fn validate(
        &self,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(u8, u8, bool), HttpResponse> {
        let adapter_id = parse_adapter_id(self.state.adapter_count(), params)?;
        let scene_id = parse_scene_id(params)?;
        reject_if_apply_active(
            self.operations.as_ref(),
            OperationType::SceneApply,
            adapter_id,
        )?;
        let snapshot = self
            .state
            .scene_apply_snapshot(adapter_id, scene_id)
            .ok_or_else(|| json_err(404, "not_found"))?;
        let diff_is_empty =
            dali2rust_domain::registry::collect_scene_apply_diff(&snapshot).is_empty();
        Ok((adapter_id, scene_id, diff_is_empty))
    }

    fn execute(&self, args: (u8, u8, bool)) -> Result<HttpResponse, HttpResponse> {
        let (adapter_id, scene_id, diff_is_empty) = args;
        if diff_is_empty {
            let dto = self
                .state
                .scene_matrix_dto(adapter_id, scene_id)
                .ok_or_else(|| json_err(404, "not_found"))?;
            return Ok(matrix_stream_response(dto));
        }
        let workflow = self.correlation.next_id();
        let operation_id = format!("scn-apply-{adapter_id}-{scene_id}-{workflow}");
        let execute = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            workflow,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            dali2rust_contracts::msg::SceneApplyExecuteCommand {
                registry_adapter_id: adapter_id,
                scene_id,
                operation_key: dali2rust_contracts::msg::fixed_text_32(&operation_id),
            },
        );
        publish_apply_execute(
            &self.publisher,
            OperationType::SceneApply,
            operation_id,
            execute,
        )
    }

    fn respond(&self, executed: HttpResponse) -> HttpResponse {
        executed
    }
}

declare_handler_shell! {
    SceneRecallHandler {
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        state: Arc<dyn SceneHttpState>,
        bus_id: BusId,
        timeout_ms: u64,
    }
}

impl SceneRecallHandler {
    fn execute(
        &self,
        command: dali2rust_contracts::msg::DaliRecallSceneCommand,
    ) -> Result<u64, HttpResponse> {
        let correlation_id = self.correlation.next_id();
        let command = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            command,
        );
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

fn parse_recall_scope(
    body: &[u8],
    adapter_id: u8,
    scene_id: u8,
) -> Result<dali2rust_contracts::msg::DaliRecallSceneCommand, HttpResponse> {
    use dali2rust_contracts::msg::DaliRecallSceneCommand;
    if body.is_empty() {
        return Ok(DaliRecallSceneCommand::broadcast(adapter_id, scene_id));
    }
    let v = parse_json_body(body)?;
    let obj = v.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;
    if obj.keys().any(|k| k != "scope" && k != "group_id") {
        return Err(json_err(422, "unsupported_field"));
    }
    match recall_scope_str(obj)? {
        None | Some("broadcast") if !obj.contains_key("group_id") => {
            Ok(DaliRecallSceneCommand::broadcast(adapter_id, scene_id))
        }
        Some("group") => {
            let group_id = obj
                .get("group_id")
                .and_then(|g| g.as_u64())
                .filter(|g| *g <= 15)
                .ok_or_else(|| json_err(422, "invalid_value"))?;
            Ok(DaliRecallSceneCommand::for_group(adapter_id, group_id as u8, scene_id))
        }
        _ => Err(json_err(422, "invalid_value")),
    }
}

fn recall_scope_str(
    obj: &serde_json::Map<String, Value>,
) -> Result<Option<&str>, HttpResponse> {
    match obj.get("scope") {
        None => Ok(None),
        Some(s) => s.as_str().map(Some).ok_or_else(|| json_err(422, "invalid_value")),
    }
}

impl ApiHandler for SceneRecallHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if method != "POST" {
            return HttpResponse::method_not_allowed();
        }
        let adapter_id = match parse_adapter_id(self.state.adapter_count(), params) {
            Ok(adapter_id) => adapter_id,
            Err(error) => return error,
        };
        let scene_id = match parse_scene_id(params) {
            Ok(scene_id) => scene_id,
            Err(error) => return error,
        };
        let command = match parse_recall_scope(body, adapter_id, scene_id) {
            Ok(command) => command,
            Err(error) => return error,
        };
        match self.execute(command) {
            Ok(correlation_id) => HttpResponse::json(
                200,
                serialize_or_log(&serde_json::json!({
                    "correlation_id": correlation_id,
                    "status": "confirmed",
                })),
            ),
            Err(error) => error,
        }
    }
}

fn parse_scene_patch_data(
    object: &serde_json::Map<String, Value>,
) -> Result<ScenePatchData, HttpResponse> {
    let mut data = ScenePatchData {
        patch_mask: 0,
        name: None,
        ha_flag: None,
    };
    for key in object.keys() {
        match key.as_str() {
            "name" | "ha_select_enabled" => {}
            "row_count_included" | "dirty" | "scene_id" | "adapter_id" => {
                return Err(json_err(422, "unsupported_field"))
            }
            _ => return Err(json_err(400, "unknown_field")),
        }
    }
    if let Some(value) = object.get("name") {
        let name = value.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
        if name.is_empty() || name.as_bytes().len() > 64 {
            return Err(json_err(422, "invalid_value"));
        }
        data.patch_mask |= SceneMetadataUpdateCommand::PATCH_NAME;
        data.name = Some(name.to_string());
    }
    if let Some(value) = object.get("ha_select_enabled") {
        data.patch_mask |= SceneMetadataUpdateCommand::PATCH_HA_SELECT_ENABLED;
        data.ha_flag = Some(
            value
                .as_bool()
                .ok_or_else(|| json_err(422, "invalid_value"))?,
        );
    }
    Ok(data)
}

#[derive(Debug, Deserialize)]
struct SceneMatrixBody {
    rows: Option<MaybeSeq<MaybeObj<SceneRowBody>>>,
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

#[derive(Debug, Deserialize)]
struct SceneRowBody {
    virtual_lamp_id: Option<MaybeU64>,
    desired: Option<MaybeObj<SceneDesiredBody>>,
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

#[derive(Debug, Deserialize)]
struct SceneDesiredBody {
    included: Option<MaybeBool>,
    #[serde(flatten)]
    setpoint: SetpointFields,
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

fn parse_scene_matrix_rows(
    request: &SceneMatrixBody,
    full_replace: bool,
    caps: &HashMap<u8, CapabilityFlagsDto>,
) -> Result<Vec<SceneMatrixDesiredRow>, HttpResponse> {
    validate_scene_matrix_root_keys(&request.extra)?;
    let rows = request
        .rows
        .as_ref()
        .and_then(MaybeSeq::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if !full_replace && rows.is_empty() {
        return Err(json_err(422, "invalid_value"));
    }
    if full_replace && rows.len() != usize::from(VIRTUAL_LAMP_COUNT) {
        return Err(json_err(422, "invalid_value"));
    }
    let mut parsed = Vec::with_capacity(rows.len());
    let mut seen = HashSet::with_capacity(rows.len());
    for row in rows {
        let parsed_row = parse_scene_row(row, caps)?;
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

fn validate_scene_matrix_root_keys(extra: &BTreeMap<String, IgnoredAny>) -> Result<(), HttpResponse> {
    let Some(key) = extra.keys().next() else {
        return Ok(());
    };
    Err(match key.as_str() {
        "adapter_id" | "scene_id" => json_err(422, "unsupported_field"),
        _ => json_err(400, "unknown_field"),
    })
}

fn parse_scene_row(
    row: &MaybeObj<SceneRowBody>,
    caps: &HashMap<u8, CapabilityFlagsDto>,
) -> Result<SceneMatrixDesiredRow, HttpResponse> {
    let MaybeObj::Valid(row) = row else {
        return Err(json_err(422, "invalid_value"));
    };
    validate_scene_row_keys(&row.extra)?;
    let virtual_lamp_id = row
        .virtual_lamp_id
        .as_ref()
        .and_then(MaybeU64::valid)
        .filter(|id| *id < u64::from(VIRTUAL_LAMP_COUNT))
        .ok_or_else(|| json_err(422, "invalid_value"))? as u8;
    let desired = row
        .desired
        .as_ref()
        .and_then(MaybeObj::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    parse_scene_desired(virtual_lamp_id, desired, caps)
}

fn validate_scene_row_keys(extra: &BTreeMap<String, IgnoredAny>) -> Result<(), HttpResponse> {
    let Some(key) = extra.keys().next() else {
        return Ok(());
    };
    Err(match key.as_str() {
        "applied" | "capabilities" | "dirty" | "name" => json_err(422, "unsupported_field"),
        _ => json_err(400, "unknown_field"),
    })
}

fn classify_scene_desired_key(key: &str) -> Result<(), HttpResponse> {
    if key == "included" || SCENE_DESIRED_SETPOINT_KEYS.contains(&key) {
        Ok(())
    } else if key == "transition" || TARGET_STATE_RUNTIME_READONLY_KEYS.contains(&key) {
        Err(json_err(422, "unsupported_field"))
    } else {
        Err(json_err(400, "unknown_field"))
    }
}

fn parse_scene_desired(
    virtual_lamp_id: u8,
    desired: &SceneDesiredBody,
    caps: &HashMap<u8, CapabilityFlagsDto>,
) -> Result<SceneMatrixDesiredRow, HttpResponse> {
    for key in desired.extra.keys() {
        classify_scene_desired_key(key.as_str())?;
    }
    let included = match &desired.included {
        Some(MaybeBool::Valid(flag)) => *flag,
        _ => return Err(json_err(422, "invalid_value")),
    };
    if !included {
        if desired.setpoint.has_any_value() {
            return Err(json_err(422, "invalid_value"));
        }
        return Ok(SceneMatrixDesiredRow {
            virtual_lamp_id,
            included: false,
            target: None,
        });
    }
    let target = parse_scene_target(virtual_lamp_id, desired, caps)?;
    Ok(SceneMatrixDesiredRow {
        virtual_lamp_id,
        included: true,
        target: Some(target),
    })
}

fn parse_scene_target(
    virtual_lamp_id: u8,
    desired: &SceneDesiredBody,
    caps: &HashMap<u8, CapabilityFlagsDto>,
) -> Result<DaliSceneTargetState, HttpResponse> {
    let fields = &desired.setpoint;
    let has_power = fields.power_present();
    let has_level = fields.level_present();
    let setpoint = apply_setpoint_fields(fields, |mode| {
        caps.get(&virtual_lamp_id)
            .map(|cap| cap_supports_color_mode(cap, mode))
            .unwrap_or(false)
    })?;
    let power = has_power.then_some(setpoint.power);
    if power == Some(PowerState::Unknown) {
        return Err(json_err(422, "invalid_value"));
    }
    let level = normalized_scene_level(power, has_level.then_some(setpoint.level))?;
    let color = setpoint.color.filter(|c| is_storable_scene_mode(c.mode));
    if let Some(color) = color.as_ref() {
        validate_color_value_present(color, fields)?;
    }
    Ok(DaliSceneTargetState {
        power,
        level,
        color,
    })
}

fn is_storable_scene_mode(mode: ColorMode) -> bool {
    matches!(
        mode,
        ColorMode::Cct | ColorMode::Xy | ColorMode::Rgb | ColorMode::Rgbwaf
    )
}

fn normalized_scene_level(
    power: Option<PowerState>,
    level: Option<u8>,
) -> Result<Option<u8>, HttpResponse> {
    if power == Some(PowerState::Off) {
        if level.unwrap_or(0) != 0 {
            return Err(json_err(422, "invalid_value"));
        }
        return Ok(Some(0));
    }
    if level.is_none() {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(level)
}

fn validate_color_value_present(
    color: &ColorValue,
    fields: &SetpointFields,
) -> Result<(), HttpResponse> {
    if !is_storable_scene_mode(color.mode) {
        return Ok(());
    }
    if !fields.color_value_present(color.mode) {
        return Err(json_err(422, "invalid_value"));
    }
    if color.mode == ColorMode::Cct && color.color_temperature_kelvin == 0 {
        return Err(json_err(422, "invalid_value"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, reason = "mirrors the group chunk publisher")]
fn publish_scene_matrix_rows(
    publisher: &BusPublisher,
    correlation: &Arc<CorrelationIdAllocator>,
    bus_id: BusId,
    adapter_id: u8,
    scene_id: u8,
    rows: &[SceneMatrixDesiredRow],
    full_replace: bool,
) -> Result<String, HttpResponse> {
    let workflow = correlation.next_id();
    let frames =
        scene_matrix_chunk_batch(workflow, bus_id, adapter_id, scene_id, rows, full_replace)?;
    publish_matrix_series(
        publisher,
        bus_id,
        workflow,
        dali2rust_contracts::msg::ConfigWriteResource::SceneMatrix,
        adapter_id,
        Some(scene_id),
        frames,
    )
}

fn scene_matrix_chunk_batch(
    correlation_id: u64,
    bus_id: BusId,
    adapter_id: u8,
    scene_id: u8,
    rows: &[SceneMatrixDesiredRow],
    full_replace: bool,
) -> Result<Vec<BusFrame>, HttpResponse> {
    let mut batch = Vec::new();
    for chunk_rows in rows.chunks(MAX_SCENE_MATRIX_ROWS_PER_COMMAND) {
        let mut bounded = SceneMatrixDesiredRowList::new();
        for row in chunk_rows {
            bounded
                .push(*row)
                .map_err(|_| json_err(503, "commands_ingress_overload"))?;
        }
        let command = if full_replace {
            dali2rust_contracts::bus::command_envelope(
                SOURCE_ID_UNSPECIFIED,
                correlation_id,
                bus_id.0,
                Some(dali2rust_contracts::msg::Origin::Api),
                dali2rust_contracts::msg::SceneMatrixDesiredReplaceCommand {
                    adapter_id,
                    scene_id,
                    rows: bounded,
                },
            )
        } else {
            dali2rust_contracts::bus::command_envelope(
                SOURCE_ID_UNSPECIFIED,
                correlation_id,
                bus_id.0,
                Some(dali2rust_contracts::msg::Origin::Api),
                dali2rust_contracts::msg::SceneMatrixDesiredPatchCommand {
                    adapter_id,
                    scene_id,
                    rows: bounded,
                },
            )
        };
        batch.push(BusFrame::command(command));
    }
    Ok(batch)
}
