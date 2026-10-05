use dali2rust_rules_lang::testing::{SECTION7_AWAY, SECTION7_BOOT, SECTION7_NIGHT, SECTION7_ONE_BUTTON};
use dali2rust_rules_lang::RulesLangV1;
use dali2rust_rules_model::testing::StubResolver;
use dali2rust_rules_model::{
    Action, LightTarget, NameResolver, Rule, RuleCompiler, RuleSet, StateAction, Trigger,
};
use dali2rust_rules_runtime::{
    Effect, Engine, EngineInput, InputEventKind, LampState, LightVerb, PartialReason, SunTimes,
    WallTime, WorldSnapshot,
};

fn resolver() -> StubResolver {
    StubResolver::permissive()
}

fn compile(source: &str) -> RuleSet {
    match RulesLangV1.compile(source, &resolver()) {
        Ok(set) => set,
        Err(e) => panic!("fixture must compile: {e}\n---\n{source}"),
    }
}

fn engine(source: &str, now_ms: u64) -> Engine {
    let mut engine = Engine::new(now_ms);
    engine.set_rules(Some(compile(source)), now_ms);
    engine
}

fn world(now_ms: u64) -> WorldSnapshot {
    WorldSnapshot::empty(now_ms)
}

fn with_lamp(mut w: WorldSnapshot, lamp: dali2rust_rules_model::LampRef, is_on: bool, level: u8) -> WorldSnapshot {
    w.lamps.push(LampState {
        adapter_id: lamp.adapter_id,
        id: lamp.id,
        is_on,
        level: Some(level),
        cct_kelvin: None,
        last_level: Some(level),
        bound: true,
    });
    w
}

fn with_unknown_level(mut w: WorldSnapshot, lamp: dali2rust_rules_model::LampRef) -> WorldSnapshot {
    w.lamps.push(LampState {
        adapter_id: lamp.adapter_id,
        id: lamp.id,
        is_on: true,
        level: None,
        cct_kelvin: None,
        last_level: None,
        bound: true,
    });
    w
}

fn lamp_level_edge(
    lamp: dali2rust_rules_model::LampRef,
    was_on: bool,
    previous_level: Option<u8>,
    level: Option<u8>,
) -> EngineInput<'static> {
    EngineInput::LampChanged {
        adapter_id: lamp.adapter_id,
        lamp_id: lamp.id,
        is_on: true,
        level,
        was_on,
        previous_level,
    }
}

fn night_world(now_ms: u64, minutes: u16, weekday: u8) -> WorldSnapshot {
    let mut w = world(now_ms);
    w.wall = Some(WallTime {
        minutes_of_day: minutes,
        weekday,
    });
    w.sun = Some(SunTimes {
        sunrise_min: 330,
        sunset_min: 1_260,
    });
    w
}

fn button(sa: u8, inst: u8, kind: InputEventKind) -> EngineInput<'static> {
    EngineInput::InputEvent {
        adapter_id: 0,
        short_address: Some(sa),
        instance_number: Some(inst),
        instance_type: Some(1),
        instance_groups: [None; 3],
        kind,
        value: 0,
    }
}

fn occupancy(sa: u8, inst: u8, kind: InputEventKind) -> EngineInput<'static> {
    EngineInput::InputEvent {
        adapter_id: 0,
        short_address: Some(sa),
        instance_number: Some(inst),
        instance_type: Some(3),
        instance_groups: [None; 3],
        kind,
        value: 0,
    }
}

fn run(name: &str) -> EngineInput<'_> {
    EngineInput::RunRule { name, dry: false }
}

fn lamp_on_edge(lamp: dali2rust_rules_model::LampRef, on: bool) -> EngineInput<'static> {
    EngineInput::LampChanged {
        adapter_id: lamp.adapter_id,
        lamp_id: lamp.id,
        is_on: on,
        level: if on { Some(200) } else { Some(0) },
        was_on: !on,
        previous_level: if on { Some(0) } else { Some(200) },
    }
}

#[test]
fn short_press_toggles_off_the_read_model_with_fade() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let mut eng = engine(SECTION7_ONE_BUTTON, 0);

    let dark = with_lamp(world(1_000), lamp, false, 0);
    let out = eng.handle(button(3, 0, InputEventKind::ShortPress), &dark);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rule, "коридор: вкл/выкл");
    assert_eq!(out[0].trigger_kind, "input_event");
    assert_eq!(
        out[0].effects,
        vec![Effect::Light {
            target: LightTarget::Lamp(lamp),
            verb: LightVerb::On { level: None },
            hold_hcl: true,
        }]
    );

    let lit = with_lamp(world(2_000), lamp, true, 200);
    let out = eng.handle(button(3, 0, InputEventKind::ShortPress), &lit);
    assert_eq!(
        out[0].effects,
        vec![Effect::Light {
            target: LightTarget::Lamp(lamp),
            verb: LightVerb::Off,
            hold_hcl: true,
        }]
    );
}

#[test]
fn dim_hold_steps_by_elapsed_time_not_by_event_count() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let mut eng = engine(SECTION7_ONE_BUTTON, 0);
    let dim = |eng: &mut Engine, at: u64| {
        let out = eng.handle(button(3, 0, InputEventKind::LongPressRepeat), &world(at));
        assert_eq!(out.len(), 1, "the dim rule fires per repeat");
        out[0].effects.clone()
    };
    let step = |delta: i16| {
        vec![Effect::Light {
            target: LightTarget::Lamp(lamp),
            verb: LightVerb::Dim { delta },
            hold_hcl: true,
        }]
    };

    assert!(eng
        .handle(button(3, 0, InputEventKind::LongPressStart), &world(10_000))
        .is_empty());
    assert_eq!(dim(&mut eng, 10_500), step(30));
    assert_eq!(dim(&mut eng, 11_000), step(30));
    assert_eq!(dim(&mut eng, 12_000), step(60));
    assert!(eng
        .handle(button(3, 0, InputEventKind::LongPressStop), &world(12_100))
        .is_empty());

    assert_eq!(dim(&mut eng, 20_000), Vec::new());
    assert_eq!(dim(&mut eng, 20_400), step(24));
}

#[test]
fn double_press_asks_full_light() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let mut eng = engine(SECTION7_ONE_BUTTON, 0);
    let out = eng.handle(button(3, 0, InputEventKind::DoublePress), &world(1_000));
    assert_eq!(
        out[0].effects,
        vec![Effect::Light {
            target: LightTarget::Lamp(lamp),
            verb: LightVerb::On { level: Some(254) },
            hold_hcl: true,
        }]
    );
}

#[test]
fn night_document_runs_sensor_or_button_through_timer_to_lights_out() {
    let group = resolver().resolve_group("ночь").unwrap();
    let target = LightTarget::Group(group);
    let mut eng = engine(SECTION7_NIGHT, 0);
    let recall = vec![Effect::SceneRecall {
        scene: 13,
        target: Some(target),
        hold_hcl: true,
    }];

    let out = eng.handle(occupancy(5, 0, InputEventKind::BecameOccupied), &night_world(100_000, 1_410, 0));
    assert_eq!(out[0].rule, "ночь: вход");
    assert_eq!(out[0].effects, recall);

    let out = eng.handle(button(3, 0, InputEventKind::ShortPress), &night_world(101_000, 1_410, 0));
    assert_eq!(out[0].rule, "ночь: вход");
    assert_eq!(out[0].effects, recall);

    let before = eng.counters().conditions_rejected;
    let out = eng.handle(occupancy(5, 0, InputEventKind::BecameOccupied), &night_world(102_000, 720, 0));
    assert!(out.is_empty());
    assert_eq!(eng.counters().conditions_rejected, before + 1);

    let out = eng.handle(occupancy(5, 0, InputEventKind::BecameVacant), &night_world(200_000, 1_420, 0));
    assert_eq!(out[0].rule, "ночь: погашение");
    assert_eq!(
        out[0].effects,
        vec![Effect::Light {
            target,
            verb: LightVerb::Level { level: 10 },
            hold_hcl: true,
        }]
    );
    assert_eq!(eng.next_deadline_ms(), Some(230_000));

    let out = eng.handle(EngineInput::Tick, &night_world(230_000, 1_421, 0));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rule, "ночь: выключение");
    assert_eq!(out[0].trigger_kind, "timer_fires");
    assert_eq!(
        out[0].effects,
        vec![
            Effect::Light {
                target,
                verb: LightVerb::Off,
                hold_hcl: true,
            },
            Effect::HclResume { target },
        ]
    );
}

#[test]
fn reentering_the_night_circuit_cancels_the_pending_timer() {
    let mut eng = engine(SECTION7_NIGHT, 0);
    eng.handle(occupancy(5, 0, InputEventKind::BecameVacant), &night_world(10_000, 1_400, 0));
    assert_eq!(eng.next_deadline_ms(), Some(40_000));
    eng.handle(occupancy(5, 0, InputEventKind::BecameOccupied), &night_world(12_000, 1_400, 0));
    assert_eq!(eng.next_deadline_ms(), None);
    assert!(eng.handle(EngineInput::Tick, &night_world(40_000, 1_401, 0)).is_empty());
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

rule "зал: запомнить" {
  when http trigger
  do   var("зал-сцена").set(2)
}
"#;

fn panel_button(option: u8) -> EngineInput<'static> {
    EngineInput::InputEvent {
        adapter_id: 0,
        short_address: Some(3),
        instance_number: Some(option),
        instance_type: Some(1),
        instance_groups: [Some(option), Some(4), None],
        kind: InputEventKind::ShortPress,
        value: 2,
    }
}

#[test]
fn scene_panel_routes_event_option_to_recall_and_radio_buttons() {
    let hall = resolver().resolve_group("зал").unwrap();
    let mut eng = engine(SECTION7_PANEL, 0);
    let out = eng.handle(panel_button(3), &world(1_000));
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].effects,
        vec![
            Effect::SceneRecall {
                scene: 3,
                target: Some(LightTarget::Group(hall)),
                hold_hcl: true,
            },
            Effect::PanelSelect {
                adapter_id: 0,
                group: 4,
                selected: 3,
            },
        ]
    );
}

#[test]
fn power_cycle_reasserts_the_remembered_selection_and_fails_honestly_without_one() {
    let cycled = EngineInput::PowerCycled {
        adapter_id: 0,
        short_address: Some(3),
    };
    let mut eng = engine(SECTION7_PANEL, 0);
    let out = eng.handle(cycled.clone(), &world(1_000));
    assert_eq!(out.len(), 1);
    assert!(out[0].effects.is_empty());
    assert_eq!(out[0].partial, Some(PartialReason::ConditionUnevaluable));
    assert_eq!(eng.counters().actions_failed, 1);

    eng.handle(run("зал: запомнить"), &world(2_000));
    let out = eng.handle(cycled, &world(3_000));
    assert_eq!(
        out[0].effects,
        vec![Effect::PanelSelect {
            adapter_id: 0,
            group: 4,
            selected: 2,
        }]
    );
}

#[test]
fn away_switches_everything_off_and_returns_hcl_five_minutes_later() {
    let broadcast = LightTarget::Broadcast { adapter_id: 0 };
    let mut eng = engine(SECTION7_AWAY, 0);
    let out = eng.handle(button(3, 2, InputEventKind::LongPressStart), &world(10_000));
    assert_eq!(
        out[0].effects,
        vec![
            Effect::Light {
                target: broadcast,
                verb: LightVerb::Off,
                hold_hcl: true,
            },
            Effect::MqttPublish {
                topic: "dali2rust/mode".into(),
                payload: "away".into(),
                retain: true,
            },
        ]
    );
    assert_eq!(eng.counters().continuations_scheduled, 1);
    assert_eq!(eng.next_deadline_ms(), Some(310_000));

    assert!(eng.handle(EngineInput::Tick, &world(309_000)).is_empty());
    let out = eng.handle(EngineInput::Tick, &world(310_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rule, "ушёл");
    assert_eq!(out[0].trigger_kind, "input_event");
    assert_eq!(out[0].effects, vec![Effect::HclResume { target: broadcast }]);
    assert_eq!(eng.counters().continuations_fired, 1);
}

#[test]
fn boot_fires_controller_starts_and_the_active_edge_dedupes_on_cooldown() {
    let mut eng = engine(SECTION7_BOOT, 0);
    let out = eng.handle(EngineInput::ControllerStarts, &world(1_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].trigger_kind, "controller_starts");
    assert_eq!(
        out[0].effects,
        vec![Effect::PanelSelect {
            adapter_id: 0,
            group: 4,
            selected: 1,
        }]
    );
    let out = eng.handle(EngineInput::ControllerActive { active: true }, &world(1_050));
    assert!(out.is_empty());
    assert_eq!(eng.counters().suppressed_cooldown, 1);
    assert_eq!(eng.counters().vars_in_use, 1);

    let out = eng.handle(EngineInput::ControllerActive { active: true }, &world(60_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].trigger_kind, "controller_becomes_active");
}

#[test]
fn state_trigger_default_cooldown_suppresses_and_counts() {
    let src = r#"rule "с" { when lamp("х") turns on do log("s") }"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    assert_eq!(eng.handle(lamp_on_edge(lamp, true), &world(1_000)).len(), 1);
    assert!(eng.handle(lamp_on_edge(lamp, true), &world(1_100)).is_empty());
    assert_eq!(eng.counters().suppressed_cooldown, 1);
    assert_eq!(eng.handle(lamp_on_edge(lamp, true), &world(1_300)).len(), 1);
}

#[test]
fn one_occupancy_report_wakes_an_any_event_rule_once() {
    for trigger in ["any_event", "event(data=0)"] {
        let src = format!("rule \"любое\" {{ when input(dev=5, inst=0) is {trigger} do log(\"a\") }}");
        let mut eng = engine(&src, 0);
        let wire = eng.handle(occupancy(5, 0, InputEventKind::Movement), &world(1_000)).len();
        let edge = eng.handle(occupancy(5, 0, InputEventKind::BecameOccupied), &world(1_000)).len();
        assert_eq!(
            (wire, edge),
            (1, 0),
            "{trigger}: the occupancy edge is derived from the same report, not a second event"
        );
    }
}

#[test]
fn input_class_default_cooldown_is_zero() {
    let src = r#"rule "и" { when input(dev=1, inst=0) is press do log("p") }"#;
    let mut eng = engine(src, 0);
    assert_eq!(eng.handle(button(1, 0, InputEventKind::Press), &world(1_000)).len(), 1);
    assert_eq!(eng.handle(button(1, 0, InputEventKind::Press), &world(1_001)).len(), 1);
    assert_eq!(eng.counters().suppressed_cooldown, 0);
}

#[test]
fn wait_splits_the_activation_and_resumes_at_its_deadline() {
    let src = r#"
rule "пауза" {
  when http trigger
  do   log("до")
       wait 500ms
       log("после")
}
"#;
    let mut eng = engine(src, 0);
    let out = eng.handle(run("пауза"), &world(10_000));
    assert_eq!(out[0].effects, vec![Effect::Log { text: "до".into() }]);
    assert_eq!(eng.next_deadline_ms(), Some(10_500));

    assert!(eng.handle(EngineInput::Tick, &world(10_400)).is_empty());
    let out = eng.handle(EngineInput::Tick, &world(10_500));
    assert_eq!(out[0].rule, "пауза");
    assert_eq!(out[0].trigger_kind, "http_trigger");
    assert_eq!(out[0].effects, vec![Effect::Log { text: "после".into() }]);
    assert_eq!(eng.next_deadline_ms(), None);
}

#[test]
fn replacing_the_document_cancels_pending_continuations() {
    let src = r#"rule "п" { when http trigger do wait 1s log("хвост") }"#;
    let mut eng = engine(src, 0);
    eng.handle(run("п"), &world(0));
    assert_eq!(eng.counters().continuations_scheduled, 1);
    eng.set_rules(Some(compile(src)), 100);
    assert_eq!(eng.counters().continuations_dropped, 1);
    assert!(eng.handle(EngineInput::Tick, &world(1_000)).is_empty());
    assert_eq!(eng.counters().continuations_fired, 0);
}

#[test]
fn disabling_a_rule_cancels_its_continuations() {
    let src = r#"
rule "жертва" { when http trigger do wait 1s log("хвост") }
rule "нож" { when http trigger do rule("жертва").disable() }
"#;
    let mut eng = engine(src, 0);
    eng.handle(run("жертва"), &world(0));
    eng.handle(run("нож"), &world(100));
    assert_eq!(eng.counters().continuations_dropped, 1);
    assert!(eng.handle(EngineInput::Tick, &world(1_000)).is_empty());
}

#[test]
fn repeat_multiplies_an_inlined_def_block_in_order() {
    let src = r#"
def "пара" { log("один") log("два") }
rule "восемь" { when http trigger do repeat 4 { call("пара") } }
"#;
    let mut eng = engine(src, 0);
    let out = eng.handle(run("восемь"), &world(0));
    let texts: Vec<&str> = out[0]
        .effects
        .iter()
        .map(|e| match e {
            Effect::Log { text } => text.as_str(),
            other => panic!("expected log, got {other:?}"),
        })
        .collect();
    assert_eq!(texts, ["один", "два"].repeat(4));
    assert_eq!(out[0].partial, None);
}

#[test]
fn a_forty_action_expansion_is_the_compilers_to_refuse() {
    let src = r#"
def "пять" { log("а") log("б") log("в") log("г") log("д") }
rule "сорок" { when http trigger do repeat 8 { call("пять") } }
"#;
    let err = RulesLangV1.compile(src, &resolver()).unwrap_err();
    assert!(
        err.message.contains("expands to 40"),
        "names the expansion: {err}"
    );
}

fn log_only_rule(name: &str, count: usize) -> RuleSet {
    let actions = (0..count)
        .map(|i| {
            Action::State(StateAction::Log {
                text: format!("шаг{i:02}"),
            })
        })
        .collect();
    RuleSet {
        lang_id: 1,
        blocks: Vec::new(),
        rules: vec![Rule {
            name: name.into(),
            enabled: true,
            cooldown_ms: 0,
            hold_hcl: true,
            triggers: vec![Trigger::HttpTrigger],
            conditions: Vec::new(),
            actions,
        }],
    }
}

#[test]
fn the_engine_defends_the_budget_the_compiler_normally_guarantees() {
    let mut eng = Engine::new(0);
    eng.set_rules(Some(log_only_rule("сорок", 40)), 0);
    let out = eng.handle(run("сорок"), &world(0));
    assert_eq!(out[0].effects.len(), 32, "first 32 execute");
    assert_eq!(out[0].partial, Some(PartialReason::EffectBudget));

    let mut eng = Engine::new(0);
    eng.set_rules(Some(log_only_rule("ровно", 32)), 0);
    let out = eng.handle(run("ровно"), &world(0));
    assert_eq!(out[0].effects.len(), 32);
    assert_eq!(out[0].partial, None, "exactly 32 is within budget");
}

#[test]
fn every_fires_on_its_period_and_collapses_missed_periods() {
    let src = r#"rule "тик" { when every 1s do log("т") }"#;
    let mut eng = engine(src, 0);
    assert_eq!(eng.next_deadline_ms(), Some(1_000));
    assert!(eng.handle(EngineInput::Tick, &world(900)).is_empty());
    let out = eng.handle(EngineInput::Tick, &world(1_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].trigger_kind, "every");
    let out = eng.handle(EngineInput::Tick, &world(5_000));
    assert_eq!(out.len(), 1);
    assert_eq!(eng.next_deadline_ms(), Some(6_000));
}

#[test]
fn at_time_fires_on_crossing_with_the_day_filter_across_midnight() {
    let src = r#"rule "будильник" { when at 00:05 on tue do log("в") }"#;
    let mut eng = engine(src, 0);
    let monday_night = night_world(10_000, 1_438, 0);
    assert!(eng.handle(EngineInput::Tick, &monday_night).is_empty());
    let tuesday_morning = night_world(490_000, 6, 1);
    let out = eng.handle(EngineInput::Tick, &tuesday_morning);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].trigger_kind, "at_time");

    let wed_prime = night_world(500_000, 1_438, 1);
    assert!(eng.handle(EngineInput::Tick, &wed_prime).is_empty());
    let wed_morning = night_world(990_000, 6, 2);
    assert!(eng.handle(EngineInput::Tick, &wed_morning).is_empty());
}

#[test]
fn a_schedule_enabled_late_judges_from_now_not_from_history() {
    let src = r#"
rule "утро" enabled false { when at 07:30 do log("у") }
rule "вкл" { when http trigger do rule("утро").enable() }
"#;
    let mut eng = engine(src, 0);
    assert!(eng.handle(EngineInput::Tick, &night_world(10_000, 445, 0)).is_empty());
    assert!(eng.handle(EngineInput::Tick, &night_world(600_000, 455, 0)).is_empty());
    eng.handle(run("вкл"), &world(700_000));
    assert!(eng.handle(EngineInput::Tick, &night_world(710_000, 456, 0)).is_empty());
    assert!(eng.handle(EngineInput::Tick, &night_world(86_000_000, 449, 1)).is_empty());
    let out = eng.handle(EngineInput::Tick, &night_world(86_120_000, 451, 1));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rule, "утро");
}

#[test]
fn an_unsynced_wall_clock_fires_nothing_and_is_counted() {
    let src = r#"rule "утро" { when at 07:30 do log("у") }"#;
    let mut eng = engine(src, 0);
    assert!(eng.handle(EngineInput::Tick, &world(1_000)).is_empty());
    assert!(eng.handle(EngineInput::Tick, &world(2_000)).is_empty());
    assert_eq!(eng.counters().ticks_time_unsynced, 2);
    assert_eq!(eng.next_deadline_ms(), Some(2_000 + 1_000));
}

#[test]
fn sunrise_offset_fires_from_the_solar_table() {
    let src = r#"rule "рассвет" { when at sunrise-15m do log("р") }"#;
    let mut eng = engine(src, 0);
    assert!(eng.handle(EngineInput::Tick, &night_world(10_000, 313, 0)).is_empty());
    let out = eng.handle(EngineInput::Tick, &night_world(190_000, 316, 0));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].trigger_kind, "at_solar");
}

#[test]
fn a_position_slider_carries_event_value_into_a_level() {
    let src = r#"
rule "слайдер яркости" {
  when input(dev=7, inst=0).position changes
  do   group("кухня").level(event.value)
}
"#;
    let kitchen = resolver().resolve_group("кухня").unwrap();
    let mut eng = engine(src, 0);
    let slide = |value: u16| EngineInput::InputEvent {
        adapter_id: 0,
        short_address: Some(7),
        instance_number: Some(0),
        instance_type: Some(2),
        instance_groups: [None; 3],
        kind: InputEventKind::PositionChanged,
        value,
    };
    let out = eng.handle(slide(180), &world(1_000));
    assert_eq!(
        out[0].effects,
        vec![Effect::Light {
            target: LightTarget::Group(kitchen),
            verb: LightVerb::Level { level: 180 },
            hold_hcl: true,
        }]
    );
    let out = eng.handle(slide(999), &world(1_100));
    assert!(matches!(
        out[0].effects[0],
        Effect::Light {
            verb: LightVerb::Level { level: 254 },
            ..
        }
    ));
}

#[test]
fn a_dry_run_previews_the_whole_walk_and_executes_nothing() {
    let src = r#"
rule "сухо" {
  when http trigger
  do   var("м").set(1)
       timer("т").start(10s)
       lamp("х").on()
       wait 1s
       log("после")
       after 2s do { log("отложено") }
}
"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    let out = eng.handle(EngineInput::RunRule { name: "сухо", dry: true }, &world(0));
    assert_eq!(out.len(), 1);
    assert!(out[0].dry);
    assert_eq!(
        out[0].effects,
        vec![
            Effect::Light {
                target: LightTarget::Lamp(lamp),
                verb: LightVerb::On { level: None },
                hold_hcl: true,
            },
            Effect::Log { text: "после".into() },
            Effect::Log { text: "отложено".into() },
        ]
    );
    assert_eq!(eng.next_deadline_ms(), None);
    let c = eng.counters();
    assert_eq!(
        (c.activations_dry, c.activations_total, c.timers_active, c.vars_in_use),
        (1, 0, 0, 0)
    );
}

#[test]
fn a_wet_manual_run_requires_the_rule_enabled_and_a_dry_one_does_not() {
    let src = r#"rule "выкл" enabled false { when http trigger do log("х") }"#;
    let mut eng = engine(src, 0);
    assert!(eng.handle(run("выкл"), &world(0)).is_empty());
    assert_eq!(eng.counters().suppressed_disabled, 1);
    let out = eng.handle(EngineInput::RunRule { name: "выкл", dry: true }, &world(0));
    assert_eq!(out.len(), 1);
    assert!(out[0].dry);
}

fn determinism_script(eng: &mut Engine) -> Vec<Vec<dali2rust_rules_runtime::ActivationOutcome>> {
    let lamp = resolver().resolve_lamp("х").unwrap();
    vec![
        eng.handle(button(1, 0, InputEventKind::Press), &world(1_000)),
        eng.handle(lamp_on_edge(lamp, true), &with_lamp(world(2_000), lamp, true, 200)),
        eng.handle(EngineInput::Tick, &world(3_000)),
        eng.handle(run("б: свет"), &world(4_000)),
    ]
}

#[test]
fn the_same_inputs_from_fresh_engines_produce_identical_outcomes() {
    let src = r#"
rule "в: кнопка" { when input(dev=1, inst=0) is press do log("к") var("н").add(1) }
rule "б: свет" { when lamp("х") turns on when http trigger do log("с") }
rule "а: свет" { when lamp("х") turns on do log("а") }
"#;
    let mut one = engine(src, 0);
    let mut two = engine(src, 0);
    assert_eq!(determinism_script(&mut one), determinism_script(&mut two));
    assert_eq!(one.counters(), two.counters());
}

#[test]
fn rules_sharing_a_trigger_activate_in_ascending_name_order() {
    let src = r#"
rule "б" { when lamp("х") turns on do log("б") }
rule "а" { when lamp("х") turns on do log("а") }
rule "в" { when lamp("х") turns on do log("в") }
"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    let out = eng.handle(lamp_on_edge(lamp, true), &world(1_000));
    let names: Vec<&str> = out.iter().map(|o| o.rule.as_str()).collect();
    assert_eq!(names, ["а", "б", "в"], "document order was б, а, в");
}

#[test]
fn a_light_feedback_loop_is_cut_at_the_chain_depth() {
    let src = r#"rule "петля" { when lamp("х") turns on do lamp("х").level(200) }"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    for i in 0..5u64 {
        let at = 1_000 + i * 250;
        let out = eng.handle(lamp_on_edge(lamp, true), &with_lamp(world(at), lamp, true, 100));
        assert_eq!(out.len(), 1, "depth {i} still activates");
        assert_eq!(out[0].partial, None);
    }
    let out = eng.handle(lamp_on_edge(lamp, true), &with_lamp(world(2_250), lamp, true, 100));
    assert_eq!(out.len(), 1);
    assert!(out[0].effects.is_empty());
    assert_eq!(out[0].partial, Some(PartialReason::ChainDepth));
    assert_eq!(eng.counters().chain_depth_exceeded, 1);
    assert_eq!(eng.counters().activations_total, 5);
}

#[test]
fn the_chain_window_expiry_resets_the_depth() {
    let src = r#"rule "петля" { when lamp("х") turns on do lamp("х").level(200) }"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    for i in 0..8u64 {
        let at = 1_000 + i * 2_500;
        let out = eng.handle(lamp_on_edge(lamp, true), &with_lamp(world(at), lamp, true, 100));
        assert_eq!(out[0].partial, None, "iteration {i} starts a fresh chain");
    }
    assert_eq!(eng.counters().chain_depth_exceeded, 0);
}

#[test]
fn timer_mediated_cycles_are_not_chains() {
    let src = r#"
rule "мигалка" { when timer("б") fires do lamp("х").toggle() timer("б").restart(1s) }
rule "старт" { when http trigger do timer("б").start(1s) }
"#;
    let lamp = resolver().resolve_lamp("х").unwrap();
    let mut eng = engine(src, 0);
    eng.handle(run("старт"), &world(0));
    for i in 1..=8u64 {
        let at = i * 1_000;
        let out = eng.handle(EngineInput::Tick, &with_lamp(world(at), lamp, false, 0));
        assert_eq!(out.len(), 1, "blink {i} fires");
        assert_eq!(out[0].partial, None);
        assert_eq!(eng.next_deadline_ms(), Some(at + 1_000));
    }
    assert_eq!(eng.counters().chain_depth_exceeded, 0);
}

#[test]
fn timer_start_keeps_a_running_deadline_and_restart_rearms_it() {
    let src = r#"
rule "старт" { when http trigger do timer("х").start(5s) }
rule "заново" { when http trigger do timer("х").restart(5s) }
"#;
    let mut eng = engine(src, 0);
    eng.handle(run("старт"), &world(0));
    assert_eq!(eng.next_deadline_ms(), Some(5_000));
    eng.handle(run("старт"), &world(1_000));
    assert_eq!(eng.next_deadline_ms(), Some(5_000), ".start on a running timer is a no-op");
    eng.handle(run("заново"), &world(2_000));
    assert_eq!(eng.next_deadline_ms(), Some(7_000), ".restart always re-arms");
}

#[test]
fn scene_cycle_advances_per_wet_run_and_a_dry_run_peeks_without_moving() {
    let src = r#"rule "цикл" { when http trigger do scene.cycle(1, 3, 7) }"#;
    let mut eng = engine(src, 0);
    let scene_of = |out: &[dali2rust_rules_runtime::ActivationOutcome]| match out[0].effects[0] {
        Effect::SceneRecall { scene, .. } => scene,
        ref other => panic!("expected a recall, got {other:?}"),
    };
    assert_eq!(scene_of(&eng.handle(run("цикл"), &world(0))), 1);
    assert_eq!(scene_of(&eng.handle(run("цикл"), &world(1))), 3);
    let dry = eng.handle(EngineInput::RunRule { name: "цикл", dry: true }, &world(2));
    assert_eq!(scene_of(&dry), 7, "a dry run previews the next scene");
    assert_eq!(scene_of(&eng.handle(run("цикл"), &world(3))), 7, "…without advancing");
    assert_eq!(scene_of(&eng.handle(run("цикл"), &world(4))), 1, "and the list wraps");
}

#[test]
fn var_add_counts_from_zero_and_gates_a_condition() {
    let src = r#"
rule "плюс" { when http trigger do var("н").add(1) }
rule "порог" { when http trigger if var("н") >= 2 do log("ok") }
"#;
    let mut eng = engine(src, 0);
    eng.handle(run("плюс"), &world(0));
    assert!(eng.handle(run("порог"), &world(1)).is_empty());
    assert_eq!(eng.counters().conditions_rejected, 1);
    eng.handle(run("плюс"), &world(2));
    let out = eng.handle(run("порог"), &world(3));
    assert_eq!(out[0].effects, vec![Effect::Log { text: "ok".into() }]);
}

#[test]
fn an_unevaluable_condition_is_a_partial_outcome_not_a_false() {
    let src = r#"rule "слеп" { when http trigger if lamp("нет") is on do log("х") }"#;
    let mut eng = engine(src, 0);
    let out = eng.handle(run("слеп"), &world(0));
    assert_eq!(out.len(), 1);
    assert!(out[0].effects.is_empty());
    assert_eq!(out[0].partial, Some(PartialReason::ConditionUnevaluable));
    assert_eq!(eng.counters().partial_outcomes, 1);
    assert_eq!(eng.counters().conditions_rejected, 0);
}

#[test]
fn an_unknown_level_fails_every_comparison_on_it_including_not_equal() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let src = r#"
rule "ниже" { when http trigger if lamp("коридор").level < 50 do log("<") }
rule "мимо" { when http trigger if lamp("коридор").level != 50 do log("!=") }
"#;
    let mut eng = engine(src, 0);
    let unknown = with_unknown_level(world(0), lamp);
    assert!(eng.handle(run("ниже"), &unknown).is_empty());
    assert!(eng.handle(run("мимо"), &unknown).is_empty());
    assert_eq!(eng.counters().conditions_rejected, 2);
    assert_eq!(eng.counters().partial_outcomes, 0, "a false condition is not a partial outcome");
    let known = with_lamp(world(1), lamp, true, 20);
    assert_eq!(eng.handle(run("ниже"), &known).len(), 1, "a known level passes the same gate");
}

#[test]
fn an_unknown_level_read_as_a_value_is_unevaluable() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let kitchen = resolver().resolve_lamp("кухня").unwrap();
    let src = r#"
rule "эхо" { when http trigger do lamp("кухня").level(lamp("коридор").level) }
rule "память" { when http trigger do lamp("кухня").level(lamp("коридор").last_level) }
rule "справа" { when http trigger if lamp("кухня").level > lamp("коридор").level do log(">") }
"#;
    let mut eng = engine(src, 0);
    let unknown = with_lamp(with_unknown_level(world(0), lamp), kitchen, true, 100);
    for name in ["эхо", "память", "справа"] {
        let out = eng.handle(run(name), &unknown);
        assert_eq!(out.len(), 1, "{name}");
        assert!(out[0].effects.is_empty(), "{name}: {out:?}");
        assert!(out[0].partial.is_some(), "{name}: {out:?}");
    }
    assert_eq!(eng.counters().partial_outcomes, 3);
    assert_eq!(eng.counters().conditions_rejected, 0, "unevaluable is not false");
}

#[test]
fn a_level_crossing_needs_two_known_levels() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let src = r#"rule "порог" { when lamp("коридор").level crosses above 100 do log("↑") }"#;
    let mut eng = engine(src, 0);
    assert!(eng.handle(lamp_level_edge(lamp, true, None, Some(150)), &world(1_000)).is_empty());
    assert!(eng.handle(lamp_level_edge(lamp, true, Some(50), None), &world(2_000)).is_empty());
    let known = lamp_level_edge(lamp, true, Some(50), Some(150));
    assert_eq!(eng.handle(known, &world(3_000)).len(), 1, "two known levels cross");
}

#[test]
fn event_value_of_a_lamp_with_an_unknown_level_is_unevaluable() {
    let lamp = resolver().resolve_lamp("коридор").unwrap();
    let src = r#"rule "эхо" { when lamp("коридор") turns on do lamp("кухня").level(event.value) }"#;
    let mut eng = engine(src, 0);
    let out = eng.handle(lamp_level_edge(lamp, false, None, None), &world(1_000));
    assert_eq!(out.len(), 1);
    assert!(out[0].effects.is_empty(), "{out:?}");
    assert!(out[0].partial.is_some(), "{out:?}");
    assert_eq!(eng.counters().partial_outcomes, 1);
}

#[test]
fn hcl_override_edges_match_their_scoped_target() {
    let src = r#"rule "щит" { when hcl override starts for group("кухня") do log("щ") }"#;
    let kitchen = resolver().resolve_group("кухня").unwrap();
    let mut eng = engine(src, 0);
    let started = EngineInput::HclOverride {
        started: true,
        target: LightTarget::Group(kitchen),
    };
    assert_eq!(eng.handle(started, &world(1_000)).len(), 1);
    let cleared = EngineInput::HclOverride {
        started: false,
        target: LightTarget::Group(kitchen),
    };
    assert!(eng.handle(cleared, &world(2_000)).is_empty());
}

#[test]
fn light_crossings_are_judged_by_the_rule_threshold_over_the_edge_cache() {
    let src = r#"rule "темнеет" { when input(dev=6, inst=0).light crosses below 150 do log("т") }"#;
    let mut eng = engine(src, 0);
    let sample = |value: u16| EngineInput::InputEvent {
        adapter_id: 0,
        short_address: Some(6),
        instance_number: Some(0),
        instance_type: Some(4),
        instance_groups: [None; 3],
        kind: InputEventKind::LightCrossedBelow,
        value,
    };
    assert!(eng.handle(sample(200), &world(1_000)).is_empty());
    assert_eq!(eng.handle(sample(140), &world(2_000)).len(), 1);
    assert!(eng.handle(sample(1_023), &world(3_000)).is_empty());
    assert!(eng.handle(sample(100), &world(4_000)).is_empty());
}

#[test]
fn group_becomes_matches_transitions_only() {
    let src = r#"rule "г" { when group("кухня") becomes any_on do log("г") }"#;
    let kitchen = resolver().resolve_group("кухня").unwrap();
    let mut eng = engine(src, 0);
    let edge = |was: bool, now: bool| EngineInput::GroupChanged {
        adapter_id: kitchen.adapter_id,
        group_id: kitchen.id,
        any_on: now,
        was_any_on: was,
    };
    assert_eq!(eng.handle(edge(false, true), &world(1_000)).len(), 1);
    assert!(eng.handle(edge(true, true), &world(2_000)).is_empty());
    assert!(eng.handle(edge(true, false), &world(3_000)).is_empty());
}

#[test]
fn an_idle_engine_parks_and_an_empty_document_matches_nothing() {
    let src = r#"rule "ручной" { when http trigger do log("х") }"#;
    let mut eng = engine(src, 0);
    assert_eq!(eng.next_deadline_ms(), None, "nothing scheduled → park");
    eng.set_rules(None, 100);
    assert!(eng.handle(button(1, 0, InputEventKind::Press), &world(200)).is_empty());
    assert_eq!(eng.counters().rules_loaded, 0);
}

const MQTT_SCENES: &str = "rule \"a-вечер\" {\n  when mqtt \"home/scene\" is \"evening\"\n  do broadcast.off()\n}\nrule \"b-любой\" {\n  when mqtt \"home/scene\"\n  do log(\"scene\")\n}\n";

fn mqtt<'a>(topic: &'a str, payload: &'a [u8], truncated: bool) -> EngineInput<'a> {
    EngineInput::MqttMessage { topic, payload, truncated }
}

fn fired(out: &[dali2rust_rules_runtime::ActivationOutcome]) -> Vec<&str> {
    out.iter().map(|o| o.rule.as_str()).collect()
}

#[test]
fn a_broker_message_fires_the_rules_on_its_topic_by_payload() {
    let mut eng = engine(MQTT_SCENES, 0);
    let out = eng.handle(mqtt("home/scene", b"evening", false), &world(1_000));
    assert_eq!(fired(&out), ["a-вечер", "b-любой"]);
    assert_eq!(out[0].trigger_kind, "mqtt_message");
    assert!(matches!(out[0].effects[0], Effect::Light { verb: LightVerb::Off, .. }));

    let out = eng.handle(mqtt("home/scene", b"morning", false), &world(2_000));
    assert_eq!(fired(&out), ["b-любой"], "another payload wakes only the payload-less trigger");

    let out = eng.handle(mqtt("home/scenery", b"evening", false), &world(3_000));
    assert!(out.is_empty(), "a topic is one exact name, not a prefix: {out:?}");
}

#[test]
fn a_truncated_payload_never_equals_a_literal_it_starts_with() {
    let literal = "a".repeat(dali2rust_rules_model::limits::MAX_MQTT_PAYLOAD_BYTES);
    let source = format!(
        "rule \"a-точно\" {{\n  when mqtt \"home/scene\" is \"{literal}\"\n  do broadcast.off()\n}}\nrule \"b-любой\" {{\n  when mqtt \"home/scene\"\n  do log(\"scene\")\n}}\n"
    );
    let mut eng = engine(&source, 0);
    let out = eng.handle(mqtt("home/scene", literal.as_bytes(), true), &world(1_000));
    assert_eq!(fired(&out), ["b-любой"]);
    let out = eng.handle(mqtt("home/scene", literal.as_bytes(), false), &world(2_000));
    assert_eq!(fired(&out), ["a-точно", "b-любой"]);
}

#[test]
fn a_numeric_payload_is_the_event_value_and_anything_else_is_unevaluable() {
    let source = "rule \"уровень\" {\n  when mqtt \"home/level\"\n  do broadcast.level(event.value)\n}\n";
    let mut eng = engine(source, 0);
    let out = eng.handle(mqtt("home/level", b"128", false), &world(1_000));
    assert!(
        matches!(out[0].effects[..], [Effect::Light { verb: LightVerb::Level { level: 128 }, .. }]),
        "{out:?}"
    );
    assert_eq!(out[0].partial, None);
    for (at, payload) in [(2_000, &b"evening"[..]), (3_000, b"12.5"), (4_000, b" 7"), (5_000, b"2147483648")] {
        let out = eng.handle(mqtt("home/level", payload, false), &world(at));
        assert_eq!(
            out[0].partial,
            Some(PartialReason::ConditionUnevaluable),
            "{:?}: {out:?}",
            String::from_utf8_lossy(payload)
        );
        assert!(out[0].effects.is_empty(), "{out:?}");
    }
    let out = eng.handle(mqtt("home/level", b"128", true), &world(6_000));
    assert_eq!(out[0].partial, Some(PartialReason::ConditionUnevaluable), "a cut number is no number");
}

#[test]
fn the_rule_payload_ceiling_is_the_bus_frame() {
    assert_eq!(
        dali2rust_rules_model::limits::MAX_MQTT_PAYLOAD_BYTES,
        dali2rust_contracts::msg::MQTT_RULE_PAYLOAD_BYTES,
        "a literal longer than the frame could never match, a shorter one would be cut short"
    );
    assert!(
        dali2rust_rules_model::limits::MAX_MQTT_TOPIC_BYTES
            <= dali2rust_contracts::msg::FixedText48::new().capacity(),
        "every trigger topic fits the event's topic field"
    );
}

fn chain_refused(out: &[dali2rust_rules_runtime::ActivationOutcome]) -> bool {
    out.iter().any(|o| o.partial == Some(PartialReason::ChainDepth))
}

#[test]
fn a_rule_that_publishes_to_its_own_topic_stops_at_the_chain_depth() {
    let source = "rule \"эхо\" cooldown 0ms {\n  when mqtt \"home/echo\"\n  do mqtt.publish(\"home/echo\", \"x\")\n}\n";
    let mut eng = engine(source, 0);
    for step in 0..=u64::from(dali2rust_rules_runtime::runtime::engine::MAX_CHAIN_DEPTH) {
        let out = eng.handle(mqtt("home/echo", b"x", false), &world(1_000 + step * 100));
        assert!(!chain_refused(&out), "turn {step} of the loop still fires: {out:?}");
    }
    let out = eng.handle(mqtt("home/echo", b"x", false), &world(1_600));
    assert!(chain_refused(&out), "the loop ends at the chain depth: {out:?}");
    let quiet = 1_600 + dali2rust_rules_runtime::runtime::engine::CHAIN_WINDOW_MS + 1;
    let out = eng.handle(mqtt("home/echo", b"x", false), &world(quiet));
    assert!(!chain_refused(&out), "a message after the window starts a fresh chain: {out:?}");
}

#[test]
fn a_light_rule_on_an_external_topic_never_counts_its_own_messages_as_a_chain() {
    let source = "rule \"пульт\" cooldown 0ms {\n  when mqtt \"remote/dim\"\n  do broadcast.level(event.value)\n}\n";
    let mut eng = engine(source, 0);
    for step in 0..20u64 {
        let out = eng.handle(mqtt("remote/dim", b"100", false), &world(1_000 + step * 400));
        assert_eq!(out.len(), 1, "message {step} fires: {out:?}");
        assert_eq!(out[0].partial, None, "message {step} is no link of a chain: {out:?}");
    }
}

#[test]
fn a_publish_chains_only_the_messages_on_its_own_topic() {
    let source = "rule \"эхо\" cooldown 0ms {\n  when mqtt \"home/echo\"\n  do mqtt.publish(\"home/echo\", \"x\")\n}\n\
                  rule \"пульт\" cooldown 0ms {\n  when mqtt \"remote/dim\"\n  do broadcast.level(event.value)\n}\n";
    let mut eng = engine(source, 0);
    for step in 0..=u64::from(dali2rust_rules_runtime::runtime::engine::MAX_CHAIN_DEPTH) {
        let _ = eng.handle(mqtt("home/echo", b"x", false), &world(1_000 + step * 100));
    }
    let out = eng.handle(mqtt("remote/dim", b"100", false), &world(1_550));
    assert_eq!(out[0].partial, None, "the echo's depth is not the remote's: {out:?}");
    let out = eng.handle(mqtt("home/echo", b"x", false), &world(1_600));
    assert!(chain_refused(&out), "while the echo itself is at its limit: {out:?}");
}

fn stat_counts(eng: &Engine) -> Vec<(String, u32)> {
    eng.stat_counts().map(|(name, count)| (name.to_owned(), count)).collect()
}

#[test]
fn named_counters_count_wet_runs_only_and_a_reload_keeps_the_names_it_keeps() {
    let mut eng = engine(
        "rule \"a\" {\n  when http trigger\n  do stat(\"x\").count()\n     stat(\"y\").count()\n}\n",
        0,
    );
    assert_eq!(stat_counts(&eng), [("x".into(), 0), ("y".into(), 0)], "a name the document brings starts at 0");
    eng.handle(run("a"), &world(10));
    eng.handle(EngineInput::RunRule { name: "a", dry: true }, &world(20));
    assert_eq!(stat_counts(&eng), [("x".into(), 1), ("y".into(), 1)], "a dry run counts nothing");

    eng.set_rules(
        Some(compile("rule \"b\" {\n  when http trigger\n  do stat(\"x\").count()\n     stat(\"z\").count()\n}\n")),
        30,
    );
    assert_eq!(
        stat_counts(&eng),
        [("x".into(), 1), ("z".into(), 0)],
        "x survives with its count, y is gone, z starts at 0"
    );
}

#[test]
fn a_pending_continuation_is_a_gauge_that_falls_when_it_fires() {
    let mut eng = engine(
        "rule \"later\" {\n  when http trigger\n  do after 1s do { log(\"late\") }\n}\n",
        0,
    );
    assert_eq!(eng.counters().continuations_pending, 0);
    eng.handle(run("later"), &world(0));
    assert_eq!(eng.counters().continuations_pending, 1, "the after block waits on the wheel");

    let fired = eng.handle(EngineInput::Tick, &world(1_000));
    assert_eq!(eng.counters().continuations_pending, 0, "a fired continuation is no longer pending");
    assert!(
        fired.iter().any(|outcome| outcome.effects.iter().any(|e| matches!(e, Effect::Log { .. }))),
        "the continuation ran its body: {fired:?}"
    );
    assert_eq!(eng.counters().continuations_fired, 1);
}
