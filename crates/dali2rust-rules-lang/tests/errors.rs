mod support;

use dali2rust_rules_model::limits::MAX_MQTT_TRIGGER_TOPICS;
use support::{compile, compile_err, compile_ok, wrap_action, wrap_condition, wrap_trigger};

const TWO_RULES: &str = r#"# comment line
rule "первое" {
  when http trigger
  do lamp("коридор").on()
}
rule "второе" {
  when http trigger
  do lamp("коридор").togle()
}
"#;

#[test]
fn a_misspelled_verb_mid_file_reports_its_line_and_column() {
    let err = compile_err(TWO_RULES);
    assert_eq!((err.line, err.column), (8, 22), "{err}");
    assert!(err.message.contains("togle"), "{err}");
}

#[test]
fn an_unknown_name_reports_the_name_position() {
    let strict = dali2rust_rules_model::testing::StubResolver::strict();
    let source = "rule \"t\" {\n  when http trigger\n  do lamp(\"нет такой\").on()\n}\n";
    let err = dali2rust_rules_model::RuleCompiler::compile(
        &dali2rust_rules_lang::RulesLangV1,
        source,
        &strict,
    )
    .unwrap_err();
    assert_eq!(err.line, 3, "{err}");
    assert!(err.message.contains("unknown lamp"), "{err}");
    assert!(err.message.contains("нет такой"), "{err}");
}

#[test]
fn an_unknown_adapter_is_a_compile_error_with_coordinates() {
    let err = compile_err(&wrap_trigger("input(adapter=7, dev=3, inst=0) is press"));
    assert!(err.message.contains("unknown adapter 7"), "{err}");
    assert_eq!(err.line, 4, "the qualifier sits on the wrapped when line: {err}");
}

#[test]
fn a_topic_filter_is_refused_at_the_topic() {
    for snippet in ["mqtt \"home/+/mode\"", "mqtt \"home/#\"", "mqtt \"\""] {
        let err = compile_err(&wrap_trigger(snippet));
        assert_eq!((err.line, err.column), (4, 13), "{snippet}: {err}");
        assert!(err.message.contains("one exact topic"), "{snippet}: {err}");
    }
    let err = compile_err(&wrap_trigger("mqtt 42"));
    assert!(err.message.contains("expected quoted"), "{err}");
}

const MALFORMED_TOPICS: &[&str] = &[
    "home/scene\t",
    "home/\u{1}",
    "home/\u{7f}",
    "home/\u{85}",
    "home/\u{9f}",
    "home/\u{fdd0}",
    "home/\u{fdef}",
    "home/\u{fffe}",
    "home/\u{1ffff}",
    "home/\u{10ffff}",
];

#[test]
fn a_control_character_or_a_noncharacter_in_a_topic_is_refused_at_the_topic() {
    for topic in MALFORMED_TOPICS {
        let err = compile_err(&wrap_trigger(&format!("mqtt \"{topic}\"")));
        assert_eq!((err.line, err.column), (4, 13), "trigger {topic:?}: {err}");
        assert!(err.message.contains("one exact topic"), "trigger {topic:?}: {err}");
        let err = compile_err(&wrap_action(&format!("mqtt.publish(\"{topic}\", \"on\")")));
        assert_eq!((err.line, err.column), (5, 19), "publish {topic:?}: {err}");
        assert!(err.message.contains("one exact topic"), "publish {topic:?}: {err}");
    }
    compile_ok(&wrap_trigger("mqtt \"дом/сцена/\u{a0}\u{fffd}\""));
}

#[test]
fn a_publish_to_a_topic_filter_is_refused_at_the_topic() {
    for topic in ["home/+/mode", "home/#", ""] {
        let err = compile_err(&wrap_action(&format!("mqtt.publish(\"{topic}\", \"on\")")));
        assert_eq!((err.line, err.column), (5, 19), "{topic:?}: {err}");
        assert!(err.message.contains("one exact topic"), "{topic:?}: {err}");
    }
}

#[test]
fn an_mqtt_topic_or_payload_past_the_frame_is_refused_where_it_is_written() {
    let long = "x".repeat(49);
    let err = compile_err(&wrap_trigger(&format!("mqtt \"{long}\"")));
    assert_eq!((err.line, err.column), (4, 13), "{err}");
    assert!(err.message.contains("mqtt topic exceeds 48 bytes"), "{err}");
    let err = compile_err(&wrap_trigger(&format!("mqtt \"t\" is \"{long}\"")));
    assert_eq!((err.line, err.column), (4, 20), "{err}");
    assert!(err.message.contains("mqtt payload exceeds 48 bytes"), "{err}");
}

#[test]
fn the_ninth_distinct_topic_is_refused_at_its_trigger() {
    let mut source = String::new();
    for n in 0..MAX_MQTT_TRIGGER_TOPICS {
        source.push_str(&format!(
            "rule \"r{n}\" {{ when mqtt \"t/{n}\" when mqtt \"t/0\" do log(\"x\") }}\n"
        ));
    }
    compile_ok(&source);
    source.push_str("rule \"late\" {\n  when mqtt \"t/3\"\n  when mqtt \"t/8\"\n  do log(\"x\")\n}\n");
    let err = compile_err(&source);
    assert_eq!((err.line, err.column), (11, 8), "{err}");
    assert!(err.message.contains("at most 8 mqtt trigger topics"), "{err}");
}

#[test]
fn duplicate_rule_names_point_at_the_second_occurrence() {
    let source = "rule \"same\" { when http trigger do log(\"a\") }\n\
                  rule \"same\" { when http trigger do log(\"b\") }\n";
    let err = compile_err(source);
    assert_eq!(err.line, 2, "{err}");
    assert!(err.message.contains("duplicate"), "{err}");
}

#[test]
fn a_fifth_when_reports_the_fifth_when_line() {
    let source = "\
rule \"t\" {\n\
  when http trigger\n\
  when controller starts\n\
  when controller becomes active\n\
  when every 1s\n\
  when at 07:00\n\
  do log(\"x\")\n\
}\n";
    let err = compile_err(source);
    assert_eq!(err.line, 6, "{err}");
    assert!(err.message.contains("when"), "{err}");
}

const GARBAGE: &[&str] = &[
    "",
    "   \n\n# only a comment\n",
    "rule",
    "rule \"",
    "rule \"x\" {",
    "rule \"x\" { when }",
    "rule \"x\" { do }",
    "{}{}{}",
    "rule \"x\" x{ when http trigger do log(\"y\") }",
    "def \"a\" { call(\"a\") } rule \"r\" { when http trigger do call(\"a\") }",
    "rule \"x\" { when input(dev=999999999999999999999, inst=0) is press do log(\"y\") }",
    "rule \"x\" { when at 99:99 do log(\"y\") }",
    "rule \"x\" { when every 0.0001s do log(\"y\") }",
    "rule \"x\" { when http trigger do wait 5x }",
    "rule \"x\" { when http trigger do lamp(\"y\").level(1e9) }",
    "rule \"x\" { when http trigger do repeat 8 { repeat 8 { log(\"y\") } } }",
    "rule \"x\" { when http trigger do after 1s do { after 1s do { log(\"y\") } } }",
    "rule \"x\" { when http trigger do if sun is up { if sun is up { log(\"y\") } } }",
    "rule \"x\" { when http trigger do lamp(\"y\").on(level=254, level=254) }",
    "rule \"x\" { when http trigger do mqtt.publish(\"t\") }",
    "when do if else and or not",
    "\u{0}\u{1}\u{2}",
    "🦀🦀🦀",
    "rule \"🦀\" { when http trigger do log(\"🦀\") } trailing",
    "rule \"x\" { when http trigger do var(\"v\").set(\"аааааааааааааааааааа\") }",
];

#[test]
fn garbage_never_panics_and_always_carries_coordinates() {
    for source in GARBAGE {
        match compile(source) {
            Ok(_) => {}
            Err(e) => {
                assert!(e.line >= 1 && e.column >= 1, "no coordinates for: {source:?}");
            }
        }
    }
}

#[test]
fn every_prefix_truncation_of_a_valid_document_errors_cleanly() {
    let source = "\
def \"blk\" { log(\"a\") }\n\
rule \"полное правило\" {\n\
  when input(dev=3, inst=0) is short_press\n\
  if time in 23:00 .. sunrise+15m and var(\"режим\") != \"ночь\"\n\
  do lamp(\"коридор\").level(+8)\n\
     after 1s do { call(\"blk\") }\n\
}\n";
    assert!(compile(source).is_ok(), "the base document must be valid");
    for (offset, _) in source.char_indices() {
        let prefix = &source[..offset];
        match compile(prefix) {
            Ok(set) => {
                assert!(
                    set.rules.len() + set.blocks.len() <= 2,
                    "prefix at {offset} produced an impossible set"
                );
            }
            Err(e) => assert!(e.line >= 1 && e.column >= 1, "prefix at {offset}: {e}"),
        }
    }
}

#[test]
fn mutated_documents_never_panic() {
    let base = "rule \"t\" { when at 07:30 on mon..fri do lamp(\"x\").cct(2700) }";
    let mut state: u32 = 0x1234_5678;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state
    };
    for _ in 0..2_000 {
        let mut bytes = base.as_bytes().to_vec();
        let flips = 1 + (next() as usize % 4);
        for _ in 0..flips {
            let i = next() as usize % bytes.len();
            bytes[i] = (next() & 0xFF) as u8;
        }
        if let Ok(mutated) = String::from_utf8(bytes) {
            let _ = compile(&mutated);
        }
    }
}

#[test]
fn fade_is_refused_and_says_where_the_duration_lives() {
    let err = compile_err(
        "rule \"t\" {\n  when http trigger\n  do lamp(\"коридор\").off(fade=3s)\n}\n",
    );
    assert!(err.message.contains("fade"), "{err}");
    assert!(
        err.message.contains("write-attributes"),
        "the refusal must name the surface that sets it: {err}",
    );
    assert!(err.column > 1, "{err}");
}

#[test]
fn refusing_fade_leaves_durations_working_everywhere_else() {
    let doc = "rule \"t\" {\n  when http trigger\n  cooldown 5m\n  do lamp(\"коридор\").off()\n}\n";
    let _ = compile(doc);
}

#[test]
fn hold_hcl_false_is_refused_at_both_sites() {
    for source in [
        "rule \"t\" {\n  when http trigger\n  do lamp(\"коридор\").off(hold_hcl=false)\n}\n",
        "rule \"t\" hold_hcl false {\n  when http trigger\n  do lamp(\"коридор\").off()\n}\n",
    ] {
        let err = compile_err(source);
        assert!(err.message.contains("ISSUE-96"), "{err}");
        assert!(err.message.contains("hold_hcl=false"), "{err}");
    }
}

#[test]
fn hold_hcl_true_is_still_accepted() {
    let _ = compile("rule \"t\" {\n  when http trigger\n  do lamp(\"коридор\").off(hold_hcl=true)\n}\n");
    let _ = compile("rule \"t\" hold_hcl true {\n  when http trigger\n  do lamp(\"коридор\").off()\n}\n");
}

#[test]
fn a_lamp_id_beyond_the_virtual_lamps_is_refused_at_the_number() {
    let err = compile_err(&wrap_action("lamp(300).off()"));
    assert_eq!((err.line, err.column), (5, 11), "{err}");
    assert!(err.message.contains("lamp id 300 out of range 0..=63"), "{err}");
    let err = compile_err(&wrap_action("lamp(64).off()"));
    assert!(err.message.contains("lamp id 64"), "{err}");
    let _ = compile_ok(&wrap_action("lamp(63).off()"));
}

#[test]
fn a_group_id_beyond_the_dali_groups_is_refused_at_the_number() {
    let err = compile_err(&wrap_action("group(300).off()"));
    assert_eq!((err.line, err.column), (5, 12), "{err}");
    assert!(err.message.contains("group id 300 out of range 0..=15"), "{err}");
    let err = compile_err(&wrap_action("scene(3).recall(group(16))"));
    assert!(err.message.contains("group id 16"), "{err}");
    let _ = compile_ok(&wrap_action("group(15).off()"));
}

#[test]
fn an_out_of_range_id_is_refused_wherever_a_reference_is_written() {
    for (source, what, at) in [
        (wrap_trigger("lamp(64) turns on"), "lamp id 64", (4, 13)),
        (wrap_trigger("group(16) becomes any_on"), "group id 16", (4, 14)),
        (wrap_condition("lamp(64) is on"), "lamp id 64", (5, 11)),
        (wrap_condition("hcl is overridden for group(16)"), "group id 16", (5, 34)),
        (wrap_action("hcl.resume(group(16))"), "group id 16", (5, 23)),
        (wrap_action("broadcast.level(lamp(64).level)"), "lamp id 64", (5, 27)),
    ] {
        let err = compile_err(&source);
        assert!(err.message.contains(what), "{what}: {err}");
        assert_eq!((err.line, err.column), at, "{what}: {err}");
    }
}
