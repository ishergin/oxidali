use dali2rust_contracts::msg::{
    fixed_text_32, FixedText32, HclAlgorithm, HclLevelMode, HclSchedulePointRow, HclTargetRow,
    HclTargetScope, HclTimeRef,
};

use crate::http::hcl_state::{
    deg_to_microdeg, hcl_algorithm_from_str, hcl_level_mode_from_str, hcl_target_scope_from_str,
    hcl_time_ref_from_str, weekday_bit, HclLocationDto, HclSchedulePointDto, HclScheduleDto,
    HclTargetDto, DALI_GROUP_COUNT,
};
use crate::http::handlers::common::json_err;
use crate::http::types::HttpResponse;

const MAX_TARGETS: usize = 16;
const MAX_POINTS: usize = 24;
const MAX_SCHEDULE_ID_LEN: usize = 32;
const MAX_LEVEL: u8 = 254;
const MIN_KELVIN: u16 = 1_000;
const MAX_KELVIN: u16 = 20_000;
const MAX_ABSOLUTE_OFFSET: i32 = 1_439;
const MAX_ASTRONOMICAL_OFFSET: i32 = 720;
const MAX_LATITUDE_DEG: f64 = 90.0;
const MAX_LONGITUDE_DEG: f64 = 180.0;
const REFUSED_SCOPES: [&str; 2] = ["virtual_lamp", "scene"];

pub(crate) struct ValidatedSchedule {
    pub schedule_id: FixedText32,
    pub enabled: bool,
    pub algorithm: HclAlgorithm,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: Vec<HclTargetRow>,
    pub points: Vec<HclSchedulePointRow>,
}

fn invalid_value() -> HttpResponse {
    json_err(422, "invalid_value")
}

fn invalid_enum() -> HttpResponse {
    json_err(422, "invalid_enum")
}

pub(crate) fn validate_schedule(dto: &HclScheduleDto) -> Result<ValidatedSchedule, HttpResponse> {
    let schedule_id = validate_schedule_id(&dto.schedule_id)?;
    let algorithm = hcl_algorithm_from_str(&dto.algorithm).ok_or_else(invalid_enum)?;
    let active_days_mask = validate_active_days(&dto.active_days)?;
    let (latitude_microdeg, longitude_microdeg) = validate_location(dto.location.as_ref())?;
    Ok(ValidatedSchedule {
        schedule_id,
        enabled: dto.enabled,
        algorithm,
        active_days_mask,
        latitude_microdeg,
        longitude_microdeg,
        targets: validate_targets(&dto.targets)?,
        points: validate_points(&dto.points, dto.location.is_some())?,
    })
}

pub(crate) fn validate_schedule_id(raw: &str) -> Result<FixedText32, HttpResponse> {
    let valid = !raw.is_empty()
        && raw.len() <= MAX_SCHEDULE_ID_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !valid {
        return Err(json_err(422, "invalid_resource_id"));
    }
    Ok(fixed_text_32(raw))
}

fn validate_active_days(days: &[String]) -> Result<u8, HttpResponse> {
    if days.is_empty() {
        return Err(invalid_value());
    }
    let mut mask = 0u8;
    for day in days {
        let bit = weekday_bit(day).ok_or_else(invalid_enum)?;
        let flag = 1u8 << bit;
        if mask & flag != 0 {
            return Err(invalid_value());
        }
        mask |= flag;
    }
    Ok(mask)
}

fn validate_location(
    location: Option<&HclLocationDto>,
) -> Result<(Option<i32>, Option<i32>), HttpResponse> {
    let Some(location) = location else {
        return Ok((None, None));
    };
    let in_range = location.latitude_deg.abs() <= MAX_LATITUDE_DEG
        && location.longitude_deg.abs() <= MAX_LONGITUDE_DEG;
    if !in_range || !location.latitude_deg.is_finite() || !location.longitude_deg.is_finite() {
        return Err(invalid_value());
    }
    Ok((
        Some(deg_to_microdeg(location.latitude_deg)),
        Some(deg_to_microdeg(location.longitude_deg)),
    ))
}

fn validate_targets(targets: &[HclTargetDto]) -> Result<Vec<HclTargetRow>, HttpResponse> {
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(invalid_value());
    }
    targets.iter().map(validate_target).collect()
}

fn validate_target(target: &HclTargetDto) -> Result<HclTargetRow, HttpResponse> {
    if REFUSED_SCOPES.contains(&target.scope.as_str()) {
        return Err(json_err(422, "unsupported_field"));
    }
    let scope = hcl_target_scope_from_str(&target.scope).ok_or_else(invalid_enum)?;
    let group_mask = match scope {
        HclTargetScope::Group => group_mask_from_ids(target.group_ids.as_deref())?,
        HclTargetScope::Broadcast if target.group_ids.is_some() => return Err(invalid_value()),
        HclTargetScope::Broadcast => 0,
    };
    Ok(HclTargetRow {
        adapter_id: target.adapter_id,
        scope,
        group_mask,
    })
}

fn group_mask_from_ids(group_ids: Option<&[u8]>) -> Result<u16, HttpResponse> {
    let ids = group_ids.filter(|ids| !ids.is_empty()).ok_or_else(invalid_value)?;
    let mut mask = 0u16;
    for id in ids {
        if *id >= DALI_GROUP_COUNT {
            return Err(invalid_value());
        }
        let flag = 1u16 << id;
        if mask & flag != 0 {
            return Err(invalid_value());
        }
        mask |= flag;
    }
    Ok(mask)
}

fn validate_points(
    points: &[HclSchedulePointDto],
    has_location: bool,
) -> Result<Vec<HclSchedulePointRow>, HttpResponse> {
    if points.is_empty() || points.len() > MAX_POINTS {
        return Err(invalid_value());
    }
    let rows: Vec<HclSchedulePointRow> = points
        .iter()
        .map(|point| validate_point(point, has_location))
        .collect::<Result<_, _>>()?;
    reject_duplicate_times(&rows)?;
    Ok(rows)
}

fn validate_point(
    point: &HclSchedulePointDto,
    has_location: bool,
) -> Result<HclSchedulePointRow, HttpResponse> {
    let time_ref = hcl_time_ref_from_str(&point.time_ref).ok_or_else(invalid_enum)?;
    let level_mode = hcl_level_mode_from_str(&point.level_mode).ok_or_else(invalid_enum)?;
    if time_ref != HclTimeRef::Absolute && !has_location {
        return Err(invalid_value());
    }
    validate_offset(time_ref, point.offset_minutes)?;
    validate_level(level_mode, point.level)?;
    validate_kelvin(point.color_temperature_kelvin)?;
    Ok(HclSchedulePointRow {
        time_ref,
        offset_minutes: point.offset_minutes as i16,
        level_mode,
        level: point.level,
        color_temperature_kelvin: point.color_temperature_kelvin,
    })
}

fn validate_offset(time_ref: HclTimeRef, offset_minutes: i32) -> Result<(), HttpResponse> {
    let in_range = match time_ref {
        HclTimeRef::Absolute => (0..=MAX_ABSOLUTE_OFFSET).contains(&offset_minutes),
        HclTimeRef::Sunrise | HclTimeRef::Sunset => {
            (-MAX_ASTRONOMICAL_OFFSET..=MAX_ASTRONOMICAL_OFFSET).contains(&offset_minutes)
        }
    };
    if in_range {
        Ok(())
    } else {
        Err(invalid_value())
    }
}

fn validate_level(level_mode: HclLevelMode, level: Option<u8>) -> Result<(), HttpResponse> {
    let valid = match level_mode {
        HclLevelMode::Absolute => level.is_some_and(|level| level <= MAX_LEVEL),
        HclLevelMode::LastActive | HclLevelMode::None => level.is_none(),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_value())
    }
}

fn validate_kelvin(kelvin: Option<u16>) -> Result<(), HttpResponse> {
    match kelvin {
        Some(k) if !(MIN_KELVIN..=MAX_KELVIN).contains(&k) => Err(invalid_value()),
        _ => Ok(()),
    }
}

fn reject_duplicate_times(rows: &[HclSchedulePointRow]) -> Result<(), HttpResponse> {
    for (index, row) in rows.iter().enumerate() {
        let duplicate = rows[..index]
            .iter()
            .any(|seen| seen.time_ref == row.time_ref && seen.offset_minutes == row.offset_minutes);
        if duplicate {
            return Err(invalid_value());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(time_ref: &str, offset: i32, mode: &str, level: Option<u8>) -> HclSchedulePointDto {
        HclSchedulePointDto {
            time_ref: time_ref.to_string(),
            offset_minutes: offset,
            level_mode: mode.to_string(),
            level,
            color_temperature_kelvin: Some(2700),
        }
    }

    fn group_target(group_ids: Option<Vec<u8>>) -> HclTargetDto {
        HclTargetDto {
            adapter_id: 0,
            scope: "group".to_string(),
            group_ids,
        }
    }

    fn dto(targets: Vec<HclTargetDto>, points: Vec<HclSchedulePointDto>) -> HclScheduleDto {
        HclScheduleDto {
            schedule_id: "morning".to_string(),
            enabled: true,
            algorithm: "stepped".to_string(),
            active_days: vec!["mon".to_string(), "tue".to_string()],
            location: None,
            targets,
            points,
        }
    }

    fn error_code(response: HttpResponse) -> String {
        String::from_utf8_lossy(&response.body.into_bytes()).into_owned()
    }

    fn status_of(dto: &HclScheduleDto) -> u16 {
        match validate_schedule(dto) {
            Ok(_) => 200,
            Err(response) => response.status,
        }
    }

    #[test]
    fn a_valid_schedule_converts_to_bus_rows() {
        let valid = dto(
            vec![group_target(Some(vec![1, 5]))],
            vec![point("absolute", 360, "absolute", Some(80))],
        );
        let Ok(out) = validate_schedule(&valid) else {
            panic!("valid schedule must pass validation");
        };
        assert_eq!(out.active_days_mask, 0b0000_0011);
        assert_eq!(out.targets[0].group_mask, 0b0010_0010);
        assert_eq!(out.points[0].offset_minutes, 360);
        assert!(out.latitude_microdeg.is_none());
    }

    #[test]
    fn refused_scopes_answer_unsupported_field_not_invalid_enum() {
        for scope in ["virtual_lamp", "scene"] {
            let mut bad = dto(
                vec![group_target(Some(vec![1]))],
                vec![point("absolute", 0, "absolute", Some(10))],
            );
            bad.targets[0].scope = scope.to_string();
            let Err(error) = validate_schedule(&bad) else {
                panic!("scope {scope} must be refused");
            };
            assert_eq!(error.status, 422);
            assert!(
                error_code(error).contains("unsupported_field"),
                "scope {scope} must answer unsupported_field"
            );
        }
    }

    #[test]
    fn structural_negatives_are_refused() {
        let base_point = point("absolute", 0, "absolute", Some(10));
        let cases: Vec<HclScheduleDto> = vec![
            dto(vec![group_target(None)], vec![base_point.clone()]),
            dto(vec![group_target(Some(vec![]))], vec![base_point.clone()]),
            dto(vec![group_target(Some(vec![16]))], vec![base_point.clone()]),
            dto(vec![group_target(Some(vec![1, 1]))], vec![base_point.clone()]),
            dto(
                vec![HclTargetDto {
                    adapter_id: 0,
                    scope: "broadcast".to_string(),
                    group_ids: Some(vec![1]),
                }],
                vec![base_point.clone()],
            ),
            dto(vec![], vec![base_point.clone()]),
            dto(vec![group_target(Some(vec![1]))], vec![]),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![point("absolute", 0, "absolute", None)],
            ),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![point("absolute", 0, "last_active", Some(10))],
            ),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![point("absolute", 0, "absolute", Some(255))],
            ),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![point("absolute", 1440, "absolute", Some(10))],
            ),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![point("sunset", -30, "none", None)],
            ),
            dto(
                vec![group_target(Some(vec![1]))],
                vec![base_point.clone(), base_point.clone()],
            ),
        ];
        for case in &cases {
            assert_eq!(status_of(case), 422, "expected 422 for {case:?}");
        }
    }

    #[test]
    fn unknown_enum_values_are_distinguished_from_bad_values() {
        let mut bad = dto(
            vec![group_target(Some(vec![1]))],
            vec![point("absolute", 0, "absolute", Some(10))],
        );
        bad.algorithm = "linear".to_string();
        let Err(error) = validate_schedule(&bad) else {
            panic!("unknown algorithm must be refused");
        };
        assert!(error_code(error).contains("invalid_enum"));
    }

    #[test]
    fn schedule_id_must_be_a_slug() {
        for bad in ["", "Morning", "with space", "слово", &"x".repeat(33)] {
            assert!(validate_schedule_id(bad).is_err(), "{bad:?} must be refused");
        }
        for good in ["morning", "a", "wake-up_2"] {
            assert!(validate_schedule_id(good).is_ok(), "{good:?} must be accepted");
        }
    }
}
