#![allow(dead_code, reason = "Shared across several test binaries; each binary uses a subset, so the per-binary dead_code lint would fire on helpers its siblings use")]

use dali2rust_rules_lang::RulesLangV1;
use dali2rust_rules_model::testing::StubResolver;
use dali2rust_rules_model::{CompileError, RuleCompiler, RuleSet};

pub fn resolver() -> StubResolver {
    StubResolver::permissive().with_adapter(1)
}

pub fn compile(source: &str) -> Result<RuleSet, CompileError> {
    RulesLangV1.compile(source, &resolver())
}

pub fn compile_ok(source: &str) -> RuleSet {
    match compile(source) {
        Ok(set) => set,
        Err(e) => panic!("expected the document to compile, got {e}\n---\n{source}"),
    }
}

pub fn compile_err(source: &str) -> CompileError {
    match compile(source) {
        Ok(_) => panic!("expected a compile error\n---\n{source}"),
        Err(e) => e,
    }
}

pub fn wrap_trigger(snippet: &str) -> String {
    format!(
        "rule \"X\" {{ when http trigger do log(\"c\") }}\n\
         rule \"ночной режим\" {{ when http trigger do log(\"c\") }}\n\
         rule \"t\" {{\n  when {snippet}\n  do log(\"x\")\n}}\n"
    )
}

pub fn wrap_condition(snippet: &str) -> String {
    format!(
        "rule \"X\" {{ when http trigger do log(\"c\") }}\n\
         rule \"ночной режим\" {{ when http trigger do log(\"c\") }}\n\
         rule \"t\" {{\n  when http trigger\n  if {snippet}\n  do log(\"x\")\n}}\n"
    )
}

pub fn wrap_action(snippet: &str) -> String {
    format!(
        "rule \"X\" {{ when http trigger do log(\"c\") }}\n\
         rule \"ночной режим\" {{ when http trigger do log(\"c\") }}\n\
         rule \"t\" {{\n  when http trigger\n  do {snippet}\n}}\n"
    )
}

pub fn the_rule(set: &RuleSet) -> &dali2rust_rules_model::Rule {
    set.rule("t").expect("wrapped rule \"t\" present")
}
