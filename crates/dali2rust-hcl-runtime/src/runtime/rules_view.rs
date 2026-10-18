use std::sync::Arc;

use dali2rust_contracts::msg::HclTargetScope;
use dali2rust_domain::registry::{HclScheduleReadPort, HclScheduleView};
use dali2rust_platform::wall_clock::LocalCivilTime;

use super::astronomy::{solar_events, Location};
use super::curve::MINUTES_PER_DAY;
use super::overrides::SuspendedTarget;
use super::plan::{expand_target, TargetKey};
use super::scheduler_worker::{lock_ledger, SharedOverrideLedger};

const LAST_MINUTE_OF_DAY: i32 = MINUTES_PER_DAY - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunMinutes {
    pub sunrise: u16,
    pub sunset: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HclTargetStatus {
    pub target: TargetKey,
    pub enabled: bool,
    pub overridden: bool,
}

pub struct HclRulesView {
    schedules: Arc<dyn HclScheduleReadPort>,
    overrides: SharedOverrideLedger,
}

impl HclRulesView {
    pub fn new(schedules: Arc<dyn HclScheduleReadPort>, overrides: SharedOverrideLedger) -> Self {
        Self { schedules, overrides }
    }

    pub fn sun(&self, local: LocalCivilTime) -> Option<SunMinutes> {
        let location = first_location(&self.schedules.list_hcl_schedule_views())?;
        sun_minutes(location, local.year_day, local.utc_offset_minutes)
    }

    pub fn target_statuses(&self) -> Vec<HclTargetStatus> {
        let mut rows = Vec::new();
        for schedule in self.schedules.list_hcl_schedule_views() {
            let suspended = lock_ledger(&self.overrides).suspended_targets(&schedule.schedule_id);
            rows.extend(schedule_target_statuses(&schedule, &suspended));
        }
        rows
    }
}

fn first_location(schedules: &[HclScheduleView]) -> Option<Location> {
    schedules
        .iter()
        .find_map(|s| Location::from_microdeg(s.latitude_microdeg, s.longitude_microdeg))
}

fn sun_minutes(location: Location, year_day: u16, utc_offset_minutes: i16) -> Option<SunMinutes> {
    let events = solar_events(location, year_day, utc_offset_minutes);
    Some(SunMinutes {
        sunrise: minute_of_day(events.sunrise_minutes?),
        sunset: minute_of_day(events.sunset_minutes?),
    })
}

fn minute_of_day(minutes: i32) -> u16 {
    u16::try_from(minutes.clamp(0, LAST_MINUTE_OF_DAY)).unwrap_or(0)
}

fn schedule_target_statuses(
    schedule: &HclScheduleView,
    suspended: &[SuspendedTarget],
) -> Vec<HclTargetStatus> {
    schedule
        .targets
        .iter()
        .flat_map(expand_target)
        .map(|target| HclTargetStatus {
            target,
            enabled: schedule.enabled,
            overridden: suspended.iter().any(|s| suspended_matches(&s.target, &target)),
        })
        .collect()
}

fn suspended_matches(suspended: &TargetKey, target: &TargetKey) -> bool {
    suspended.scope == target.scope
        && suspended.adapter_id == target.adapter_id
        && (target.scope == HclTargetScope::Broadcast || suspended.group_id == target.group_id)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use dali2rust_contracts::msg::{HclAlgorithm, HclOverrideTarget, HclTargetRow};

    use super::*;
    use crate::runtime::overrides::{Causes, OverrideLedger};

    const ALL_DAYS: u8 = 0x7F;
    const MOSCOW_LAT: i32 = 55_755_800;
    const MOSCOW_LON: i32 = 37_617_300;
    const MOSCOW_UTC_OFFSET: i16 = 180;
    const SUMMER_SOLSTICE: u16 = 172;
    const WINTER_SOLSTICE: u16 = 355;
    const ON_ARRIVAL: u16 = 600;

    struct Schedules(Vec<HclScheduleView>);

    impl HclScheduleReadPort for Schedules {
        fn hcl_schedule_view(&self, schedule_id: &str) -> Option<HclScheduleView> {
            self.0.iter().find(|s| s.schedule_id == schedule_id).cloned()
        }

        fn list_hcl_schedule_views(&self) -> Vec<HclScheduleView> {
            self.0.clone()
        }
    }

    fn schedule(id: &str, location: (Option<i32>, Option<i32>), targets: Vec<HclTargetRow>) -> HclScheduleView {
        HclScheduleView {
            schedule_id: id.to_string(),
            enabled: true,
            algorithm: HclAlgorithm::Stepped,
            active_days_mask: ALL_DAYS,
            latitude_microdeg: location.0,
            longitude_microdeg: location.1,
            targets,
            points: Vec::new(),
        }
    }

    fn located(latitude: i32, longitude: i32) -> HclScheduleView {
        schedule("located", (Some(latitude), Some(longitude)), Vec::new())
    }

    fn view(schedules: Vec<HclScheduleView>, ledger: OverrideLedger) -> HclRulesView {
        HclRulesView::new(Arc::new(Schedules(schedules)), Arc::new(Mutex::new(ledger)))
    }

    fn sun_of(schedules: Vec<HclScheduleView>, year_day: u16, utc_offset_minutes: i16) -> Option<SunMinutes> {
        let local = LocalCivilTime {
            minutes_since_midnight: ON_ARRIVAL,
            weekday: 0,
            year_day,
            utc_offset_minutes,
        };
        view(schedules, OverrideLedger::new()).sun(local)
    }

    fn sun(sunrise: u16, sunset: u16) -> Option<SunMinutes> {
        Some(SunMinutes { sunrise, sunset })
    }

    fn group(adapter_id: u8, group_id: u8) -> TargetKey {
        TargetKey { adapter_id, scope: HclTargetScope::Group, group_id }
    }

    fn broadcast(adapter_id: u8, group_id: u8) -> TargetKey {
        TargetKey { adapter_id, scope: HclTargetScope::Broadcast, group_id }
    }

    fn row(adapter_id: u8, scope: HclTargetScope, group_mask: u16) -> HclTargetRow {
        HclTargetRow { adapter_id, scope, group_mask }
    }

    #[test]
    fn the_sun_comes_from_the_first_schedule_with_a_whole_location() {
        let schedules = vec![
            schedule("none", (None, None), Vec::new()),
            schedule("half", (Some(MOSCOW_LAT), None), Vec::new()),
            located(MOSCOW_LAT, MOSCOW_LON),
            located(40_000_000, 10_000_000),
        ];
        assert_eq!(sun_of(schedules, SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET), sun(224, 1278));
    }

    #[test]
    fn the_sun_follows_the_season_and_the_hemisphere() {
        let moscow = || vec![located(MOSCOW_LAT, MOSCOW_LON)];
        assert_eq!(sun_of(moscow(), SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET), sun(224, 1278));
        assert_eq!(sun_of(moscow(), WINTER_SOLSTICE, MOSCOW_UTC_OFFSET), sun(537, 958));
        let cape_town = vec![located(-33_924_900, 18_424_100)];
        assert_eq!(sun_of(cape_town, SUMMER_SOLSTICE, 120), sun(471, 1064));
    }

    #[test]
    fn a_sun_event_outside_the_local_day_is_clamped_to_its_edge() {
        let moscow = || vec![located(MOSCOW_LAT, MOSCOW_LON)];
        assert_eq!(sun_of(moscow(), SUMMER_SOLSTICE, -60), sun(0, 1038));
        assert_eq!(sun_of(moscow(), SUMMER_SOLSTICE, 600), sun(644, 1439));
    }

    #[test]
    fn no_sun_without_a_located_schedule_or_on_a_day_without_sunrise() {
        let unlocated = vec![
            schedule("none", (None, None), Vec::new()),
            schedule("half", (Some(MOSCOW_LAT), None), Vec::new()),
        ];
        assert_eq!(sun_of(unlocated, SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET), None);
        assert_eq!(sun_of(Vec::new(), SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET), None);
        let midnight_sun = vec![located(66_900_000, 25_000_000)];
        assert_eq!(sun_of(midnight_sun, SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET), None);
    }

    #[test]
    fn targets_expand_per_group_and_carry_the_schedule_switch_and_its_overrides() {
        let mut ledger = OverrideLedger::new();
        let cause = Causes::of_hold(HclOverrideTarget::Broadcast);
        ledger.suspend("a", broadcast(0, 3), ON_ARRIVAL, cause);
        ledger.suspend("a", group(1, 5), ON_ARRIVAL, cause);
        ledger.suspend("b", group(0, 8), ON_ARRIVAL, cause);
        ledger.suspend("b", broadcast(0, 0), ON_ARRIVAL, cause);
        let a = schedule(
            "a",
            (None, None),
            vec![
                row(0, HclTargetScope::Broadcast, 0b1000),
                row(1, HclTargetScope::Group, (1 << 5) | (1 << 2)),
            ],
        );
        let b = HclScheduleView {
            enabled: false,
            ..schedule("b", (None, None), vec![row(0, HclTargetScope::Group, 1 << 7)])
        };
        let status = |target, enabled, overridden| HclTargetStatus { target, enabled, overridden };
        assert_eq!(
            view(vec![a, b], ledger).target_statuses(),
            vec![
                status(broadcast(0, 0), true, true),
                status(group(1, 2), true, false),
                status(group(1, 5), true, true),
                status(group(0, 7), false, false),
            ]
        );
    }

    #[test]
    fn a_suspended_broadcast_and_a_suspended_group_zero_are_told_apart_by_scope() {
        assert!(suspended_matches(&broadcast(0, 0), &broadcast(0, 0)));
        assert!(
            !suspended_matches(&broadcast(0, 0), &group(0, 0)),
            "a held broadcast key carries group id 0 and is not group 0"
        );
        assert!(suspended_matches(&group(0, 0), &group(0, 0)));
        assert!(!suspended_matches(&group(0, 0), &broadcast(0, 0)));
    }

    #[test]
    fn a_suspended_broadcast_matches_by_adapter_whatever_group_id_it_carries() {
        assert!(suspended_matches(&broadcast(0, 3), &broadcast(0, 0)));
        assert!(!suspended_matches(&broadcast(1, 0), &broadcast(0, 0)));
        assert!(!suspended_matches(&group(1, 5), &group(0, 5)));
        assert!(!suspended_matches(&group(0, 8), &group(0, 7)));
    }
}
