use super::{collect_group_apply_diff, GroupApplyRowView, GroupApplySnapshot};
use dali2rust_contracts::msg::GroupMembershipAction;

fn row(vl: u8, desired: u16, applied: u16, short: Option<u8>) -> GroupApplyRowView {
    GroupApplyRowView {
        virtual_lamp_id: vl,
        desired_groups_mask: desired,
        applied_groups_mask: applied,
        binding_short: short,
    }
}

#[test]
fn diff_is_empty_when_desired_matches_applied() {
    let snapshot = GroupApplySnapshot {
        adapter_id: 0,
        rows: vec![row(0, 0b0101, 0b0101, Some(0))],
    };
    assert!(collect_group_apply_diff(&snapshot).is_empty());
}

#[test]
fn diff_yields_adds_and_removes_sorted_by_lamp_then_group() {
    let snapshot = GroupApplySnapshot {
        adapter_id: 0,
        rows: vec![
            row(5, 0b0010, 0b0001, Some(5)),
            row(1, 0b1000, 0b0000, None),
        ],
    };
    let diff = collect_group_apply_diff(&snapshot);
    let cells: Vec<(u8, u8, GroupMembershipAction, Option<u8>)> = diff
        .iter()
        .map(|c| (c.virtual_lamp_id, c.group_id, c.action, c.binding_short))
        .collect();
    assert_eq!(
        cells,
        vec![
            (1, 3, GroupMembershipAction::Add, None),
            (5, 0, GroupMembershipAction::Remove, Some(5)),
            (5, 1, GroupMembershipAction::Add, Some(5)),
        ]
    );
}

#[test]
fn diff_covers_the_full_matrix_when_everything_changes() {
    let rows = (0..64)
        .map(|vl| row(vl, 0xFFFF, 0x0000, Some(vl)))
        .collect();
    let snapshot = GroupApplySnapshot {
        adapter_id: 0,
        rows,
    };
    assert_eq!(collect_group_apply_diff(&snapshot).len(), 64 * 16);
}
