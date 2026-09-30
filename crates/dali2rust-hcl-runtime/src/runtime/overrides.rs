use std::collections::HashMap;

use dali2rust_contracts::msg::{
    HclLevelMode, HclOverrideTarget, HclSchedulePointRow, HclTargetScope, RuntimeSource,
    SetpointDimensions,
};
use dali2rust_domain::registry::{GroupApplyRowView, GroupReadPort};
use dali2rust_platform::small_sort::insertion_sort_by;

use super::plan::TargetKey;

use dali2rust_domain::registry::GROUP_COUNT;

const YEAR_DAY_RING: u16 = 366;

pub fn driven_dimensions(points: &[HclSchedulePointRow]) -> SetpointDimensions {
    SetpointDimensions {
        level: points
            .iter()
            .any(|point| point.level_mode != HclLevelMode::None),
        color: points
            .iter()
            .any(|point| point.color_temperature_kelvin.is_some()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeCommit {
    pub adapter_id: u8,
    pub virtual_lamp_id: Option<u8>,
    pub value_source: Option<RuntimeSource>,
    pub states: SetpointDimensions,
    pub holds_hcl: bool,
}

impl RuntimeCommit {
    fn is_foreign(&self) -> bool {
        !matches!(
            self.value_source,
            Some(RuntimeSource::Hcl)
                | Some(RuntimeSource::Poller)
                | Some(RuntimeSource::Readback)
                | None
        )
    }

    pub(crate) fn overrides(&self) -> bool {
        self.holds_hcl && self.is_foreign()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuspendedTarget {
    pub target: TargetKey,
    pub since_local_minutes: u16,
}

#[derive(Default)]
pub struct OverrideLedger {
    day: Option<u16>,
    suspended: HashMap<String, HashMap<TargetKey, u16>>,
}

impl OverrideLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn roll_over_to(&mut self, year_day: u16) -> Option<usize> {
        let rolled_forward = match self.day {
            None => true,
            Some(prev) if prev == year_day => return None,
            Some(prev) => {
                let forward = (year_day + YEAR_DAY_RING - prev) % YEAR_DAY_RING;
                let backward = (prev + YEAR_DAY_RING - year_day) % YEAR_DAY_RING;
                forward <= backward
            }
        };
        if !rolled_forward {
            return None;
        }
        let cleared = self.len();
        self.day = Some(year_day);
        self.suspended.clear();
        Some(cleared)
    }

    pub fn is_suspended(&self, schedule: &str, target: TargetKey) -> bool {
        self.suspended
            .get(schedule)
            .is_some_and(|targets| targets.contains_key(&target))
    }

    pub fn suspend(&mut self, schedule: &str, target: TargetKey, at_minutes: u16) -> bool {
        let mut started = false;
        self.suspended
            .entry(schedule.to_string())
            .or_default()
            .entry(target)
            .or_insert_with(|| {
                started = true;
                at_minutes
            });
        started
    }

    pub fn clear_schedule(&mut self, schedule: &str) -> usize {
        self.suspended
            .remove(schedule)
            .map_or(0, |targets| targets.len())
    }

    pub fn flags(&self) -> Vec<(String, TargetKey)> {
        self.suspended
            .iter()
            .flat_map(|(schedule, targets)| targets.keys().map(|target| (schedule.clone(), *target)))
            .collect()
    }

    pub fn lift(&mut self, flags: &[(String, TargetKey)]) -> usize {
        let mut lifted = 0;
        for (schedule, target) in flags {
            let Some(targets) = self.suspended.get_mut(schedule) else {
                continue;
            };
            lifted += usize::from(targets.remove(target).is_some());
            if targets.is_empty() {
                self.suspended.remove(schedule);
            }
        }
        lifted
    }

    pub fn suspended_targets(&self, schedule: &str) -> Vec<SuspendedTarget> {
        let Some(targets) = self.suspended.get(schedule) else {
            return Vec::new();
        };
        let mut rows: Vec<SuspendedTarget> = targets
            .iter()
            .map(|(target, since)| SuspendedTarget {
                target: *target,
                since_local_minutes: *since,
            })
            .collect();
        insertion_sort_by(&mut rows, |a, b| sort_key(&a.target) > sort_key(&b.target));
        rows
    }

    pub fn len(&self) -> usize {
        self.suspended.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn sort_key(target: &TargetKey) -> (u8, u8, u8) {
    let scope_rank = match target.scope {
        HclTargetScope::Broadcast => 0,
        HclTargetScope::Group => 1,
    };
    (target.adapter_id, scope_rank, target.group_id)
}

pub fn commit_hits_target(
    read_port: &dyn GroupReadPort,
    commit: &RuntimeCommit,
    target: TargetKey,
    driven: SetpointDimensions,
) -> bool {
    if !commit.overrides() || !commit.states.intersects(driven) {
        return false;
    }
    match commit.virtual_lamp_id {
        Some(lamp_id) => lamp_in_target(read_port, commit.adapter_id, lamp_id, target),
        None => commit.adapter_id == target.adapter_id && target.scope == HclTargetScope::Broadcast,
    }
}

pub fn hold_covers_target(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    held: HclOverrideTarget,
    target: TargetKey,
) -> bool {
    if adapter_id != target.adapter_id {
        return false;
    }
    match held {
        HclOverrideTarget::Broadcast => true,
        HclOverrideTarget::Group { group_id } => {
            group_covers_target(read_port, adapter_id, group_id, target)
        }
        HclOverrideTarget::VirtualLamp { virtual_lamp_id } => {
            lamp_in_target(read_port, adapter_id, virtual_lamp_id, target)
        }
    }
}

pub fn resume_covers_target(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    resumed: HclOverrideTarget,
    target: TargetKey,
) -> bool {
    if adapter_id != target.adapter_id {
        return false;
    }
    match (resumed, target.scope) {
        (HclOverrideTarget::Broadcast, _) => true,
        (_, HclTargetScope::Broadcast) => false,
        (HclOverrideTarget::Group { group_id }, HclTargetScope::Group) => {
            target.group_id == group_id
                || members_all(read_port, adapter_id, target.group_id, |row| in_group(row, group_id))
        }
        (HclOverrideTarget::VirtualLamp { virtual_lamp_id }, HclTargetScope::Group) => {
            let only_the_lamp = |row: &GroupApplyRowView| row.virtual_lamp_id == virtual_lamp_id;
            members_all(read_port, adapter_id, target.group_id, only_the_lamp)
        }
    }
}

fn members_all(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    group_id: u8,
    inside: impl Fn(&GroupApplyRowView) -> bool,
) -> bool {
    let Some(snapshot) = read_port.group_apply_snapshot(adapter_id) else {
        return false;
    };
    let mut members = snapshot.rows.iter().filter(|row| in_group(row, group_id)).peekable();
    members.peek().is_some() && members.all(inside)
}

fn in_group(row: &GroupApplyRowView, group_id: u8) -> bool {
    group_id < GROUP_COUNT && row.applied_groups_mask & (1u16 << group_id) != 0
}

fn group_covers_target(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    group_id: u8,
    target: TargetKey,
) -> bool {
    let members_groups = groups_of_members(read_port, adapter_id, group_id);
    match target.scope {
        HclTargetScope::Broadcast => members_groups != 0,
        HclTargetScope::Group => {
            target.group_id == group_id
                || (target.group_id < GROUP_COUNT && members_groups & (1u16 << target.group_id) != 0)
        }
    }
}

fn groups_of_members(read_port: &dyn GroupReadPort, adapter_id: u8, group_id: u8) -> u16 {
    if group_id >= GROUP_COUNT {
        return 0;
    }
    let Some(snapshot) = read_port.group_apply_snapshot(adapter_id) else {
        return 0;
    };
    snapshot
        .rows
        .iter()
        .filter(|row| row.applied_groups_mask & (1u16 << group_id) != 0)
        .fold(0, |groups, row| groups | row.applied_groups_mask)
}

fn lamp_in_target(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    virtual_lamp_id: u8,
    target: TargetKey,
) -> bool {
    if adapter_id != target.adapter_id {
        return false;
    }
    match target.scope {
        HclTargetScope::Broadcast => true,
        HclTargetScope::Group => {
            lamp_is_in_group(read_port, adapter_id, virtual_lamp_id, target.group_id)
        }
    }
}

fn lamp_is_in_group(
    read_port: &dyn GroupReadPort,
    adapter_id: u8,
    virtual_lamp_id: u8,
    group_id: u8,
) -> bool {
    if group_id >= GROUP_COUNT {
        return false;
    }
    let Some(snapshot) = read_port.group_apply_snapshot(adapter_id) else {
        return false;
    };
    snapshot
        .rows
        .iter()
        .find(|row| row.virtual_lamp_id == virtual_lamp_id)
        .is_some_and(|row| row.applied_groups_mask & (1u16 << group_id) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::{ColorMode, ColorValue, HclTargetScope, LightSetpoint, PowerState};
    use dali2rust_domain::registry::{
        AdapterReadPort, AdapterView, GroupApplyRowView, GroupApplySnapshot,
        GroupMembershipMatrixView, GroupView,
    };

    struct MembershipStub {
        rows: Vec<GroupApplyRowView>,
    }

    impl MembershipStub {
        fn with_lamp_in_groups(virtual_lamp_id: u8, groups: &[u8]) -> Self {
            Self { rows: Vec::new() }.and_lamp_in_groups(virtual_lamp_id, groups)
        }

        fn and_lamp_in_groups(mut self, virtual_lamp_id: u8, groups: &[u8]) -> Self {
            let applied = groups.iter().fold(0u16, |mask, group| mask | (1u16 << group));
            self.rows.push(GroupApplyRowView {
                virtual_lamp_id,
                desired_groups_mask: applied,
                applied_groups_mask: applied,
                binding_short: Some(virtual_lamp_id),
            });
            self
        }
    }

    impl AdapterReadPort for MembershipStub {
        fn adapter_count(&self) -> u8 {
            1
        }
        fn adapter_view(&self, _adapter_id: u8) -> Option<AdapterView> {
            None
        }
        fn list_adapter_views(&self) -> Vec<AdapterView> {
            Vec::new()
        }
    }

    impl GroupReadPort for MembershipStub {
        fn group_view(&self, _adapter_id: u8, _group_id: u8) -> Option<GroupView> {
            None
        }
        fn list_group_views(&self, _adapter_id: u8) -> Vec<GroupView> {
            Vec::new()
        }
        fn group_membership_matrix_view(&self, _adapter_id: u8) -> Option<GroupMembershipMatrixView> {
            None
        }
        fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
            Some(GroupApplySnapshot {
                adapter_id,
                rows: self.rows.clone(),
            })
        }
    }

    fn group_target(group_id: u8) -> TargetKey {
        TargetKey {
            adapter_id: 0,
            scope: HclTargetScope::Group,
            group_id,
        }
    }

    fn broadcast_target() -> TargetKey {
        TargetKey {
            adapter_id: 0,
            scope: HclTargetScope::Broadcast,
            group_id: 0,
        }
    }

    fn commit(source: RuntimeSource, lamp: Option<u8>) -> RuntimeCommit {
        RuntimeCommit {
            adapter_id: 0,
            virtual_lamp_id: lamp,
            value_source: Some(source),
            states: SetpointDimensions {
                level: true,
                color: false,
            },
            holds_hcl: true,
        }
    }

    fn drives_level() -> SetpointDimensions {
        SetpointDimensions {
            level: true,
            color: false,
        }
    }

    fn foreign_commit(setpoint: LightSetpoint) -> RuntimeCommit {
        RuntimeCommit {
            adapter_id: 0,
            virtual_lamp_id: Some(1),
            value_source: Some(RuntimeSource::Api),
            states: setpoint.dimensions(),
            holds_hcl: true,
        }
    }

    fn drives_color() -> SetpointDimensions {
        SetpointDimensions {
            level: false,
            color: true,
        }
    }

    fn point(level_mode: HclLevelMode, level: Option<u8>, kelvin: Option<u16>) -> HclSchedulePointRow {
        HclSchedulePointRow {
            time_ref: dali2rust_contracts::msg::HclTimeRef::Absolute,
            offset_minutes: 0,
            level_mode,
            level,
            color_temperature_kelvin: kelvin,
        }
    }

    #[test]
    fn the_schedules_own_commits_never_override_it() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        assert!(!commit_hits_target(
            &stub,
            &commit(RuntimeSource::Hcl, Some(1)),
            group_target(3),
            drives_level()
        ));
    }

    #[test]
    fn a_manual_commit_on_a_member_lamp_hits_the_group() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        for source in [
            RuntimeSource::Api,
            RuntimeSource::Mqtt,
            RuntimeSource::Sniffer,
        ] {
            assert!(
                commit_hits_target(&stub, &commit(source, Some(1)), group_target(3), drives_level()),
                "{source:?} must suspend the schedule"
            );
        }
    }

    #[test]
    fn a_commit_that_does_not_hold_leaves_the_schedule_running() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let spared = RuntimeCommit {
            holds_hcl: false,
            ..commit(RuntimeSource::Rules, Some(1))
        };
        assert!(
            !commit_hits_target(&stub, &spared, group_target(3), drives_level()),
            "a rule that said hold_hcl false moved the lamp without overriding the schedule"
        );
        assert!(
            commit_hits_target(&stub, &commit(RuntimeSource::Rules, Some(1)), group_target(3), drives_level()),
            "the same rule commit that holds does stand the schedule down"
        );
    }

    #[test]
    fn a_level_command_leaves_a_colour_only_schedule_running() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let dimmed = foreign_commit(LightSetpoint::from_level(200, None));
        assert!(
            !commit_hits_target(&stub, &dimmed, group_target(3), drives_color()),
            "a curve that only shifts colour owns no brightness to be overridden"
        );
        assert!(
            commit_hits_target(&stub, &dimmed, group_target(3), drives_level()),
            "the same command against a curve that does drive the level still stands it down"
        );
    }

    #[test]
    fn a_colour_command_stands_down_a_colour_only_schedule() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let recoloured = foreign_commit(LightSetpoint {
            power: PowerState::Unknown,
            level: None,
            color: Some(ColorValue {
                mode: ColorMode::Cct,
                color_temperature_kelvin: 3000,
                ..ColorValue::default()
            }),
        });
        assert!(commit_hits_target(
            &stub,
            &recoloured,
            group_target(3),
            drives_color()
        ));
        assert!(
            !commit_hits_target(&stub, &recoloured, group_target(3), drives_level()),
            "somebody else's colour is not a claim on the brightness"
        );
    }

    #[test]
    fn a_commit_that_states_nothing_overrides_nothing() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let silent = foreign_commit(LightSetpoint::default());
        for driven in [drives_level(), drives_color()] {
            assert!(!commit_hits_target(&stub, &silent, group_target(3), driven));
        }
    }

    #[test]
    fn a_schedule_drives_every_dimension_any_of_its_points_names() {
        let colour_only = [point(HclLevelMode::None, None, Some(4000))];
        assert_eq!(
            driven_dimensions(&colour_only),
            SetpointDimensions {
                level: false,
                color: true
            }
        );
        let mixed = [
            point(HclLevelMode::Absolute, Some(80), None),
            point(HclLevelMode::None, None, Some(4000)),
        ];
        assert_eq!(
            driven_dimensions(&mixed),
            SetpointDimensions {
                level: true,
                color: true
            }
        );
        let recall = [point(HclLevelMode::LastActive, None, None)];
        assert_eq!(
            driven_dimensions(&recall),
            SetpointDimensions {
                level: true,
                color: false
            }
        );
        assert_eq!(
            driven_dimensions(&[point(HclLevelMode::None, None, None)]),
            SetpointDimensions::default(),
            "a point that says nothing drives nothing"
        );
    }

    #[test]
    fn a_poller_readback_never_overrides_the_schedule() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        assert!(
            !commit_hits_target(&stub, &commit(RuntimeSource::Poller, Some(1)), group_target(3), drives_level()),
            "a background read reports the schedule's own level back at it"
        );
        assert!(!commit_hits_target(
            &stub,
            &commit(RuntimeSource::Poller, Some(1)),
            broadcast_target(),
            drives_level()
        ));
    }

    #[test]
    fn an_on_demand_readback_never_overrides_the_schedule() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        assert!(
            !commit_hits_target(&stub, &commit(RuntimeSource::Readback, Some(1)), group_target(3), drives_level()),
            "an operator's attribute read reports the gear, it does not drive it"
        );
        assert!(
            commit_hits_target(&stub, &commit(RuntimeSource::Api, Some(1)), group_target(3), drives_level()),
            "an operator's WRITE still stands the schedule down"
        );
    }

    #[test]
    fn a_lamp_with_no_row_and_a_group_beyond_the_mask_are_both_simply_not_members() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        assert!(!commit_hits_target(
            &stub,
            &commit(RuntimeSource::Api, Some(2)),
            group_target(3),
            drives_level()
        ));
        assert!(!commit_hits_target(
            &stub,
            &commit(RuntimeSource::Api, Some(1)),
            group_target(GROUP_COUNT),
            drives_level()
        ));
    }

    #[test]
    fn a_commit_on_a_lamp_outside_the_group_leaves_the_schedule_running() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        assert!(!commit_hits_target(
            &stub,
            &commit(RuntimeSource::Api, Some(1)),
            group_target(5),
            drives_level()
        ));
    }

    #[test]
    fn broadcast_targets_take_any_foreign_commit_on_their_adapter() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[]);
        assert!(commit_hits_target(
            &stub,
            &commit(RuntimeSource::Api, Some(1)),
            broadcast_target(),
            drives_level()
        ));
        let other_adapter = RuntimeCommit {
            adapter_id: 1,
            ..commit(RuntimeSource::Api, Some(1))
        };
        assert!(!commit_hits_target(
            &stub,
            &other_adapter,
            broadcast_target(),
            drives_level()
        ));
    }

    #[test]
    fn a_group_without_members_covers_itself_and_not_the_broadcast_key_that_shares_its_id() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let group_zero = HclOverrideTarget::Group { group_id: 0 };
        assert!(hold_covers_target(&stub, 0, group_zero, group_target(0)));
        assert!(
            !hold_covers_target(&stub, 0, group_zero, broadcast_target()),
            "a broadcast key carries group id 0 too; the scope tells them apart"
        );
        assert!(!hold_covers_target(&stub, 0, group_zero, group_target(3)));
        assert!(!hold_covers_target(&stub, 1, group_zero, group_target(0)), "another adapter");
    }

    #[test]
    fn a_group_hold_covers_what_a_commit_on_any_member_would() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3, 5]).and_lamp_in_groups(2, &[6]);
        let group_three = HclOverrideTarget::Group { group_id: 3 };
        for target in [group_target(3), group_target(5), group_target(6), broadcast_target()] {
            let member_commit = commit(RuntimeSource::Api, Some(1));
            assert_eq!(
                hold_covers_target(&stub, 0, group_three, target),
                commit_hits_target(&stub, &member_commit, target, drives_level()),
                "{target:?}"
            );
        }
        assert!(hold_covers_target(&stub, 0, group_three, broadcast_target()));
        assert!(hold_covers_target(&stub, 0, group_three, group_target(5)), "a shared member");
        assert!(!hold_covers_target(&stub, 0, group_three, group_target(6)));
        assert!(!hold_covers_target(&stub, 1, group_three, broadcast_target()), "another adapter");
    }

    #[test]
    fn a_broadcast_hold_covers_every_target_on_its_adapter() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[0]);
        for target in [broadcast_target(), group_target(0), group_target(9)] {
            assert!(hold_covers_target(&stub, 0, HclOverrideTarget::Broadcast, target), "{target:?}");
        }
        assert!(!hold_covers_target(&stub, 1, HclOverrideTarget::Broadcast, group_target(0)));
    }

    #[test]
    fn a_resume_lifts_only_targets_that_lie_inside_its_own() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3, 5]).and_lamp_in_groups(2, &[3, 9]);
        let group = |group_id| HclOverrideTarget::Group { group_id };
        let lamp = |virtual_lamp_id| HclOverrideTarget::VirtualLamp { virtual_lamp_id };
        let inside = |resumed, target| resume_covers_target(&stub, 0, resumed, target);

        assert!(inside(group(3), group_target(3)), "itself");
        assert!(inside(group(3), group_target(5)), "group 5 is lamp 1, which group 3 holds");
        assert!(inside(group(3), group_target(9)), "group 9 is lamp 2, which group 3 holds");
        assert!(!inside(group(5), group_target(3)), "group 3 is wider than group 5");
        assert!(!inside(group(3), broadcast_target()), "the broadcast is wider than any group");
        assert!(!inside(group(3), group_target(6)), "an empty group lies inside nothing but itself");
        assert!(inside(group(6), group_target(6)));
        assert!(inside(lamp(1), group_target(5)), "group 5 is lamp 1 alone");
        assert!(!inside(lamp(1), group_target(3)), "group 3 also holds lamp 2");
        assert!(!inside(lamp(1), broadcast_target()));
        for target in [broadcast_target(), group_target(3), group_target(12)] {
            assert!(inside(HclOverrideTarget::Broadcast, target), "{target:?}");
        }
        assert!(!resume_covers_target(&stub, 1, HclOverrideTarget::Broadcast, group_target(3)));
    }

    #[test]
    fn a_lamp_hold_covers_exactly_what_a_commit_on_that_lamp_would() {
        let stub = MembershipStub::with_lamp_in_groups(1, &[3]);
        let lamp = HclOverrideTarget::VirtualLamp { virtual_lamp_id: 1 };
        for target in [group_target(3), group_target(5), broadcast_target()] {
            let commit = commit(RuntimeSource::Api, Some(1));
            assert_eq!(
                hold_covers_target(&stub, 0, lamp, target),
                commit_hits_target(&stub, &commit, target, drives_level()),
                "{target:?}"
            );
        }
        assert!(hold_covers_target(&stub, 0, lamp, group_target(3)));
        assert!(!hold_covers_target(&stub, 0, lamp, group_target(5)));
        assert!(!hold_covers_target(&stub, 1, lamp, broadcast_target()), "another adapter");
    }

    #[test]
    fn a_flag_lasts_until_the_local_date_changes() {
        let mut ledger = OverrideLedger::new();
        let schedule = "morning";
        ledger.roll_over_to(172);
        assert!(ledger.suspend(schedule, group_target(3), 861));
        assert!(
            !ledger.suspend(schedule, group_target(3), 900),
            "already flagged"
        );
        assert!(ledger.is_suspended(schedule, group_target(3)));

        assert_eq!(
            ledger.roll_over_to(172),
            None,
            "the same day changes nothing"
        );
        assert!(ledger.is_suspended(schedule, group_target(3)));

        assert_eq!(
            ledger.roll_over_to(173),
            Some(1),
            "midnight clears the ledger"
        );
        assert!(!ledger.is_suspended(schedule, group_target(3)));
    }

    #[test]
    fn a_backwards_clock_correction_does_not_wipe_the_ledger() {
        let mut ledger = OverrideLedger::new();
        let _ = ledger.roll_over_to(172);
        ledger.suspend("morning", group_target(3), 861);
        assert_eq!(ledger.roll_over_to(171), None);
        assert!(
            ledger.is_suspended("morning", group_target(3)),
            "yesterday again is not a new day"
        );
        assert_eq!(ledger.roll_over_to(173), Some(1));
        assert!(
            !ledger.is_suspended("morning", group_target(3)),
            "the next forward midnight still clears"
        );
    }

    #[test]
    fn the_year_wrap_is_a_forward_roll() {
        let mut ledger = OverrideLedger::new();
        let _ = ledger.roll_over_to(365);
        ledger.suspend("nye", group_target(3), 1420);
        assert_eq!(ledger.roll_over_to(0), Some(1));
        assert!(!ledger.is_suspended("nye", group_target(3)));
    }

    #[test]
    fn overriding_one_target_leaves_the_schedules_other_targets_running() {
        let mut ledger = OverrideLedger::new();
        let schedule = "morning";
        ledger.suspend(schedule, group_target(3), 861);
        assert!(!ledger.is_suspended(schedule, group_target(5)));
        assert!(!ledger.is_suspended("evening", group_target(3)));
    }

    #[test]
    fn a_repeat_commit_keeps_the_time_the_override_started() {
        let mut ledger = OverrideLedger::new();
        ledger.suspend("morning", group_target(3), 861);
        ledger.suspend("morning", group_target(3), 1020);
        assert_eq!(
            ledger.suspended_targets("morning")[0].since_local_minutes,
            861
        );
    }

    #[test]
    fn a_reset_lifts_one_schedule_and_reports_what_it_held() {
        let mut ledger = OverrideLedger::new();
        ledger.suspend("morning", group_target(3), 861);
        ledger.suspend("morning", broadcast_target(), 870);
        ledger.suspend("evening", group_target(3), 880);

        assert_eq!(ledger.clear_schedule("morning"), 2);
        assert!(ledger.suspended_targets("morning").is_empty());
        assert!(
            ledger.is_suspended("evening", group_target(3)),
            "a reset is per schedule"
        );
        assert_eq!(ledger.clear_schedule("morning"), 0, "nothing left to lift");
    }

    #[test]
    fn suspended_targets_report_in_a_stable_order() {
        let mut ledger = OverrideLedger::new();
        ledger.suspend("morning", group_target(9), 900);
        ledger.suspend("morning", group_target(3), 861);
        ledger.suspend("morning", broadcast_target(), 870);
        let rows = ledger.suspended_targets("morning");
        let groups: Vec<u8> = rows.iter().map(|row| row.target.group_id).collect();
        assert_eq!(groups, vec![0, 3, 9], "broadcast first, then groups by id");
    }
}
