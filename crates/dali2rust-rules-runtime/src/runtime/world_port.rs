use crate::runtime::engine::{
    DeviceState, GroupState, HclTargetState, InputState, LampState, SunTimes, WallTime,
};

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
    fn input_instance_groups(
        &self,
        adapter_id: u8,
        short_address: u8,
        instance_number: u8,
    ) -> [Option<u8>; 3];
    fn hcl_schedules_for(&self, target: &dali2rust_rules_model::LightTarget) -> Vec<String>;
}
