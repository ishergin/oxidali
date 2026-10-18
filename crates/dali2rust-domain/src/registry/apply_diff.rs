use serde::{Deserialize, Serialize};
use dali2rust_contracts::msg::{GroupMembershipAction, SceneProgramAction};
use dali2rust_platform::small_sort::insertion_sort_by;

use super::limits::GROUP_COUNT;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneApplyRowView {
    pub virtual_lamp_id: u8,
    pub desired_included: bool,
    pub desired_target: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub applied_included: bool,
    pub applied_target: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub binding_short: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneApplySnapshot {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub rows: Vec<SceneApplyRowView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneApplyDiffRow {
    pub virtual_lamp_id: u8,
    pub scene_id: u8,
    pub action: dali2rust_contracts::msg::SceneProgramAction,
    pub target_state: Option<dali2rust_contracts::msg::DaliSceneTargetState>,
    pub binding_short: Option<u8>,
}

#[must_use]
pub fn scene_targets_converged(
    desired: Option<&dali2rust_contracts::msg::DaliSceneTargetState>,
    applied: Option<&dali2rust_contracts::msg::DaliSceneTargetState>,
) -> bool {
    match (desired, applied) {
        (Some(desired), Some(applied)) => desired.stores_as(applied),
        (desired, applied) => desired == applied,
    }
}

pub fn collect_scene_apply_diff(snapshot: &SceneApplySnapshot) -> Vec<SceneApplyDiffRow> {
    let mut diff = Vec::new();
    for row in &snapshot.rows {
        let action = match (row.desired_included, row.applied_included) {
            (true, false) => SceneProgramAction::Write,
            (true, true)
                if !scene_targets_converged(row.desired_target.as_ref(), row.applied_target.as_ref()) =>
            {
                SceneProgramAction::Update
            }
            (false, true) => SceneProgramAction::Clear,
            _ => continue,
        };
        diff.push(SceneApplyDiffRow {
            virtual_lamp_id: row.virtual_lamp_id,
            scene_id: snapshot.scene_id,
            action,
            target_state: (action != SceneProgramAction::Clear)
                .then_some(row.desired_target)
                .flatten(),
            binding_short: row.binding_short,
        });
    }
    insertion_sort_by(&mut diff, |a, b| a.virtual_lamp_id > b.virtual_lamp_id);
    diff
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupApplyRowView {
    pub virtual_lamp_id: u8,
    pub desired_groups_mask: u16,
    pub applied_groups_mask: u16,
    pub binding_short: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupApplySnapshot {
    pub adapter_id: u8,
    pub rows: Vec<GroupApplyRowView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupApplyDiffCell {
    pub virtual_lamp_id: u8,
    pub group_id: u8,
    pub action: dali2rust_contracts::msg::GroupMembershipAction,
    pub binding_short: Option<u8>,
}

pub fn collect_group_apply_diff(snapshot: &GroupApplySnapshot) -> Vec<GroupApplyDiffCell> {
    let mut diff = Vec::new();
    for row in &snapshot.rows {
        let changed = row.desired_groups_mask ^ row.applied_groups_mask;
        for group_id in 0..GROUP_COUNT {
            let bit = 1u16 << group_id;
            if changed & bit == 0 {
                continue;
            }
            diff.push(GroupApplyDiffCell {
                virtual_lamp_id: row.virtual_lamp_id,
                group_id,
                action: if row.desired_groups_mask & bit != 0 {
                    GroupMembershipAction::Add
                } else {
                    GroupMembershipAction::Remove
                },
                binding_short: row.binding_short,
            });
        }
    }
    insertion_sort_by(&mut diff, |a, b| {
        (a.virtual_lamp_id, a.group_id) > (b.virtual_lamp_id, b.group_id)
    });
    diff
}
