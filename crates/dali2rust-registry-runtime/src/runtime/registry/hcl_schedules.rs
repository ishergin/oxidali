use std::collections::HashMap;

use dali2rust_contracts::msg::{
    fixed_text_32, FixedItems, FixedText32, HclAlgorithm, HclLevelMode, HclSchedulePointRow,
    HclScheduleUpsertCommand, HclTargetRow, HclTargetScope, HclTimeRef,
};
use dali2rust_domain::registry::{HclScheduleReadPort, HclScheduleView};
use dali2rust_platform::small_sort::insertion_sort_by;

use crate::runtime::registry::store::{evict_stale, registry_unix_ms, Inner, RegistryStore, Staged, STAGE_MAX_AGE_MS};

pub(crate) const MAX_HCL_SCHEDULES: usize = 8;
pub(crate) const MAX_HCL_TARGETS: usize = 16;
pub(crate) const MAX_HCL_POINTS: usize = 24;

pub(crate) type HclTargetRows = FixedItems<HclTargetRow, MAX_HCL_TARGETS>;
pub(crate) type HclPointRows = FixedItems<HclSchedulePointRow, MAX_HCL_POINTS>;
pub(crate) type HclScheduleMap = HashMap<FixedText32, HclScheduleRecord>;
pub(crate) type HclScheduleStageMap = HashMap<FixedText32, HclScheduleStage>;

pub(crate) const HCL_SCHEDULE_STAGE_MAX_AGE_MS: u64 = STAGE_MAX_AGE_MS;

#[derive(Clone, Debug)]
pub(crate) struct HclScheduleStage {
    pub(crate) record: HclScheduleRecord,
    pub(crate) staged_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct HclScheduleRecord {
    pub enabled: bool,
    pub algorithm: HclAlgorithm,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: HclTargetRows,
    pub points: HclPointRows,
}

impl HclScheduleRecord {
    fn from_header(command: &HclScheduleUpsertCommand) -> Self {
        Self {
            enabled: command.enabled,
            algorithm: command.algorithm,
            active_days_mask: command.active_days_mask,
            latitude_microdeg: command.latitude_microdeg,
            longitude_microdeg: command.longitude_microdeg,
            targets: HclTargetRows::new(),
            points: HclPointRows::new(),
        }
    }

    fn view(&self, schedule_id: &str) -> HclScheduleView {
        HclScheduleView {
            schedule_id: schedule_id.to_string(),
            enabled: self.enabled,
            algorithm: self.algorithm,
            active_days_mask: self.active_days_mask,
            latitude_microdeg: self.latitude_microdeg,
            longitude_microdeg: self.longitude_microdeg,
            targets: self.targets.to_vec(),
            points: self.points.to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HclChunkRejection {
    OutOfOrder,
    TooManyRows,
    ScheduleLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HclChunkOutcome {
    Staged,
    Committed,
}

fn opens_sequence(command: &HclScheduleUpsertCommand) -> bool {
    command.first_target_index == 0 && command.first_point_index == 0
}

fn append_in_order<T: Copy, const N: usize>(
    dst: &mut FixedItems<T, N>,
    first_index: u8,
    rows: &[T],
) -> Result<(), HclChunkRejection> {
    if usize::from(first_index) != dst.len() {
        return Err(HclChunkRejection::OutOfOrder);
    }
    dst.extend_from_slice(rows)
        .map_err(|_| HclChunkRejection::TooManyRows)
}

impl RegistryStore {
    pub(crate) fn apply_hcl_schedule_chunk(
        &self,
        schedule_id: &FixedText32,
        command: &HclScheduleUpsertCommand,
    ) -> Result<HclChunkOutcome, HclChunkRejection> {
        let mut inner = self.write_inner();
        let outcome = stage_chunk(&mut inner, schedule_id, command);
        if outcome.is_err() {
            inner.hcl_schedule_stage.remove(schedule_id);
        }
        drop(inner);
        if outcome == Ok(HclChunkOutcome::Committed) {
            self.dirty.mark_hcl_schedules_dirty();
        }
        outcome
    }

    pub(crate) fn remove_hcl_schedule(&self, schedule_id: &FixedText32) -> bool {
        let mut inner = self.write_inner();
        inner.hcl_schedule_stage.remove(schedule_id);
        let removed = inner.hcl_schedules.remove(schedule_id).is_some();
        if removed {
            inner.hcl_schedules_revision = inner.hcl_schedules_revision.wrapping_add(1);
        }
        drop(inner);
        if removed {
            self.dirty.mark_hcl_schedules_dirty();
        }
        removed
    }

    pub fn hcl_schedules_revision(&self) -> u32 {
        self.read_inner().hcl_schedules_revision
    }

    pub(crate) fn evict_stale_hcl_schedule_stages(&self, max_age_ms: u64) {
        let now = registry_unix_ms();
        evict_stale(&mut self.write_inner().hcl_schedule_stage, now, max_age_ms);
    }
}

fn stage_chunk(
    inner: &mut Inner,
    schedule_id: &FixedText32,
    command: &HclScheduleUpsertCommand,
) -> Result<HclChunkOutcome, HclChunkRejection> {
    if opens_sequence(command) {
        reserve_schedule_slot(inner, schedule_id)?;
        inner.hcl_schedule_stage.insert(
            schedule_id.clone(),
            HclScheduleStage {
                record: HclScheduleRecord::from_header(command),
                staged_at_ms: registry_unix_ms(),
            },
        );
    }
    let staged = &mut inner
        .hcl_schedule_stage
        .get_mut(schedule_id)
        .ok_or(HclChunkRejection::OutOfOrder)?
        .record;
    append_in_order(
        &mut staged.targets,
        command.first_target_index,
        command.targets.as_slice(),
    )?;
    append_in_order(
        &mut staged.points,
        command.first_point_index,
        command.points.as_slice(),
    )?;
    if !command.last_chunk {
        return Ok(HclChunkOutcome::Staged);
    }
    let committed = inner
        .hcl_schedule_stage
        .remove(schedule_id)
        .ok_or(HclChunkRejection::OutOfOrder)?;
    inner
        .hcl_schedules
        .insert(schedule_id.clone(), committed.record);
    inner.hcl_schedules_revision = inner.hcl_schedules_revision.wrapping_add(1);
    Ok(HclChunkOutcome::Committed)
}

fn reserve_schedule_slot(
    inner: &Inner,
    schedule_id: &FixedText32,
) -> Result<(), HclChunkRejection> {
    if inner.hcl_schedules.contains_key(schedule_id) {
        return Ok(());
    }
    let staged_new = inner
        .hcl_schedule_stage
        .keys()
        .filter(|id| *id != schedule_id && !inner.hcl_schedules.contains_key(*id))
        .count();
    if inner.hcl_schedules.len() + staged_new >= MAX_HCL_SCHEDULES {
        return Err(HclChunkRejection::ScheduleLimit);
    }
    Ok(())
}

impl HclScheduleReadPort for RegistryStore {
    fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView> {
        let inner = self.read_inner();
        inner
            .hcl_schedules
            .iter()
            .find(|(id, _)| id.as_str() == schedule_id)
            .map(|(id, record)| record.view(id))
    }

    fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView> {
        let inner = self.read_inner();
        let mut ids: Vec<&FixedText32> = inner.hcl_schedules.keys().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| inner.hcl_schedules.get(id).map(|record| record.view(id)))
            .collect()
    }

    fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool {
        let inner = self.read_inner();
        inner
            .hcl_schedules
            .keys()
            .chain(inner.hcl_schedule_stage.keys())
            .any(|id| id.as_str() == schedule_id)
    }
}

use crate::runtime::registry::persistence_slices::{
    PersistableHclPoint, PersistableHclSchedule, PersistableHclSchedulesSlice, PersistableHclTarget,
};

fn algorithm_code(algorithm: HclAlgorithm) -> u8 {
    match algorithm {
        HclAlgorithm::Stepped => 0,
        HclAlgorithm::Interpolated => 1,
    }
}

fn algorithm_from_code(code: u8) -> Option<HclAlgorithm> {
    match code {
        0 => Some(HclAlgorithm::Stepped),
        1 => Some(HclAlgorithm::Interpolated),
        _ => None,
    }
}

fn scope_code(scope: HclTargetScope) -> u8 {
    match scope {
        HclTargetScope::Group => 0,
        HclTargetScope::Broadcast => 1,
    }
}

fn scope_from_code(code: u8) -> Option<HclTargetScope> {
    match code {
        0 => Some(HclTargetScope::Group),
        1 => Some(HclTargetScope::Broadcast),
        _ => None,
    }
}

fn time_ref_code(time_ref: HclTimeRef) -> u8 {
    match time_ref {
        HclTimeRef::Absolute => 0,
        HclTimeRef::Sunrise => 1,
        HclTimeRef::Sunset => 2,
    }
}

fn time_ref_from_code(code: u8) -> Option<HclTimeRef> {
    match code {
        0 => Some(HclTimeRef::Absolute),
        1 => Some(HclTimeRef::Sunrise),
        2 => Some(HclTimeRef::Sunset),
        _ => None,
    }
}

fn level_mode_code(level_mode: HclLevelMode) -> u8 {
    match level_mode {
        HclLevelMode::Absolute => 0,
        HclLevelMode::LastActive => 1,
        HclLevelMode::None => 2,
    }
}

fn level_mode_from_code(code: u8) -> Option<HclLevelMode> {
    match code {
        0 => Some(HclLevelMode::Absolute),
        1 => Some(HclLevelMode::LastActive),
        2 => Some(HclLevelMode::None),
        _ => None,
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableHclSchedulesSlice {
    let mut schedules: Vec<PersistableHclSchedule> = inner
        .hcl_schedules
        .iter()
        .map(|(id, record)| PersistableHclSchedule {
            schedule_id: id.as_str().to_string(),
            enabled: record.enabled,
            algorithm: algorithm_code(record.algorithm),
            active_days_mask: record.active_days_mask,
            latitude_microdeg: record.latitude_microdeg,
            longitude_microdeg: record.longitude_microdeg,
            targets: record
                .targets
                .iter()
                .map(|row| PersistableHclTarget {
                    adapter_id: row.adapter_id,
                    scope: scope_code(row.scope),
                    group_mask: row.group_mask,
                })
                .collect(),
            points: record
                .points
                .iter()
                .map(|row| PersistableHclPoint {
                    time_ref: time_ref_code(row.time_ref),
                    offset_minutes: row.offset_minutes,
                    level_mode: level_mode_code(row.level_mode),
                    level: row.level,
                    color_temperature_kelvin: row.color_temperature_kelvin,
                })
                .collect(),
        })
        .collect();
    insertion_sort_by(&mut schedules, |a, b| a.schedule_id > b.schedule_id);
    PersistableHclSchedulesSlice { schedules }
}

fn record_from_persistable(stored: &PersistableHclSchedule) -> Option<HclScheduleRecord> {
    let mut targets = HclTargetRows::new();
    for row in &stored.targets {
        let target = HclTargetRow {
            adapter_id: row.adapter_id,
            scope: scope_from_code(row.scope)?,
            group_mask: row.group_mask,
        };
        targets.push(target).ok()?;
    }
    let mut points = HclPointRows::new();
    for row in &stored.points {
        let point = HclSchedulePointRow {
            time_ref: time_ref_from_code(row.time_ref)?,
            offset_minutes: row.offset_minutes,
            level_mode: level_mode_from_code(row.level_mode)?,
            level: row.level,
            color_temperature_kelvin: row.color_temperature_kelvin,
        };
        points.push(point).ok()?;
    }
    Some(HclScheduleRecord {
        enabled: stored.enabled,
        algorithm: algorithm_from_code(stored.algorithm)?,
        active_days_mask: stored.active_days_mask,
        latitude_microdeg: stored.latitude_microdeg,
        longitude_microdeg: stored.longitude_microdeg,
        targets,
        points,
    })
}

pub(crate) fn hydrate_hcl_schedules_inner(inner: &mut Inner, slice: &PersistableHclSchedulesSlice) {
    inner.hcl_schedules.clear();
    for stored in slice.schedules.iter().take(MAX_HCL_SCHEDULES) {
        let Some(record) = record_from_persistable(stored) else {
            log::warn!(
                "persistence: hcl schedule {:?} undecodable, skipped",
                stored.schedule_id
            );
            continue;
        };
        inner
            .hcl_schedules
            .insert(fixed_text_32(&stored.schedule_id), record);
    }
    inner.hcl_schedules_revision = inner.hcl_schedules_revision.wrapping_add(1);
}

impl Staged for HclScheduleStage {
    fn staged_at_ms(&self) -> u64 {
        self.staged_at_ms
    }
}
