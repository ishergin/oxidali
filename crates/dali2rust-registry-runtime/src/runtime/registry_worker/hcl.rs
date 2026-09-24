use std::sync::atomic::Ordering;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    ErrorCode, HclLevelMode, HclSchedulePointRow, HclScheduleDeleteCommand,
    HclScheduleUpsertCommand, HclTargetRow, HclTargetScope, HclTimeRef,
};

use crate::runtime::registry::hcl_schedules::{HclChunkOutcome, HclChunkRejection};
use crate::runtime::registry::publish::{publish_config_write_signal, publish_hcl_schedule_changed};
use crate::runtime::registry::RegistryStore;

use super::confirm::{check_primary_adapter, publish_correlation_failed, publish_correlation_ok};
use super::RegistryCommandCounters;

const MAX_ABSOLUTE_OFFSET_MINUTES: i16 = 1439;
const MAX_ASTRONOMICAL_OFFSET_MINUTES: i16 = 720;
const MAX_LEVEL: u8 = 254;

type Rejection = (ErrorCode, &'static str);

pub(super) fn handle_hcl_schedule_upsert(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HclScheduleUpsertCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err() {
        return;
    }
    let terminal = body.last_chunk;
    if let Err((code, message)) = validate_upsert_chunk(body) {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(publisher, corr, code, message);
        if terminal {
            publish_config_write_signal(publisher, corr, primary_adapter_id, Some((code, message)), counters);
        }
        return;
    }
    match store.apply_hcl_schedule_chunk(&body.schedule_id, body) {
        Ok(HclChunkOutcome::Staged) => publish_correlation_ok(publisher, corr),
        Ok(HclChunkOutcome::Committed) => {
            publish_hcl_schedule_changed(
                publisher,
                corr,
                tid,
                body.schedule_id.clone(),
                false,
                body.enabled,
            );
            counters
                .hcl_schedule_upserts_applied
                .fetch_add(1, Ordering::Relaxed);
            counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
            publish_correlation_ok(publisher, corr);
            publish_config_write_signal(publisher, corr, primary_adapter_id, None, counters);
        }
        Err(rejection) => {
            counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
            let (code, message) = chunk_rejection_error(rejection);
            publish_correlation_failed(publisher, corr, code, message);
            if terminal {
                publish_config_write_signal(publisher, corr, primary_adapter_id, Some((code, message)), counters);
            }
        }
    }
}

pub(super) fn handle_hcl_schedule_delete(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HclScheduleDeleteCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err() {
        return;
    }
    if !store.remove_hcl_schedule(&body.schedule_id) {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(publisher, corr, ErrorCode::NotFound, "schedule_not_found");
        return;
    }
    publish_hcl_schedule_changed(publisher, corr, tid, body.schedule_id.clone(), true, false);
    counters
        .hcl_schedule_deletes_applied
        .fetch_add(1, Ordering::Relaxed);
    counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

fn validate_upsert_chunk(body: &HclScheduleUpsertCommand) -> Result<(), Rejection> {
    if body.schedule_id.is_empty() {
        return Err((ErrorCode::InvalidResourceId, "schedule_id_empty"));
    }
    if body.active_days_mask == 0 {
        return Err((ErrorCode::InvalidValue, "active_days_empty"));
    }
    for target in body.targets.iter() {
        validate_target(target)?;
    }
    let has_location = body.latitude_microdeg.is_some() && body.longitude_microdeg.is_some();
    for point in body.points.iter() {
        validate_point(point, has_location)?;
    }
    Ok(())
}

fn validate_target(target: &HclTargetRow) -> Result<(), Rejection> {
    if target.scope == HclTargetScope::Group && target.group_mask == 0 {
        return Err((ErrorCode::InvalidValue, "target_group_mask_empty"));
    }
    Ok(())
}

fn validate_point(point: &HclSchedulePointRow, has_location: bool) -> Result<(), Rejection> {
    if point.time_ref != HclTimeRef::Absolute && !has_location {
        return Err((ErrorCode::InvalidValue, "point_needs_location"));
    }
    if !offset_in_range(point) {
        return Err((ErrorCode::InvalidValue, "point_offset_out_of_range"));
    }
    if !level_matches_mode(point) {
        return Err((ErrorCode::InvalidValue, "point_level_invalid"));
    }
    Ok(())
}

fn offset_in_range(point: &HclSchedulePointRow) -> bool {
    match point.time_ref {
        HclTimeRef::Absolute => (0..=MAX_ABSOLUTE_OFFSET_MINUTES).contains(&point.offset_minutes),
        HclTimeRef::Sunrise | HclTimeRef::Sunset => {
            (-MAX_ASTRONOMICAL_OFFSET_MINUTES..=MAX_ASTRONOMICAL_OFFSET_MINUTES)
                .contains(&point.offset_minutes)
        }
    }
}

fn level_matches_mode(point: &HclSchedulePointRow) -> bool {
    match point.level_mode {
        HclLevelMode::Absolute => point.level.is_some_and(|level| level <= MAX_LEVEL),
        HclLevelMode::LastActive | HclLevelMode::None => point.level.is_none(),
    }
}

fn chunk_rejection_error(rejection: HclChunkRejection) -> Rejection {
    match rejection {
        HclChunkRejection::OutOfOrder => (ErrorCode::Conflict, "chunk_out_of_order"),
        HclChunkRejection::TooManyRows => (ErrorCode::InvalidValue, "schedule_rows_exceeded"),
        HclChunkRejection::ScheduleLimit => (ErrorCode::Conflict, "schedule_limit_reached"),
    }
}
