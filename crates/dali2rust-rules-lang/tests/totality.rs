mod support;

use dali2rust_rules_lang::{
    PARSED_ACTION_KINDS, PARSED_CONDITION_KINDS, PARSED_TRIGGER_KINDS, PARSED_VALUE_KINDS,
};
use dali2rust_rules_model::coverage::KindCoverage;
use dali2rust_rules_model::{ActionKind, ConditionKind, TriggerKind, ValueKind};
use std::collections::BTreeSet;
use support::compile_ok;

const KITCHEN_SINK: &str = r#"
# kitchen sink: the whole v1 vocabulary in one document

def "night" {
  scene(13).recall(group("ночь"))
  timer("t1").restart(3m)
}

rule "buttons" {
  when input(dev=3, inst=0) is short_press
  when input(group=4, type=button) is press
  when input(dev=3, inst=1) is event(data=5)
  when input(dev=3, inst=2) is any_event
  if time in 07:00 .. 23:00 and day in mon..fri and sun is up and lamp("шкаф") is on
  do lamp("коридор").on(level=200)
     lamp("коридор").off()
     lamp("коридор").toggle(hold_hcl=true)
     lamp("коридор").level(event.value)
     group("кухня").dim(-8)
     lamp("коридор").dim_hold(+60)
     lamp("коридор").cct(2700)
     broadcast.xy(0.313, 0.329)
}

rule "colours" {
  when input(dev=5, inst=0) becomes occupied
  if lamp("шкаф").level >= var("min") + 10 and lamp("шкаф").cct <= 6500 and group("кухня") any_on and input(dev=5, inst=0) is occupied
  do lamp("коридор").rgb(255, 180, 40)
     lamp("коридор").last_active()
     lamp("коридор").stop_fade()
     scene(1).recall(lamp("коридор"))
     scene(2).apply()
     scene.cycle(1, 3, 7)
}

rule "hcl and feedback" {
  when input(dev=6, inst=0).light crosses above 400
  if input(dev=6, inst=0).light above 300 and hcl is enabled for group("кухня") and device(3) is online and rule("buttons") is enabled
  do hcl.resume(group("кухня"))
     hcl.hold(broadcast)
     hcl.enable("дневное")
     hcl.disable("дневное")
     input(dev=3, inst=0).feedback.on()
     input(dev=3, inst=0).feedback.off()
     input(dev=5, inst=0).cancel_hold()
     input(dev=5, inst=0).catch_movement()
}

rule "flow" {
  when input(dev=7, inst=0).position changes
  if last_fired("buttons") older than 10m and timer("t1") is running and controller is active and var("режим") == "ночь"
  do wait 1s
     after 5m do { log("later") }
     timer("t1").start(30s)
     timer("t2").restart(2h)
     timer("t2").cancel()
     call("night")
     panel_select(group=4, selected=event.option)
}

rule "structure" {
  when input device(dev=3) power cycled
  when input device(dev=3) manual config changed
  do repeat 2 { log("twice") }
     if sun is down { log("dark") } else { log("bright") }
     var("режим").set("ночь")
     var("счётчик").add(1)
     rule("buttons").enable()
     rule("buttons").disable()
}

rule "clock" {
  when at 07:30 on mon,tue,sat
  when at sunrise-15m
  do group("кухня").level(lamp("шкаф").level)
     lamp("коридор").cct(lamp("шкаф").cct - 100)
     mqtt.publish("dali2rust/mode", "away", retain=true)
     log("morning")
     stat("wakeups").count()
}

rule "periodic" {
  when every 15m
  if var("b") == lamp("шкаф").is_on and var("g") == group("кухня").any_on
  do lamp("коридор").level(lamp("шкаф").last_level)
}

rule "lamp state" {
  when lamp("коридор") turns on
  when lamp("коридор").level crosses above 100
  if var("q") == group("кухня").all_off and var("n") >= group("кухня").member_count
  do lamp("шкаф").level(+8)
}

rule "aggregates" {
  when group("кухня") becomes all_off
  when scene(3) recalled
  if var("o") == input(dev=5, inst=0).occupied and lamp("шкаф").level >= input(dev=6, inst=0).light
  do lamp("шкаф").level(-8)
}

rule "presence" {
  when device(3) goes offline
  when device(4) comes online
  when hcl override starts for group("кухня")
  when hcl override clears for lamp("коридор")
  if var("age") <= input(dev=5, inst=0).last_event_age
  do lamp("шкаф").level(input(dev=7, inst=0).position)
}

rule "clockwork" {
  when timer("t1") fires
  if var("t") == time.now and var("h") >= time.hour and var("m") <= time.minute and var("w") == time.weekday
  do lamp("шкаф").on()
}

rule "boot" {
  when controller starts
  when controller becomes active
  when at sunset+30m
  if var("r") == sun.rise and var("s") == sun.set and var("u") == sun.is_up
  do panel_select(group=4, selected=1)
}

rule "watchdog" {
  when rule("buttons") fails
  when http trigger
  if var("d") == event.device and var("i") == event.instance
  do lamp("коридор").on(level=10)
}
"#;

fn names<T: Copy>(kinds: &[T], name: fn(T) -> &'static str) -> BTreeSet<&'static str> {
    kinds.iter().map(|k| name(*k)).collect()
}

#[test]
fn parser_coverage_consts_equal_the_registries() {
    assert_eq!(
        names(PARSED_TRIGGER_KINDS, TriggerKind::name),
        names(&TriggerKind::ALL, TriggerKind::name),
        "trigger kinds without a parser arm (or stale coverage const)"
    );
    assert_eq!(
        names(PARSED_CONDITION_KINDS, ConditionKind::name),
        names(&ConditionKind::ALL, ConditionKind::name),
        "condition kinds without a parser arm (or stale coverage const)"
    );
    assert_eq!(
        names(PARSED_ACTION_KINDS, ActionKind::name),
        names(&ActionKind::ALL, ActionKind::name),
        "action kinds without a parser arm (or stale coverage const)"
    );
    assert_eq!(
        names(PARSED_VALUE_KINDS, ValueKind::name),
        names(&ValueKind::ALL, ValueKind::name),
        "value kinds without a parser arm (or stale coverage const)"
    );
}

#[test]
fn the_kitchen_sink_exercises_every_kind() {
    let set = compile_ok(KITCHEN_SINK);
    let mut coverage = KindCoverage::default();
    coverage.add(&set);
    assert_eq!(
        coverage.triggers,
        names(&TriggerKind::ALL, TriggerKind::name),
        "trigger kinds the kitchen sink never produced"
    );
    assert_eq!(
        coverage.conditions,
        names(&ConditionKind::ALL, ConditionKind::name),
        "condition kinds the kitchen sink never produced"
    );
    assert_eq!(
        coverage.actions,
        names(&ActionKind::ALL, ActionKind::name),
        "action kinds the kitchen sink never produced"
    );
    assert_eq!(
        coverage.values,
        names(&ValueKind::ALL, ValueKind::name),
        "value kinds the kitchen sink never produced"
    );
}
