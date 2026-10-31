use dali2rust_contracts::msg::{PolicyException, PolicyExceptionList};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HclScheduleView {
    pub schedule_id: String,
    pub enabled: bool,
    pub algorithm: dali2rust_contracts::msg::HclAlgorithm,
    pub active_days_mask: u8,
    pub latitude_microdeg: Option<i32>,
    pub longitude_microdeg: Option<i32>,
    pub targets: Vec<dali2rust_contracts::msg::HclTargetRow>,
    pub points: Vec<dali2rust_contracts::msg::HclSchedulePointRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HclOverrideTargetView {
    pub adapter_id: u8,
    pub scope: dali2rust_contracts::msg::HclTargetScope,
    pub group_id: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HclOverrideView {
    pub suspended: bool,
    pub targets: Vec<HclOverrideTargetView>,
    pub since_local_minutes: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollerSettingsView {
    pub enabled: bool,
    pub interval_ms: u32,
    pub attribute_groups_mask: u8,
    pub include_dt8_color: bool,
    pub include_energy: bool,
    pub include_diagnostics: bool,
    pub skip_unbound_virtual_lamps: bool,
    pub power_cycle_watch: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DaliSettingsView {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
    pub power_cycle_notification: bool,
    pub random_address: u32,
    pub device_groups: u32,
    pub operating_mode: u8,
    pub set_by: BusUnitSetBy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusUnitSetter {
    #[default]
    Unrecorded,
    Operator,
    Wire,
    Redundancy,
}

impl BusUnitSetter {
    #[must_use]
    pub const fn mover(self) -> ApplicationActiveMover {
        match self {
            Self::Unrecorded => ApplicationActiveMover::Unmoved,
            Self::Operator | Self::Redundancy => ApplicationActiveMover::Command,
            Self::Wire => ApplicationActiveMover::Wire,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusUnitSetBy {
    pub short_address: BusUnitSetter,
    pub random_address: BusUnitSetter,
    pub device_groups: BusUnitSetter,
    pub operating_mode: BusUnitSetter,
    pub power_cycle_notification: BusUnitSetter,
    pub application_active: BusUnitSetter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationActiveMover {
    #[default]
    Unmoved,
    Command,
    Wire,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedundancySettingsView {
    pub enabled: bool,
    pub standby_role: bool,
    pub probe_interval_ms: u32,
    pub takeover_after_missed: u8,
    pub boot_listen_ms: u32,
    pub peer_device_short_address: Option<u8>,
    pub peer_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PoliciesView {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub apply_on_discovery: bool,
    pub exceptions: PolicyExceptionList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GearPolicyLevels {
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
}

impl PoliciesView {
    #[must_use]
    pub fn manages_anything(&self) -> bool {
        self.system_failure_level.is_some() || self.power_on_level.is_some()
    }

    #[must_use]
    pub fn excepts(&self, device: PolicyException) -> bool {
        self.exceptions.contains(&device)
    }

    #[must_use]
    pub fn mismatch(&self, device: PolicyException, held: GearPolicyLevels) -> Option<bool> {
        if self.excepts(device) {
            return None;
        }
        let comparisons = [
            (self.system_failure_level, held.system_failure_level),
            (self.power_on_level, held.power_on_level),
        ];
        let mut managed = comparisons.into_iter().filter(|(policy, _)| policy.is_some()).peekable();
        managed.peek()?;
        let mut every_level_seen = true;
        for (policy, held) in managed {
            match held {
                Some(held) if Some(held) != policy => return Some(true),
                Some(_) => {}
                None => every_level_seen = false,
            }
        }
        every_level_seen.then_some(false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollTargetView {
    pub short_address: u8,
    pub is_dt8: bool,
    pub is_dt6: bool,
    pub declares_energy: bool,
    pub declares_diagnostics: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollTargetSelection {
    pub targets: Vec<PollTargetView>,
    pub excluded_unbound: u16,
    pub adapter_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyApplyCell {
    pub short_address: u8,
    pub system_failure_level: Option<u8>,
    pub power_on_level: Option<u8>,
    pub known_device_types: Option<dali2rust_contracts::msg::DeviceTypeSet>,
    pub excepted: bool,
}

#[cfg(test)]
mod policy_mismatch_tests {
    use super::*;

    const DEVICE: PolicyException = PolicyException { adapter_id: 0, short_address: 4 };
    const OTHER: PolicyException = PolicyException { adapter_id: 0, short_address: 5 };

    fn policy(system_failure_level: Option<u8>, power_on_level: Option<u8>) -> PoliciesView {
        PoliciesView { system_failure_level, power_on_level, ..PoliciesView::default() }
    }

    fn held(system_failure_level: Option<u8>, power_on_level: Option<u8>) -> GearPolicyLevels {
        GearPolicyLevels { system_failure_level, power_on_level }
    }

    #[test]
    fn an_unmanaged_policy_states_nothing_whatever_the_gear_holds() {
        assert_eq!(policy(None, None).mismatch(DEVICE, held(Some(1), Some(2))), None);
    }

    #[test]
    fn an_excepted_device_states_nothing_and_only_that_device_is_excepted() {
        let mut excepting = policy(Some(254), None);
        excepting.exceptions.push(DEVICE).expect("capacity");
        assert_eq!(excepting.mismatch(DEVICE, held(Some(0), None)), None);
        assert_eq!(excepting.mismatch(OTHER, held(Some(0), None)), Some(true));
    }

    #[test]
    fn a_managed_level_never_observed_states_nothing() {
        assert_eq!(policy(Some(254), Some(100)).mismatch(DEVICE, held(None, None)), None);
    }

    #[test]
    fn either_managed_level_that_differs_is_a_mismatch() {
        let managed = policy(Some(254), Some(100));
        assert_eq!(managed.mismatch(DEVICE, held(Some(0), Some(100))), Some(true));
        assert_eq!(managed.mismatch(DEVICE, held(Some(254), Some(50))), Some(true));
        assert_eq!(managed.mismatch(DEVICE, held(None, Some(50))), Some(true), "one unread level does not hide a drifted one");
    }

    #[test]
    fn a_match_is_stated_only_when_every_managed_level_was_observed() {
        let managed = policy(Some(254), Some(100));
        assert_eq!(managed.mismatch(DEVICE, held(Some(254), Some(100))), Some(false));
        assert_eq!(managed.mismatch(DEVICE, held(Some(254), None)), None);
        assert_eq!(managed.mismatch(DEVICE, held(None, Some(100))), None);
    }

    #[test]
    fn an_unmanaged_level_is_never_compared() {
        let power_on_only = policy(None, Some(100));
        assert_eq!(power_on_only.mismatch(DEVICE, held(Some(7), Some(100))), Some(false));
        assert_eq!(power_on_only.mismatch(DEVICE, held(Some(7), None)), None);
    }
}
