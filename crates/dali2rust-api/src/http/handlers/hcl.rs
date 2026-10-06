use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_bus::{BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::{
    fixed_text_32, FixedText32, HclPointList, HclScheduleDeleteCommand, HclScheduleUpsertCommand,
    HclTargetList, CONFIG_WRITE_TTL_MS, MAX_HCL_POINTS_PER_COMMAND, MAX_HCL_TARGETS_PER_COMMAND,
};
use dali2rust_domain::registry::MAX_HCL_SCHEDULES;
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::{
    dispatch_and_wait_for_success, CorrelationIdAllocator,
};
use crate::http::hcl_state::{HclScheduleDto, HclScheduleHttpState, HclSchedulesListBody};
use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{
    accepted_config_write_response, body_parse_error, json_err, json_err_with_message,
    json_stream_dto, parse_typed_body,
};
use crate::http::handlers::hcl_validate::{
    schedule_id_path, validate_schedule, validate_schedule_id, ValidatedSchedule,
};
use crate::http::handlers::operation_dispatch::publish_begin_then_chunk_series;
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

const GENERATED_ID_PREFIX: &str = "schedule-";
const GENERATED_ID_LIMIT: u16 = 64;
const PENDING_ID_HOLD: Duration = Duration::from_millis(CONFIG_WRITE_TTL_MS as u64);
const ACCEPTED: u16 = 202;

#[derive(Clone)]
pub struct HclBusContext {
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub bus_id: BusId,
    pub timeout_ms: u64,
}

fn parse_schedule_body(body: &[u8]) -> Result<HclScheduleDto, HttpResponse> {
    let value = parse_typed_body::<serde_json::Value>(body)?;
    serde_json::from_value(value).map_err(|error| body_parse_error(&error, 400, "invalid_json"))
}

fn schedule_id_param(params: &HashMap<String, String>) -> Result<String, HttpResponse> {
    let Some(raw) = params.get("schedule_id") else {
        return Err(json_err(400, "missing_resource_id"));
    };
    schedule_id_path(raw)?;
    Ok(raw.clone())
}

declare_handler_shell! {
    HclScheduleListHandler {
        state: Arc<dyn HclScheduleHttpState>,
    }
}

impl ApiHandler for HclScheduleListHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        json_stream_dto(HclSchedulesListBody {
            schedules: self.state.list_hcl_schedule_dtos(),
        })
    }
}

declare_handler_shell! {
    HclScheduleDetailHandler {
        state: Arc<dyn HclScheduleHttpState>,
    }
}

impl ApiHandler for HclScheduleDetailHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let schedule_id = match schedule_id_param(params) {
            Ok(id) => id,
            Err(response) => return response,
        };
        match self.state.hcl_schedule_dto(&schedule_id) {
            Some(dto) => json_stream_dto(dto),
            None => json_err(404, "not_found"),
        }
    }
}

pub struct HclScheduleCreateHandler {
    state: Arc<dyn HclScheduleHttpState>,
    bus: HclBusContext,
    pending: Mutex<Vec<(FixedText32, Instant)>>,
}

impl HclScheduleCreateHandler {
    pub fn new(state: Arc<dyn HclScheduleHttpState>, bus: HclBusContext) -> Self {
        Self {
            state,
            bus,
            pending: Mutex::new(Vec::with_capacity(MAX_HCL_SCHEDULES)),
        }
    }

    fn claim_id(&self, requested: &str) -> Result<String, HttpResponse> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        pending.retain(|(id, since)| {
            now.duration_since(*since) < PENDING_ID_HOLD
                && !self.state.hcl_schedule_id_taken(id.as_str())
        });
        if pending.len() >= MAX_HCL_SCHEDULES {
            return Err(json_err_with_message(409, "conflict", "schedule_limit_reached"));
        }
        let taken = |id: &str| {
            self.state.hcl_schedule_id_taken(id) || pending.iter().any(|(held, _)| held.as_str() == id)
        };
        let id = if requested.is_empty() {
            generate_id(taken)?
        } else {
            validate_schedule_id(requested)?;
            if taken(requested) {
                return Err(json_err(409, "conflict"));
            }
            requested.to_string()
        };
        pending.push((fixed_text_32(&id), now));
        Ok(id)
    }

    fn release_id(&self, id: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|(held, _)| held.as_str() != id);
    }
}

fn generate_id(taken: impl Fn(&str) -> bool) -> Result<String, HttpResponse> {
    (1..=GENERATED_ID_LIMIT)
        .map(|n| format!("{GENERATED_ID_PREFIX}{n}"))
        .find(|candidate| !taken(candidate))
        .ok_or_else(|| json_err(409, "conflict"))
}

impl ApiHandler for HclScheduleCreateHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        let mut dto = match parse_schedule_body(body) {
            Ok(dto) => dto,
            Err(response) => return response,
        };
        dto.schedule_id = match self.claim_id(&dto.schedule_id) {
            Ok(id) => id,
            Err(response) => return response,
        };
        let response = write_schedule(&self.bus, &dto);
        if response.status != ACCEPTED {
            self.release_id(&dto.schedule_id);
        }
        response
    }
}

declare_handler_shell!(HclSchedulePatchHandler {
    state: Arc<dyn HclScheduleHttpState>,
    bus: HclBusContext,
});

impl ApiHandler for HclSchedulePatchHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let schedule_id = match schedule_id_param(params) {
            Ok(id) => id,
            Err(response) => return response,
        };
        let Some(current) = self.state.hcl_schedule_dto(&schedule_id) else {
            return json_err(404, "not_found");
        };
        let patched = match merge_patch_schedule(&current, body) {
            Ok(dto) => dto,
            Err(response) => return response,
        };
        write_schedule(&self.bus, &patched)
    }
}

fn merge_patch_schedule(current: &HclScheduleDto, body: &[u8]) -> Result<HclScheduleDto, HttpResponse> {
    let patch = parse_typed_body::<serde_json::Value>(body)?;
    if !patch.is_object() {
        return Err(json_err(400, "invalid_json"));
    }
    if let Some(patched_id) = patch.get("schedule_id").and_then(|v| v.as_str()) {
        if patched_id != current.schedule_id {
            return Err(json_err(422, "unsupported_field"));
        }
    }
    let mut merged = serde_json::to_value(current).map_err(|_| json_err(500, "internal_error"))?;
    apply_merge_patch(&mut merged, &patch);
    serde_json::from_value(merged).map_err(|error| body_parse_error(&error, 422, "invalid_value"))
}

fn apply_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let Some(patch_map) = patch.as_object() else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(target_map) = target.as_object_mut() else {
        return;
    };
    for (key, value) in patch_map {
        if value.is_null() {
            target_map.remove(key);
            continue;
        }
        apply_merge_patch(target_map.entry(key.clone()).or_insert(serde_json::Value::Null), value);
    }
}

declare_handler_shell! {
    HclScheduleDeleteHandler {
        state: Arc<dyn HclScheduleHttpState>,
        bus: HclBusContext,
    }
}

impl ApiHandler for HclScheduleDeleteHandler {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        _body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        let schedule_id = match schedule_id_param(params) {
            Ok(id) => id,
            Err(response) => return response,
        };
        if self.state.hcl_schedule_dto(&schedule_id).is_none() {
            return json_err(404, "not_found");
        }
        let correlation_id = self.bus.correlation.next_id();
        let command = dali2rust_contracts::bus::command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus.bus_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            HclScheduleDeleteCommand {
                schedule_id: dali2rust_contracts::msg::fixed_text_32(&schedule_id),
            },
        );
        if let Err(response) = dispatch_and_wait_for_success(
            &self.bus.publisher,
            &self.bus.slots,
            correlation_id,
            self.bus.timeout_ms,
            BusFrame::command(command),
        ) {
            return response;
        }
        HttpResponse::no_content()
    }
}

fn write_schedule(
    bus: &HclBusContext,
    dto: &HclScheduleDto,
) -> HttpResponse {
    let validated = match validate_schedule(dto) {
        Ok(validated) => validated,
        Err(response) => return response,
    };
    match publish_schedule_chunks(bus, &validated) {
        Ok(op_key) => accepted_config_write_response(op_key, &dto.schedule_id),
        Err(response) => response,
    }
}

fn publish_schedule_chunks(
    bus: &HclBusContext,
    schedule: &ValidatedSchedule,
) -> Result<String, HttpResponse> {
    let chunk_count = chunk_count(schedule);
    let mut batch = Vec::with_capacity(chunk_count);
    for index in 0..chunk_count {
        let (first_target_index, targets) =
            chunk_slice(&schedule.targets, index, MAX_HCL_TARGETS_PER_COMMAND);
        let (first_point_index, points) =
            chunk_slice(&schedule.points, index, MAX_HCL_POINTS_PER_COMMAND);
        let command = HclScheduleUpsertCommand {
            schedule_id: schedule.schedule_id.clone(),
            enabled: schedule.enabled,
            algorithm: schedule.algorithm,
            active_days_mask: schedule.active_days_mask,
            latitude_microdeg: schedule.latitude_microdeg,
            longitude_microdeg: schedule.longitude_microdeg,
            first_target_index,
            targets: HclTargetList::from_slice(targets)
                .map_err(|_| json_err(500, "internal_error"))?,
            first_point_index,
            points: HclPointList::from_slice(points).map_err(|_| json_err(500, "internal_error"))?,
            last_chunk: index + 1 == chunk_count,
        };
        batch.push(chunk_entry(bus, command));
    }
    let workflow = batch.last().expect("chunk_count is at least 1").0;
    let op_key = format!("cfg-hcl-{workflow}");
    let frames = batch.into_iter().map(|(_, frame)| frame).collect();
    publish_begin_then_chunk_series(&bus.publisher, bus.bus_id, workflow, &op_key, frames)?;
    Ok(op_key)
}

fn chunk_entry(bus: &HclBusContext, command: HclScheduleUpsertCommand) -> (u64, BusFrame) {
    let correlation_id = bus.correlation.next_id();
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        bus.bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        command,
    );
    (correlation_id, BusFrame::command(envelope))
}

fn chunk_count(schedule: &ValidatedSchedule) -> usize {
    let target_chunks = schedule.targets.len().div_ceil(MAX_HCL_TARGETS_PER_COMMAND);
    let point_chunks = schedule.points.len().div_ceil(MAX_HCL_POINTS_PER_COMMAND);
    target_chunks.max(point_chunks).max(1)
}

fn chunk_slice<T>(rows: &[T], index: usize, per_chunk: usize) -> (u8, &[T]) {
    let start = (index * per_chunk).min(rows.len());
    let end = (start + per_chunk).min(rows.len());
    (start as u8, &rows[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_bus::{BusConfig, BusHost};
    use dali2rust_contracts::msg::{HclAlgorithm, HclTargetRow, HclTargetScope};

    #[derive(Default)]
    struct RegistryNotYetStaged {
        taken: Mutex<Vec<String>>,
    }

    impl RegistryNotYetStaged {
        fn set_taken(&self, id: &str, taken: bool) {
            let mut ids = self.taken.lock().expect("ids");
            ids.retain(|held| held != id);
            if taken {
                ids.push(id.to_string());
            }
        }
    }

    impl HclScheduleHttpState for RegistryNotYetStaged {
        fn hcl_schedule_dto(&self, _schedule_id: &str) -> Option<HclScheduleDto> {
            None
        }

        fn list_hcl_schedule_dtos(&self) -> Vec<HclScheduleDto> {
            Vec::new()
        }

        fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool {
            self.taken.lock().expect("ids").iter().any(|held| held == schedule_id)
        }
    }

    fn create_handler(state: Arc<RegistryNotYetStaged>) -> (BusHost, HclScheduleCreateHandler) {
        let (host, publisher, ()) = BusHost::spawn(BusConfig::default(), |reg| {
            let _ = reg.subscribe_commands(64, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
        });
        let bus = HclBusContext {
            publisher,
            slots: Arc::new(PendingConfirmationSlots::with_capacity(4)),
            correlation: Arc::new(CorrelationIdAllocator::new()),
            bus_id: BusId(1),
            timeout_ms: 1000,
        };
        (host, HclScheduleCreateHandler::new(state, bus))
    }

    fn post(handler: &HclScheduleCreateHandler, schedule_id: Option<&str>) -> (u16, serde_json::Value) {
        let mut body = serde_json::json!({
            "enabled": true,
            "algorithm": "stepped",
            "active_days": ["mon"],
            "location": null,
            "targets": [{ "adapter_id": 0, "scope": "group", "group_ids": [1] }],
            "points": [{ "time_ref": "absolute", "offset_minutes": 360, "level_mode": "absolute",
                         "level": 80, "color_temperature_kelvin": 2700 }],
        });
        if let Some(id) = schedule_id {
            body["schedule_id"] = id.into();
        }
        let response =
            handler.handle_request("POST", "", body.to_string().as_bytes(), &HashMap::new());
        let status = response.status;
        let json = serde_json::from_slice(&response.into_body_bytes()).expect("a JSON body");
        (status, json)
    }

    #[test]
    fn two_quick_posts_without_an_id_get_two_ids() {
        let (_host, handler) = create_handler(Arc::default());
        let (first, second) = (post(&handler, None), post(&handler, None));
        assert_eq!((first.0, second.0), (202, 202));
        assert_eq!(first.1["schedule_id"], "schedule-1");
        assert_eq!(second.1["schedule_id"], "schedule-2");
    }

    #[test]
    fn a_second_quick_post_of_one_id_is_a_conflict() {
        let (_host, handler) = create_handler(Arc::default());
        assert_eq!(post(&handler, Some("morning")).0, 202);
        let (status, json) = post(&handler, Some("morning"));
        assert_eq!(status, 409);
        assert_eq!(json["error"], "conflict");
    }

    #[test]
    fn a_name_the_registry_took_over_is_free_again_once_the_registry_lets_it_go() {
        let state = Arc::new(RegistryNotYetStaged::default());
        let (_host, handler) = create_handler(Arc::clone(&state));
        assert_eq!(post(&handler, Some("morning")).0, 202);
        state.set_taken("morning", true);
        assert_eq!(post(&handler, Some("morning")).0, 409);
        state.set_taken("morning", false);
        assert_eq!(post(&handler, Some("morning")).0, 202);
    }

    #[test]
    fn a_refused_body_holds_no_name() {
        let (_host, handler) = create_handler(Arc::default());
        let refused = handler.handle_request(
            "POST",
            "",
            br#"{"schedule_id":"morning","algorithm":"unheard_of","enabled":true,"active_days":["mon"],"targets":[],"points":[]}"#,
            &HashMap::new(),
        );
        assert_eq!(refused.status, 422);
        assert_eq!(post(&handler, Some("morning")).0, 202);
    }

    #[test]
    fn creations_in_flight_stop_at_the_schedule_limit() {
        let (_host, handler) = create_handler(Arc::default());
        for _ in 0..MAX_HCL_SCHEDULES {
            assert_eq!(post(&handler, None).0, 202);
        }
        let (status, json) = post(&handler, None);
        assert_eq!(status, 409);
        assert_eq!(json["message"], "schedule_limit_reached");
    }

    fn schedule_with(target_count: usize, point_count: usize) -> ValidatedSchedule {
        ValidatedSchedule {
            schedule_id: fixed_text_32("morning"),
            enabled: true,
            algorithm: HclAlgorithm::Stepped,
            active_days_mask: 0b0111_1111,
            latitude_microdeg: None,
            longitude_microdeg: None,
            targets: (0..target_count)
                .map(|i| HclTargetRow {
                    adapter_id: i as u8,
                    scope: HclTargetScope::Group,
                    group_mask: 1,
                })
                .collect(),
            points: (0..point_count)
                .map(|i| dali2rust_contracts::msg::HclSchedulePointRow {
                    time_ref: dali2rust_contracts::msg::HclTimeRef::Absolute,
                    offset_minutes: i as i16,
                    level_mode: dali2rust_contracts::msg::HclLevelMode::None,
                    level: None,
                    color_temperature_kelvin: Some(2700),
                })
                .collect(),
        }
    }

    fn assert_chunk_plan_reassembles(target_count: usize, point_count: usize) {
        let schedule = schedule_with(target_count, point_count);
        let (mut targets, mut points) = (0usize, 0usize);
        for index in 0..chunk_count(&schedule) {
            let (first_target, target_rows) =
                chunk_slice(&schedule.targets, index, MAX_HCL_TARGETS_PER_COMMAND);
            let (first_point, point_rows) =
                chunk_slice(&schedule.points, index, MAX_HCL_POINTS_PER_COMMAND);
            assert_eq!(usize::from(first_target), targets, "target index continuity");
            assert_eq!(usize::from(first_point), points, "point index continuity");
            assert!(target_rows.len() <= MAX_HCL_TARGETS_PER_COMMAND);
            assert!(point_rows.len() <= MAX_HCL_POINTS_PER_COMMAND);
            targets += target_rows.len();
            points += point_rows.len();
        }
        assert_eq!(targets, target_count, "every target lands in exactly one chunk");
        assert_eq!(points, point_count, "every point lands in exactly one chunk");
    }

    #[test]
    fn chunk_plan_covers_every_shape_the_contract_allows() {
        for targets in [1usize, 4, 5, 16] {
            for points in [1usize, 2, 3, 24] {
                assert_chunk_plan_reassembles(targets, points);
            }
        }
    }

    #[test]
    fn a_full_schedule_is_paced_by_its_points() {
        assert_eq!(chunk_count(&schedule_with(16, 24)), 12);
        assert_eq!(chunk_count(&schedule_with(16, 1)), 4);
        assert_eq!(chunk_count(&schedule_with(1, 1)), 1);
    }
}
