mod support;

use dali2rust_rules_model::{
    Action, ActionKind, Condition, ConditionKind, FlowAction, InputEventMatch, InputSelector,
    SceneAction, StateAction, TimeBound, Trigger, TriggerKind, ValueExpr,
};
use dali2rust_rules_lang::testing::{SECTION7_AWAY, SECTION7_BOOT, SECTION7_NIGHT, SECTION7_ONE_BUTTON};
use support::{compile_ok, the_rule, wrap_condition, wrap_trigger};

const TRIGGER_SNIPPETS: &[&str] = &[
    "input(dev=3, inst=0) is short_press",
    "input(\"панель-прихожая\", inst=1) is long_press_repeat",
    "input(group=2, type=button) is press",
    "input(dev=3, inst=0) is release",
    "input(dev=3, inst=0) is double_press",
    "input(dev=3, inst=0) is long_press_start",
    "input(dev=3, inst=0) is long_press_stop",
    "input(dev=3, inst=0) is button_free",
    "input(dev=3, inst=0) is button_stuck",
    "input(adapter=1, dev=3, inst=0) is press",
    "input(dev=5, inst=0) becomes occupied",
    "input(dev=5, inst=0) becomes vacant",
    "input(dev=5, inst=0) is movement",
    "input(dev=5, inst=0) is no_movement",
    "input(dev=6, inst=0).light crosses above 400",
    "input(dev=6, inst=0).light crosses below 150",
    "input(dev=7, inst=0).position changes",
    "input(dev=3, inst=2) is any_event",
    "input(dev=3, inst=2) is event(data=1)",
    "input device(dev=3) power cycled",
    "input device(dev=3) manual config changed",
    "at 07:30 on mon,tue,wed,thu,fri",
    "at sunrise-15m",
    "at sunset+30m",
    "every 15m",
    "lamp(\"коридор\") turns on",
    "lamp(\"коридор\") turns off",
    "lamp(\"коридор\").level crosses above 100",
    "group(\"кухня\") becomes any_on",
    "group(\"кухня\") becomes all_off",
    "scene(3) recalled",
    "device(3) goes offline",
    "device(3) comes online",
    "hcl override starts for group(\"кухня\")",
    "hcl override clears for group(\"кухня\")",
    "timer(\"прихожая\") fires",
    "controller starts",
    "controller becomes active",
    "rule(\"ночной режим\") fails",
    "http trigger",
];

#[test]
fn every_trigger_snippet_from_the_doc_parses() {
    for snippet in TRIGGER_SNIPPETS {
        let set = compile_ok(&wrap_trigger(snippet));
        assert_eq!(the_rule(&set).triggers.len(), 1, "snippet: {snippet}");
    }
}

const CONDITION_SNIPPETS: &[&str] = &[
    "time in 07:00 .. 23:00",
    "time in sunset-30m .. sunrise+15m",
    "day in mon..fri",
    "day in sat,sun",
    "sun is up",
    "sun is down",
    "lamp(\"X\") is on",
    "lamp(\"X\") is off",
    "lamp(\"X\").level >= 100",
    "lamp(\"X\").cct <= 3000",
    "group(\"кухня\") any_on",
    "group(\"кухня\") all_off",
    "input(dev=5, inst=0) is occupied",
    "input(dev=5, inst=0) is vacant",
    "input(dev=6, inst=0).light above 300",
    "input(dev=6, inst=0).light below 150",
    "hcl is enabled for group(\"кухня\")",
    "hcl is overridden for group(\"кухня\")",
    "device(3) is online",
    "rule(\"X\") is enabled",
    "last_fired(\"X\") older than 10m",
    "timer(\"X\") is running",
    "controller is active",
    "var(\"режим\") == \"ночь\"",
    "var(\"счётчик\") >= 3",
];

#[test]
fn every_condition_snippet_from_the_doc_parses() {
    for snippet in CONDITION_SNIPPETS {
        let set = compile_ok(&wrap_condition(snippet));
        assert_eq!(the_rule(&set).conditions.len(), 1, "snippet: {snippet}");
    }
}

#[test]
fn button_trigger_resolves_the_scoped_source() {
    let set = compile_ok(&wrap_trigger("input(dev=3, inst=0) is short_press"));
    match &the_rule(&set).triggers[0] {
        Trigger::InputEvent { source: InputSelector::Instance(input), event } => {
            assert_eq!(input.adapter_id, 0);
            assert_eq!(input.device_short_address, 3);
            assert_eq!(input.instance_number, 0);
            assert_eq!(*event, InputEventMatch::ShortPress);
        }
        other => panic!("expected InputEvent, got {other:?}"),
    }
}

#[test]
fn explicit_adapter_qualifier_lands_in_the_ref() {
    let set = compile_ok(&wrap_trigger("input(adapter=1, dev=3, inst=0) is press"));
    match &the_rule(&set).triggers[0] {
        Trigger::InputEvent { source: InputSelector::Instance(input), .. } => {
            assert_eq!(input.adapter_id, 1);
        }
        other => panic!("expected instance selector, got {other:?}"),
    }
}

#[test]
fn instance_group_selector_carries_group_and_type() {
    let set = compile_ok(&wrap_trigger("input(group=2, type=button) is press"));
    match &the_rule(&set).triggers[0] {
        Trigger::InputEvent { source: InputSelector::Group(group), .. } => {
            assert_eq!(group.instance_group, 2);
            assert_eq!(group.instance_type, Some(1));
        }
        other => panic!("expected group selector, got {other:?}"),
    }
}

#[test]
fn solar_trigger_offsets_are_signed_milliseconds() {
    let set = compile_ok(&wrap_trigger("at sunrise-15m"));
    match &the_rule(&set).triggers[0] {
        Trigger::AtSolar { offset_ms, .. } => assert_eq!(*offset_ms, -900_000),
        other => panic!("expected AtSolar, got {other:?}"),
    }
}

#[test]
fn midnight_crossing_time_range_keeps_both_bound_forms() {
    let set = compile_ok(&wrap_condition("time in sunset-30m .. sunrise+15m"));
    match &the_rule(&set).conditions[0] {
        Condition::TimeInRange { start, end } => {
            assert!(matches!(start, TimeBound::Solar { offset_ms: -1_800_000, .. }));
            assert!(matches!(end, TimeBound::Solar { offset_ms: 900_000, .. }));
        }
        other => panic!("expected TimeInRange, got {other:?}"),
    }
}

const SECTION3_SLIDER: &str = r#"
rule "слайдер яркости" {
  when input(dev=7, inst=0).position changes
  do   group("кухня").level(event.value)
}

rule "сценные кнопки панели" {
  when input(group=4, type=button) is short_press
  do   scene(event.option).recall(group("зал"))
       panel_select(group=4, selected=event.option)
}
"#;

#[test]
fn event_context_examples_from_section_3_compile() {
    let set = compile_ok(SECTION3_SLIDER);
    let slider = set.rule("слайдер яркости").unwrap();
    assert_eq!(slider.triggers[0].kind(), TriggerKind::InputPositionChange);
    assert_eq!(slider.actions[0].kind(), ActionKind::LightLevel);
    let panel = set.rule("сценные кнопки панели").unwrap();
    match &panel.actions[0] {
        Action::Scene(SceneAction::Recall { scene, target }) => {
            assert!(matches!(scene, ValueExpr::Reading(r) if r.kind().name() == "event_option"));
            assert!(target.is_some());
        }
        other => panic!("expected scene recall, got {other:?}"),
    }
    assert_eq!(panel.actions[1].kind(), ActionKind::PanelSelect);
}

const SECTION46_DEF: &str = r#"
def "ночной контур" {
  scene(13).recall(group("ночь"))
  timer("ночь-выкл").restart(3m)
}
"#;

#[test]
fn def_block_example_from_section_46_compiles() {
    let set = compile_ok(SECTION46_DEF);
    let block = set.block("ночной контур").unwrap();
    assert_eq!(block.actions.len(), 2);
    assert_eq!(block.actions[0].kind(), ActionKind::SceneRecall);
    assert_eq!(block.actions[1].kind(), ActionKind::TimerRestart);
}

#[test]
fn one_button_document_from_section_7_compiles() {
    let set = compile_ok(SECTION7_ONE_BUTTON);
    assert_eq!(set.rules.len(), 3);
    let toggle = set.rule("коридор: вкл/выкл").unwrap();
    assert_eq!(toggle.actions[0].kind(), ActionKind::LightToggle);
    assert_eq!(toggle.cooldown_ms, 0, "input-class default cooldown");
    match &set.rule("коридор: диммирование").unwrap().actions[0] {
        Action::Light(light) => {
            assert!(matches!(
                light.op,
                dali2rust_rules_model::LightOp::DimHold { rate_per_s: 60 }
            ));
        }
        other => panic!("expected a light action, got {other:?}"),
    }
}

#[test]
fn night_document_from_section_7_compiles() {
    let set = compile_ok(SECTION7_NIGHT);
    let entry = set.rule("ночь: вход").unwrap();
    assert_eq!(entry.triggers.len(), 2, "multi-when is OR");
    assert_eq!(entry.actions[0].kind(), ActionKind::Call);
    let off = set.rule("ночь: выключение").unwrap();
    assert_eq!(off.cooldown_ms, 200, "state-trigger default cooldown");
    assert_eq!(off.actions[1].kind(), ActionKind::HclResume);
}

const SECTION7_PANEL: &str = r#"
rule "зал: сцены с панели" {
  when input(group=4, type=button) is short_press
  do   scene(event.option).recall(group("зал"))
       panel_select(group=4, selected=event.option)
}

rule "зал: вернуть индикацию" {
  when input device(dev=3) power cycled
  do   panel_select(group=4, selected=var("зал-сцена"))
}
"#;

#[test]
fn scene_panel_document_from_section_7_compiles() {
    let set = compile_ok(SECTION7_PANEL);
    let restore = set.rule("зал: вернуть индикацию").unwrap();
    assert_eq!(restore.triggers[0].kind(), TriggerKind::InputDevicePowerCycled);
    match &restore.actions[0] {
        Action::Input(dali2rust_rules_model::InputAction::PanelSelect { selected, .. }) => {
            assert!(matches!(selected, ValueExpr::Reading(r) if r.kind().name() == "var"));
        }
        other => panic!("expected panel_select, got {other:?}"),
    }
}

#[test]
fn the_docs_own_examples_exceed_its_own_byte_limits() {
    let long_name = support::compile(
        "rule \"ночь: погашение с предупреждением\" { when http trigger do log(\"x\") }",
    )
    .unwrap_err();
    assert!(long_name.message.contains("rule name exceeds 48 bytes"), "{long_name}");
    let long_var = support::compile(
        "rule \"ушёл\" { when http trigger do var(\"режим\").set(\"отсутствие\") }",
    )
    .unwrap_err();
    assert!(long_var.message.contains("var text exceeds 16 bytes"), "{long_var}");
}

#[test]
fn away_document_from_section_7_compiles() {
    let set = compile_ok(SECTION7_AWAY);
    let away = the_named(&set, "ушёл");
    assert_eq!(away.actions.len(), 4);
    match &away.actions[2] {
        Action::State(StateAction::MqttPublish { topic, payload, retain }) => {
            assert_eq!(topic, "dali2rust/mode");
            assert_eq!(payload, "away");
            assert!(retain);
        }
        other => panic!("expected mqtt.publish, got {other:?}"),
    }
    match &away.actions[3] {
        Action::Flow(FlowAction::After { delay_ms, actions }) => {
            assert_eq!(delay_ms.0, 300_000);
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].kind(), ActionKind::HclResume);
        }
        other => panic!("expected after block, got {other:?}"),
    }
}

#[test]
fn boot_document_from_section_7_compiles() {
    let set = compile_ok(SECTION7_BOOT);
    let boot = the_named(&set, "boot: режим по умолчанию");
    assert_eq!(boot.triggers[0].kind(), TriggerKind::ControllerStarts);
    assert_eq!(boot.triggers[1].kind(), TriggerKind::ControllerBecomesActive);
}

#[test]
fn condition_kind_spot_checks() {
    let set = compile_ok(&wrap_condition("var(\"счётчик\") >= 3"));
    assert_eq!(the_rule(&set).conditions[0].kind(), ConditionKind::VarCompare);
    let set = compile_ok(&wrap_condition("last_fired(\"X\") older than 10m"));
    match &the_rule(&set).conditions[0] {
        Condition::LastFiredOlderThan { rule, than_ms } => {
            assert_eq!(rule, "X");
            assert_eq!(than_ms.0, 600_000);
        }
        other => panic!("expected LastFiredOlderThan, got {other:?}"),
    }
}

fn the_named<'a>(
    set: &'a dali2rust_rules_model::RuleSet,
    name: &str,
) -> &'a dali2rust_rules_model::Rule {
    set.rule(name).expect("rule present")
}
