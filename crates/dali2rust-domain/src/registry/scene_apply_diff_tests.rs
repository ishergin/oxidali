use super::{collect_scene_apply_diff, SceneApplyRowView, SceneApplySnapshot};
use dali2rust_contracts::msg::{DaliSceneTargetState, PowerState, SceneProgramAction};

fn target(level: u8) -> DaliSceneTargetState {
    DaliSceneTargetState {
        power: Some(PowerState::On),
        level: Some(level),
        color: None,
    }
}

fn snapshot(rows: Vec<SceneApplyRowView>) -> SceneApplySnapshot {
    SceneApplySnapshot {
        adapter_id: 0,
        scene_id: 3,
        rows,
    }
}

#[test]
fn equal_rows_produce_empty_diff() {
    let rows = vec![
        SceneApplyRowView {
            virtual_lamp_id: 1,
            desired_included: true,
            desired_target: Some(target(100)),
            applied_included: true,
            applied_target: Some(target(100)),
            binding_short: Some(5),
        },
        SceneApplyRowView::default(),
    ];
    assert!(collect_scene_apply_diff(&snapshot(rows)).is_empty());
}

#[test]
fn write_update_clear_actions_are_classified_and_sorted() {
    let rows = vec![
        SceneApplyRowView {
            virtual_lamp_id: 9,
            desired_included: false,
            desired_target: None,
            applied_included: true,
            applied_target: Some(target(80)),
            binding_short: None,
        },
        SceneApplyRowView {
            virtual_lamp_id: 1,
            desired_included: true,
            desired_target: Some(target(100)),
            applied_included: false,
            applied_target: None,
            binding_short: Some(5),
        },
        SceneApplyRowView {
            virtual_lamp_id: 2,
            desired_included: true,
            desired_target: Some(target(180)),
            applied_included: true,
            applied_target: Some(target(100)),
            binding_short: Some(6),
        },
    ];
    let diff = collect_scene_apply_diff(&snapshot(rows));
    let kinds: Vec<_> = diff
        .iter()
        .map(|row| (row.virtual_lamp_id, row.action, row.target_state.is_some()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (1, SceneProgramAction::Write, true),
            (2, SceneProgramAction::Update, true),
            (9, SceneProgramAction::Clear, false),
        ]
    );
    assert_eq!(diff[0].scene_id, 3);
    assert_eq!(diff[2].binding_short, None, "unbound row keeps None");
}
