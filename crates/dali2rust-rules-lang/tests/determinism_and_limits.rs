mod support;

use dali2rust_rules_lang::{RulesLangV1, LANG_RULES_V1};
use dali2rust_rules_model::{RuleCompiler, TriggerKind};
use support::{compile_err, compile_ok};

const DOCUMENT: &str = r#"
# determinism fixture — names, numbers, structure
def "контур" {
  scene(13).recall(group("ночь"))
  timer("t").restart(3m)
}
rule "правило" enabled false cooldown 500ms hold_hcl true {
  when input(dev=3, inst=0) is short_press
  when at 23:00 on fri..mon
  if var("режим") != "ночь" and time in sunset-30m .. 06:00
  do call("контур")
     lamp("коридор").level(event.value + 10)
}
"#;

#[test]
fn parsing_the_same_source_twice_yields_equal_models() {
    let first = compile_ok(DOCUMENT);
    let second = compile_ok(DOCUMENT);
    assert_eq!(first, second);
    assert_eq!(first.lang_id, LANG_RULES_V1);
}

#[test]
fn written_modifiers_override_the_class_defaults() {
    let set = compile_ok(DOCUMENT);
    let rule = set.rule("правило").unwrap();
    assert!(!rule.enabled);
    assert_eq!(rule.cooldown_ms, 500);
    assert!(rule.hold_hcl);
    assert_eq!(rule.triggers[0].kind(), TriggerKind::InputEvent);
    assert_eq!(rule.triggers[1].kind(), TriggerKind::AtTime);
}

#[test]
fn a_repeat_of_a_five_action_def_breaks_the_expansion_ceiling_from_source() {
    let source = r#"
def "five" {
  log("1")
  log("2")
  log("3")
  log("4")
  log("5")
}
rule "over" {
  when http trigger
  do repeat 8 { call("five") }
}
"#;
    let err = compile_err(source);
    assert!(err.message.contains("expands to 40"), "{err}");
    assert_eq!(err.line, 9, "{err}");
}

#[test]
fn exactly_thirty_two_expanded_actions_compile_from_source() {
    let source = r#"
def "four" {
  log("1")
  log("2")
  log("3")
  log("4")
}
rule "edge" {
  when http trigger
  do repeat 8 { call("four") }
}
"#;
    let set = compile_ok(source);
    let rule = set.rule("edge").unwrap();
    let count =
        dali2rust_rules_model::expanded_action_count(rule, &set.blocks).expect("countable");
    assert_eq!(count, 32);
}

#[test]
fn a_repeat_smuggled_into_a_repeat_through_a_def_fails_from_source() {
    let source = r#"
def "looped" {
  repeat 2 { log("x") }
}
rule "nest" {
  when http trigger
  do repeat 2 { call("looped") }
}
"#;
    let err = compile_err(source);
    assert!(err.message.contains("nested repeat"), "{err}");
}

#[test]
fn deleting_a_def_with_live_references_names_the_rules_in_the_error() {
    let source = r#"
rule "первое" {
  when http trigger
  do call("исчез")
}
rule "второе" {
  when http trigger
  do call("исчез")
}
"#;
    let err = compile_err(source);
    assert!(err.message.contains("unknown block \"исчез\""), "{err}");
    assert!(err.message.contains("\"первое\""), "{err}");
    assert!(err.message.contains("\"второе\""), "{err}");
}

#[test]
fn the_ninth_source_action_is_rejected_where_it_stands() {
    let source = "\
rule \"t\" {\n\
  when http trigger\n\
  do log(\"1\")\n\
     log(\"2\")\n\
     log(\"3\")\n\
     log(\"4\")\n\
     log(\"5\")\n\
     log(\"6\")\n\
     log(\"7\")\n\
     log(\"8\")\n\
     log(\"9\")\n\
}\n";
    let err = compile_err(source);
    assert_eq!(err.line, 11, "{err}");
    assert!(err.message.contains("more than 8 actions"), "{err}");
}

#[test]
fn an_oversized_source_is_rejected_before_parsing() {
    let big = "#".repeat(dali2rust_rules_model::limits::MAX_RULES_SOURCE_BYTES + 1);
    let err = RulesLangV1
        .compile(&big, &support::resolver())
        .unwrap_err();
    assert!(err.message.contains("stored-document limit"), "{err}");
    assert_eq!((err.line, err.column), (1, 1));
}

#[test]
fn source_bytes_are_the_limit_not_characters() {
    let half = dali2rust_rules_model::limits::MAX_RULES_SOURCE_BYTES / 2 + 1;
    let big = format!("#{}", "ю".repeat(half));
    assert!(big.len() > dali2rust_rules_model::limits::MAX_RULES_SOURCE_BYTES);
    assert!(RulesLangV1.compile(&big, &support::resolver()).is_err());
}
