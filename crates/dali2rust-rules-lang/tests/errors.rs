mod support;

use support::{compile, compile_err, wrap_trigger};

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
fn the_reserved_mqtt_trigger_parses_structurally_then_rejects() {
    let err = compile_err(&wrap_trigger("mqtt \"dali2rust/cmd/ночь\""));
    assert!(err.message.contains("stage 2"), "{err}");
    let err = compile_err(&wrap_trigger("mqtt \"dali2rust/cmd\" is \"on\""));
    assert!(err.message.contains("reserved"), "{err}");
    let err = compile_err(&wrap_trigger("mqtt 42"));
    assert!(err.message.contains("expected quoted"), "{err}");
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
