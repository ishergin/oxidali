use dali2rust_rules_model::{
    Action, CctSpec, Condition, DaySet, DurationMs, FlowAction, GroupRef, InputEventMatch,
    InputRef, InputSelector, LampRef, LightAction, LightOp, LightTarget, Reading, Rule, RuleSet,
    SceneAction, TimeBound, TimeOfDay, Trigger, ValueExpr, Weekday,
};
use serde_json::{json, to_value};

fn input_ref() -> InputRef {
    InputRef {
        adapter_id: 0,
        device_short_address: 3,
        instance_number: 0,
    }
}

#[test]
fn input_event_trigger_matches_the_rest_contract_shape() {
    let trigger = Trigger::InputEvent {
        source: InputSelector::Instance(input_ref()),
        event: InputEventMatch::ShortPress,
    };
    assert_eq!(
        to_value(&trigger).unwrap(),
        json!({
            "kind": "input_event",
            "adapter_id": 0,
            "device_short_address": 3,
            "instance_number": 0,
            "event": "short_press"
        })
    );
}

#[test]
fn light_action_carries_kind_and_scoped_target() {
    let action = Action::Light(LightAction {
        op: LightOp::Toggle { level: None },
        target: LightTarget::Lamp(LampRef { adapter_id: 0, id: 7 }),
        hold_hcl: None,
    });
    assert_eq!(
        to_value(&action).unwrap(),
        json!({
            "kind": "light_toggle",
            "target": { "scope": "virtual_lamp", "adapter_id": 0, "id": 7 }
        })
    );
}

#[test]
fn the_projection_carries_no_fade() {
    let action = Action::Light(LightAction {
        op: LightOp::Toggle { level: None },
        target: LightTarget::Lamp(LampRef { adapter_id: 0, id: 7 }),
        hold_hcl: None,
    });
    let json = to_value(&action).unwrap();
    assert!(json.get("fade_ms").is_none(), "{json}");
}

#[test]
fn scene_recall_projects_scene_and_group_target() {
    let action = Action::Scene(SceneAction::Recall {
        scene: ValueExpr::Literal(13),
        target: Some(LightTarget::Group(GroupRef { adapter_id: 0, id: 2 })),
    });
    assert_eq!(
        to_value(&action).unwrap(),
        json!({
            "kind": "scene_recall",
            "scene": 13,
            "target": { "scope": "group", "adapter_id": 0, "id": 2 }
        })
    );
}

#[test]
fn event_context_reading_and_offset_expressions_project_flat() {
    let plain = ValueExpr::Reading(Reading::EventValue);
    assert_eq!(to_value(&plain).unwrap(), json!({ "value": "event_value" }));
    let offset = ValueExpr::Offset {
        reading: Reading::Var { name: "lvl".into() },
        delta: -10,
    };
    assert_eq!(
        to_value(&offset).unwrap(),
        json!({ "value": "var", "name": "lvl", "delta": -10 })
    );
}

#[test]
fn time_bounds_and_day_sets_project_as_written_forms() {
    let cond = Condition::TimeInRange {
        start: TimeBound::Clock(TimeOfDay { hour: 23, minute: 0 }),
        end: TimeBound::Solar {
            event: dali2rust_rules_model::SolarEvent::Sunrise,
            offset_ms: 900_000,
        },
    };
    assert_eq!(
        to_value(&cond).unwrap(),
        json!({
            "kind": "time_in_range",
            "start": "23:00",
            "end": { "event": "sunrise", "offset_ms": 900000 }
        })
    );
    let mut days = DaySet::empty();
    days.insert_range(Weekday::Sat, Weekday::Sun);
    assert_eq!(to_value(days).unwrap(), json!(["sat", "sun"]));
}

#[test]
fn rule_projection_is_snake_case_with_resolved_modifiers() {
    let rule = Rule {
        name: "коридор: вкл/выкл".into(),
        enabled: true,
        cooldown_ms: 0,
        hold_hcl: true,
        triggers: vec![Trigger::HttpTrigger],
        conditions: vec![],
        actions: vec![Action::Flow(FlowAction::Wait {
            duration_ms: DurationMs(500),
        })],
    };
    let set = RuleSet {
        lang_id: 1,
        blocks: vec![],
        rules: vec![rule],
    };
    let v = to_value(&set).unwrap();
    assert_eq!(v["lang_id"], 1);
    assert_eq!(v["rules"][0]["name"], "коридор: вкл/выкл");
    assert_eq!(v["rules"][0]["cooldown_ms"], 0);
    assert_eq!(v["rules"][0]["hold_hcl"], true);
    assert_eq!(v["rules"][0]["triggers"][0]["kind"], "http_trigger");
    assert_eq!(v["rules"][0]["actions"][0]["kind"], "wait");
    assert_eq!(v["rules"][0]["actions"][0]["duration_ms"], 500);
}

#[test]
fn cct_relative_and_group_selector_shapes() {
    let action = Action::Light(LightAction {
        op: LightOp::Cct {
            cct: CctSpec::Relative { delta_k: -200 },
        },
        target: LightTarget::Broadcast { adapter_id: 0 },
        hold_hcl: Some(false),
    });
    assert_eq!(
        to_value(&action).unwrap(),
        json!({
            "kind": "light_cct",
            "cct": { "delta_k": -200 },
            "target": { "scope": "broadcast", "adapter_id": 0 },
            "hold_hcl": false
        })
    );
}
