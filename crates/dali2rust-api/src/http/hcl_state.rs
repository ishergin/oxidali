use std::sync::Arc;

use dali2rust_contracts::msg::{
    HclAlgorithm, HclLevelMode, HclSchedulePointRow, HclTargetRow, HclTargetScope, HclTimeRef,
};
use dali2rust_domain::registry::{HclOverrideView, HclScheduleReadPort, HclScheduleView};
use serde::{Deserialize, Serialize};

pub const WEEKDAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
pub use dali2rust_domain::registry::GROUP_COUNT as DALI_GROUP_COUNT;
pub const MICRODEGREES_PER_DEGREE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HclLocationDto {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HclTargetDto {
    pub adapter_id: u8,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_ids: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HclSchedulePointDto {
    pub time_ref: String,
    pub offset_minutes: i32,
    pub level_mode: String,
    #[serde(default)]
    pub level: Option<u8>,
    #[serde(default)]
    pub color_temperature_kelvin: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HclScheduleDto {
    #[serde(default)]
    pub schedule_id: String,
    pub enabled: bool,
    pub algorithm: String,
    pub active_days: Vec<String>,
    #[serde(default)]
    pub location: Option<HclLocationDto>,
    pub targets: Vec<HclTargetDto>,
    pub points: Vec<HclSchedulePointDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HclSchedulesListBody {
    pub schedules: Vec<HclScheduleDto>,
}

pub fn hcl_algorithm_str(algorithm: HclAlgorithm) -> &'static str {
    match algorithm {
        HclAlgorithm::Stepped => "stepped",
        HclAlgorithm::Interpolated => "interpolated",
    }
}

pub fn hcl_algorithm_from_str(raw: &str) -> Option<HclAlgorithm> {
    match raw {
        "stepped" => Some(HclAlgorithm::Stepped),
        "interpolated" => Some(HclAlgorithm::Interpolated),
        _ => None,
    }
}

pub fn hcl_time_ref_str(time_ref: HclTimeRef) -> &'static str {
    match time_ref {
        HclTimeRef::Absolute => "absolute",
        HclTimeRef::Sunrise => "sunrise",
        HclTimeRef::Sunset => "sunset",
    }
}

pub fn hcl_time_ref_from_str(raw: &str) -> Option<HclTimeRef> {
    match raw {
        "absolute" => Some(HclTimeRef::Absolute),
        "sunrise" => Some(HclTimeRef::Sunrise),
        "sunset" => Some(HclTimeRef::Sunset),
        _ => None,
    }
}

pub fn hcl_level_mode_str(level_mode: HclLevelMode) -> &'static str {
    match level_mode {
        HclLevelMode::Absolute => "absolute",
        HclLevelMode::LastActive => "last_active",
        HclLevelMode::None => "none",
    }
}

pub fn hcl_level_mode_from_str(raw: &str) -> Option<HclLevelMode> {
    match raw {
        "absolute" => Some(HclLevelMode::Absolute),
        "last_active" => Some(HclLevelMode::LastActive),
        "none" => Some(HclLevelMode::None),
        _ => None,
    }
}

pub fn hcl_target_scope_str(scope: HclTargetScope) -> &'static str {
    match scope {
        HclTargetScope::Group => "group",
        HclTargetScope::Broadcast => "broadcast",
    }
}

pub fn hcl_target_scope_from_str(raw: &str) -> Option<HclTargetScope> {
    match raw {
        "group" => Some(HclTargetScope::Group),
        "broadcast" => Some(HclTargetScope::Broadcast),
        _ => None,
    }
}

pub fn weekday_bit(name: &str) -> Option<u8> {
    WEEKDAY_NAMES
        .iter()
        .position(|day| *day == name)
        .map(|index| index as u8)
    }

pub fn active_days_from_mask(mask: u8) -> Vec<String> {
    WEEKDAY_NAMES
        .iter()
        .enumerate()
        .filter(|(index, _)| mask & (1 << index) != 0)
        .map(|(_, name)| (*name).to_string())
        .collect()
}

pub fn group_ids_from_mask(mask: u16) -> Vec<u8> {
    (0..DALI_GROUP_COUNT)
        .filter(|group| mask & (1u16 << group) != 0)
        .collect()
}

pub fn microdeg_to_deg(microdeg: i32) -> f64 {
    f64::from(microdeg) / MICRODEGREES_PER_DEGREE
}

pub fn deg_to_microdeg(deg: f64) -> i32 {
    (deg * MICRODEGREES_PER_DEGREE).round() as i32
}

fn target_row_to_dto(row: &HclTargetRow) -> HclTargetDto {
    HclTargetDto {
        adapter_id: row.adapter_id,
        scope: hcl_target_scope_str(row.scope).to_string(),
        group_ids: match row.scope {
            HclTargetScope::Group => Some(group_ids_from_mask(row.group_mask)),
            HclTargetScope::Broadcast => None,
        },
    }
}

fn point_row_to_dto(row: &HclSchedulePointRow) -> HclSchedulePointDto {
    HclSchedulePointDto {
        time_ref: hcl_time_ref_str(row.time_ref).to_string(),
        offset_minutes: i32::from(row.offset_minutes),
        level_mode: hcl_level_mode_str(row.level_mode).to_string(),
        level: row.level,
        color_temperature_kelvin: row.color_temperature_kelvin,
    }
}

pub fn hcl_schedule_view_to_dto(view: HclScheduleView) -> HclScheduleDto {
    HclScheduleDto {
        schedule_id: view.schedule_id,
        enabled: view.enabled,
        algorithm: hcl_algorithm_str(view.algorithm).to_string(),
        active_days: active_days_from_mask(view.active_days_mask),
        location: match (view.latitude_microdeg, view.longitude_microdeg) {
            (Some(lat), Some(lon)) => Some(HclLocationDto {
                latitude_deg: microdeg_to_deg(lat),
                longitude_deg: microdeg_to_deg(lon),
            }),
            _ => None,
        },
        targets: view.targets.iter().map(target_row_to_dto).collect(),
        points: view.points.iter().map(point_row_to_dto).collect(),
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct HclOverrideTargetDto {
    pub adapter_id: u8,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u8>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct HclOverrideDto {
    pub suspended: bool,
    pub targets: Vec<HclOverrideTargetDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_local_minutes: Option<u16>,
}

pub fn hcl_override_view_to_dto(view: HclOverrideView) -> HclOverrideDto {
    HclOverrideDto {
        suspended: view.suspended,
        targets: view
            .targets
            .iter()
            .map(|target| HclOverrideTargetDto {
                adapter_id: target.adapter_id,
                scope: hcl_target_scope_str(target.scope).to_string(),
                group_id: target.group_id,
            })
            .collect(),
        since_local_minutes: view.since_local_minutes,
    }
}

pub trait HclScheduleHttpState: Send + Sync {
    fn hcl_schedule_dto(&self, schedule_id: &str) -> Option<HclScheduleDto>;
    fn list_hcl_schedule_dtos(&self) -> Vec<HclScheduleDto>;
    fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool;
}

pub struct HclScheduleHttpStateBridge {
    port: Arc<dyn HclScheduleReadPort>,
}

impl HclScheduleHttpStateBridge {
    pub fn new(port: Arc<dyn HclScheduleReadPort>) -> Self {
        Self { port }
    }
}

impl HclScheduleHttpState for HclScheduleHttpStateBridge {
    fn hcl_schedule_dto(&self, schedule_id: &str) -> Option<HclScheduleDto> {
        self.port
            .hcl_schedule_view(schedule_id)
            .map(hcl_schedule_view_to_dto)
    }

    fn list_hcl_schedule_dtos(&self) -> Vec<HclScheduleDto> {
        self.port
            .list_hcl_schedule_views()
            .into_iter()
            .map(hcl_schedule_view_to_dto)
            .collect()
    }

    fn hcl_schedule_id_taken(&self, schedule_id: &str) -> bool {
        self.port.hcl_schedule_id_taken(schedule_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_names_round_trip_through_the_mask() {
        for (index, name) in WEEKDAY_NAMES.iter().enumerate() {
            let mask = 1u8 << index;
            assert_eq!(weekday_bit(name), Some(index as u8));
            assert_eq!(active_days_from_mask(mask), vec![(*name).to_string()]);
        }
        assert_eq!(active_days_from_mask(0b0001_1111).len(), 5, "mon..fri");
    }

    #[test]
    fn group_mask_expands_in_ascending_order() {
        assert_eq!(group_ids_from_mask(0b0000_0000_0010_0010), vec![1, 5]);
        assert_eq!(group_ids_from_mask(0), Vec::<u8>::new());
        assert_eq!(group_ids_from_mask(u16::MAX).len(), 16);
    }

    #[test]
    fn coordinates_survive_the_degree_microdegree_round_trip() {
        for deg in [55.7558_f64, -33.8688, 0.0, 90.0, -180.0] {
            let round_tripped = microdeg_to_deg(deg_to_microdeg(deg));
            assert!(
                (round_tripped - deg).abs() < 1e-6,
                "{deg} became {round_tripped}"
            );
        }
    }
}
