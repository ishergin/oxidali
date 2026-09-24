use dali2rust_contracts::msg::{HclAlgorithm, HclLevelMode, HclSchedulePointRow, HclTimeRef};
use dali2rust_domain::registry::kelvin_to_mirek;
use dali2rust_platform::small_sort::insertion_sort_by;

use super::astronomy::{solar_events, Location, SolarEvents};

const MINUTES_PER_DAY: i32 = 1440;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectivePoint {
    pub effective_minutes: i32,
    pub level_mode: HclLevelMode,
    pub level: Option<u8>,
    pub color_temperature_kelvin: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesiredState {
    pub level_mode: HclLevelMode,
    pub level: Option<u8>,
    pub color_temperature_kelvin: Option<u16>,
}

impl DesiredState {
    pub fn is_noop(&self) -> bool {
        self.color_temperature_kelvin.is_none() && self.level_mode == HclLevelMode::None
    }

    pub fn wire_equal(&self, other: &DesiredState) -> bool {
        self.level_mode == other.level_mode
            && self.level == other.level
            && self.color_temperature_kelvin.map(kelvin_to_mirek)
                == other.color_temperature_kelvin.map(kelvin_to_mirek)
    }
}

pub fn effective_points(
    points: &[HclSchedulePointRow],
    location: Option<Location>,
    year_day: u16,
    utc_offset_minutes: i16,
) -> Vec<EffectivePoint> {
    let events = location
        .map(|loc| solar_events(loc, year_day, utc_offset_minutes))
        .unwrap_or(SolarEvents {
            sunrise_minutes: None,
            sunset_minutes: None,
        });
    let mut resolved: Vec<EffectivePoint> = points
        .iter()
        .filter_map(|point| {
            Some(EffectivePoint {
                effective_minutes: effective_minutes(point, events)?,
                level_mode: point.level_mode,
                level: point.level,
                color_temperature_kelvin: point.color_temperature_kelvin,
            })
        })
        .collect();
    insertion_sort_by(&mut resolved, |a, b| a.effective_minutes > b.effective_minutes);
    resolved
}

fn effective_minutes(point: &HclSchedulePointRow, events: SolarEvents) -> Option<i32> {
    let anchor = match point.time_ref {
        HclTimeRef::Absolute => 0,
        HclTimeRef::Sunrise => events.sunrise_minutes?,
        HclTimeRef::Sunset => events.sunset_minutes?,
    };
    Some((anchor + i32::from(point.offset_minutes)).rem_euclid(MINUTES_PER_DAY))
}

pub fn evaluate(
    points: &[EffectivePoint],
    now_minutes: i32,
    algorithm: HclAlgorithm,
) -> Option<DesiredState> {
    let previous = points
        .iter()
        .rev()
        .find(|point| point.effective_minutes <= now_minutes)?;
    let next = points
        .iter()
        .find(|point| point.effective_minutes > now_minutes);
    match (algorithm, next) {
        (HclAlgorithm::Stepped, _) | (HclAlgorithm::Interpolated, None) => Some(DesiredState {
            level_mode: previous.level_mode,
            level: previous.level,
            color_temperature_kelvin: previous.color_temperature_kelvin,
        }),
        (HclAlgorithm::Interpolated, Some(next)) => {
            Some(interpolate(previous, next, now_minutes))
        }
    }
}

fn interpolate(previous: &EffectivePoint, next: &EffectivePoint, now_minutes: i32) -> DesiredState {
    let span = next.effective_minutes - previous.effective_minutes;
    let elapsed = now_minutes - previous.effective_minutes;
    let fraction = if span > 0 {
        f64::from(elapsed) / f64::from(span)
    } else {
        0.0
    };
    DesiredState {
        level_mode: previous.level_mode,
        level: interpolated_level(previous, next, fraction),
        color_temperature_kelvin: ramp_u16(
            previous.color_temperature_kelvin,
            next.color_temperature_kelvin,
            fraction,
        ),
    }
}

fn interpolated_level(
    previous: &EffectivePoint,
    next: &EffectivePoint,
    fraction: f64,
) -> Option<u8> {
    if previous.level_mode != HclLevelMode::Absolute || next.level_mode != HclLevelMode::Absolute {
        return previous.level;
    }
    ramp_u8(previous.level, next.level, fraction)
}

fn ramp_u8(from: Option<u8>, to: Option<u8>, fraction: f64) -> Option<u8> {
    let (from, to) = (from?, to?);
    Some(ramp(f64::from(from), f64::from(to), fraction) as u8)
}

fn ramp_u16(from: Option<u16>, to: Option<u16>, fraction: f64) -> Option<u16> {
    let from_value = from?;
    let Some(to_value) = to else {
        return from;
    };
    Some(ramp(f64::from(from_value), f64::from(to_value), fraction) as u16)
}

fn ramp(from: f64, to: f64, fraction: f64) -> f64 {
    (from + (to - from) * fraction.clamp(0.0, 1.0)).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(
        time_ref: HclTimeRef,
        offset: i16,
        level_mode: HclLevelMode,
        level: Option<u8>,
        kelvin: Option<u16>,
    ) -> HclSchedulePointRow {
        HclSchedulePointRow {
            time_ref,
            offset_minutes: offset,
            level_mode,
            level,
            color_temperature_kelvin: kelvin,
        }
    }

    fn absolute(offset: i16, level: u8, kelvin: u16) -> HclSchedulePointRow {
        point(
            HclTimeRef::Absolute,
            offset,
            HclLevelMode::Absolute,
            Some(level),
            Some(kelvin),
        )
    }

    fn resolved(points: &[HclSchedulePointRow]) -> Vec<EffectivePoint> {
        effective_points(points, None, 172, 180)
    }

    #[test]
    fn before_the_first_point_the_curve_says_nothing() {
        let points = resolved(&[absolute(360, 80, 2700)]);
        assert_eq!(evaluate(&points, 300, HclAlgorithm::Stepped), None);
        assert_eq!(evaluate(&points, 300, HclAlgorithm::Interpolated), None);
    }

    #[test]
    fn stepped_holds_a_point_until_the_next_is_due() {
        let points = resolved(&[absolute(360, 80, 2700), absolute(480, 200, 4000)]);
        let held = evaluate(&points, 400, HclAlgorithm::Stepped).expect("held point");
        assert_eq!(held.level, Some(80));
        assert_eq!(held.color_temperature_kelvin, Some(2700));

        let next = evaluate(&points, 480, HclAlgorithm::Stepped).expect("second point");
        assert_eq!(next.level, Some(200), "a point applies at its own minute");
    }

    #[test]
    fn interpolated_ramps_between_the_neighbours() {
        let points = resolved(&[absolute(360, 80, 2700), absolute(480, 200, 4000)]);
        let halfway = evaluate(&points, 420, HclAlgorithm::Interpolated).expect("midpoint");
        assert_eq!(halfway.level, Some(140), "80 + (200-80)/2");
        assert_eq!(halfway.color_temperature_kelvin, Some(3350));

        let quarter = evaluate(&points, 390, HclAlgorithm::Interpolated).expect("quarter");
        assert_eq!(quarter.level, Some(110));
    }

    #[test]
    fn interpolated_past_the_last_point_holds_it() {
        let points = resolved(&[absolute(360, 80, 2700), absolute(480, 200, 4000)]);
        let after = evaluate(&points, 1200, HclAlgorithm::Interpolated).expect("last point");
        assert_eq!(after.level, Some(200));
        assert_eq!(after.color_temperature_kelvin, Some(4000));
    }

    #[test]
    fn a_last_active_side_stops_the_level_from_being_ramped() {
        let points = resolved(&[
            absolute(360, 80, 2700),
            point(
                HclTimeRef::Absolute,
                480,
                HclLevelMode::LastActive,
                None,
                Some(4000),
            ),
        ]);
        let halfway = evaluate(&points, 420, HclAlgorithm::Interpolated).expect("midpoint");
        assert_eq!(
            halfway.level,
            Some(80),
            "the level is held, not ramped toward an unknown end"
        );
        assert_eq!(
            halfway.color_temperature_kelvin,
            Some(3350),
            "colour still ramps — only the level has an unknown side"
        );
    }

    #[test]
    fn a_colour_only_point_publishes_no_level() {
        let points = resolved(&[point(
            HclTimeRef::Absolute,
            360,
            HclLevelMode::None,
            None,
            Some(2700),
        )]);
        let state = evaluate(&points, 400, HclAlgorithm::Stepped).expect("colour point");
        assert_eq!(state.level_mode, HclLevelMode::None);
        assert_eq!(state.level, None);
        assert!(!state.is_noop());
    }

    #[test]
    fn a_point_with_neither_level_nor_colour_is_a_noop() {
        let points = resolved(&[point(
            HclTimeRef::Absolute,
            360,
            HclLevelMode::None,
            None,
            None,
        )]);
        let state = evaluate(&points, 400, HclAlgorithm::Stepped).expect("point");
        assert!(state.is_noop(), "nothing to publish");
    }

    #[test]
    fn astronomical_points_resolve_against_the_days_sunset() {
        let moscow = Location {
            latitude_deg: 55.7558,
            longitude_deg: 37.6173,
        };
        let points = effective_points(
            &[point(
                HclTimeRef::Sunset,
                -30,
                HclLevelMode::Absolute,
                Some(120),
                Some(2200),
            )],
            Some(moscow),
            172,
            180,
        );
        assert_eq!(points.len(), 1);
        assert!(
            (points[0].effective_minutes - (20 * 60 + 48)).abs() <= 2,
            "resolved to {}",
            points[0].effective_minutes
        );
    }

    #[test]
    fn a_solar_point_past_midnight_fires_at_its_wrapped_minute() {
        let moscow = Location {
            latitude_deg: 55.7558,
            longitude_deg: 37.6173,
        };
        let points = effective_points(
            &[point(
                HclTimeRef::Sunset,
                180,
                HclLevelMode::Absolute,
                Some(30),
                Some(2200),
            )],
            Some(moscow),
            172,
            180,
        );
        assert_eq!(points.len(), 1);
        assert!(
            (points[0].effective_minutes - 18).abs() <= 2,
            "resolved to {}",
            points[0].effective_minutes
        );
        let fired =
            evaluate(&points, 25, HclAlgorithm::Stepped).expect("fires at its wrapped minute");
        assert_eq!(fired.level, Some(30));
        assert_eq!(
            evaluate(&points, 10, HclAlgorithm::Stepped),
            None,
            "before the wrapped minute the day has not reached the curve"
        );
    }

    #[test]
    fn a_point_anchored_to_an_event_that_never_happens_is_dropped() {
        let longyearbyen = Location {
            latitude_deg: 78.22,
            longitude_deg: 15.65,
        };
        let points = effective_points(
            &[
                point(HclTimeRef::Sunset, 0, HclLevelMode::None, None, Some(2200)),
                absolute(600, 100, 4000),
            ],
            Some(longyearbyen),
            172,
            60,
        );
        assert_eq!(points.len(), 1, "only the absolute point survives");
        assert_eq!(points[0].effective_minutes, 600);
    }

    #[test]
    fn wire_equality_runs_through_the_mirek_encoder() {
        let a = DesiredState {
            level_mode: HclLevelMode::None,
            level: None,
            color_temperature_kelvin: Some(6000),
        };
        let b = DesiredState {
            color_temperature_kelvin: Some(6005),
            ..a
        };
        let c = DesiredState {
            color_temperature_kelvin: Some(6250),
            ..a
        };
        assert_ne!(a, b, "the raw kelvins differ");
        assert!(a.wire_equal(&b), "both encode to the same mirek");
        assert!(!a.wire_equal(&c));
        assert!(
            !a.wire_equal(&DesiredState {
                color_temperature_kelvin: None,
                ..a
            }),
            "a colour and no colour are different commands"
        );
    }

    #[test]
    fn points_are_evaluated_in_time_order_not_list_order() {
        let points = resolved(&[absolute(480, 200, 4000), absolute(360, 80, 2700)]);
        let early = evaluate(&points, 400, HclAlgorithm::Stepped).expect("early point");
        assert_eq!(early.level, Some(80), "the 06:00 point wins at 06:40");
    }
}
