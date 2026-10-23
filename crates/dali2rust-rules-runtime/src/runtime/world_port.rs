use dali2rust_platform::wall_clock::LocalCivilTime;

use crate::runtime::engine::{
    DeviceState, GroupState, HclTargetState, InputState, LampState, SunTimes, WallTime,
};

impl From<LocalCivilTime> for WallTime {
    fn from(local: LocalCivilTime) -> Self {
        Self {
            minutes_of_day: local.minutes_since_midnight,
            weekday: local.weekday,
        }
    }
}

pub trait RulesWorldPort: Send + Sync {
    fn now_ms(&self) -> u64;
    fn unix_ms(&self) -> u64;
    fn wall(&self) -> Option<WallTime>;
    fn sun(&self) -> Option<SunTimes>;
    fn controller_active(&self) -> bool;
    fn lamps(&self) -> Vec<LampState>;
    fn groups(&self) -> Vec<GroupState>;
    fn devices(&self) -> Vec<DeviceState>;
    fn inputs(&self) -> Vec<InputState>;
    fn hcl(&self) -> Vec<HclTargetState>;
    fn hcl_schedules(&self) -> Vec<String>;
    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; 3];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_wall_time_is_the_local_minute_and_weekday_of_the_civil_time() {
        let local = LocalCivilTime {
            minutes_since_midnight: 23 * 60 + 5,
            weekday: 6,
            year_day: 200,
            utc_offset_minutes: -180,
        };
        assert_eq!(WallTime::from(local), WallTime { minutes_of_day: 1_385, weekday: 6 });
    }
}
