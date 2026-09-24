use dali2rust_rules_model::docs::{
    DOCUMENTED_ACTION_KINDS, DOCUMENTED_CONDITION_KINDS, DOCUMENTED_TRIGGER_KINDS,
    DOCUMENTED_VALUE_KINDS,
};
use dali2rust_rules_model::{ActionKind, ConditionKind, TriggerKind, ValueKind};
use std::collections::BTreeSet;

fn assert_same_names(registry: Vec<&'static str>, documented: &[&str], what: &str) {
    let reg: BTreeSet<&str> = registry.iter().copied().collect();
    let doc: BTreeSet<&str> = documented.iter().copied().collect();
    assert_eq!(
        reg.len(),
        registry.len(),
        "{what}: duplicate names in the registry"
    );
    assert_eq!(
        doc.len(),
        documented.len(),
        "{what}: duplicate names in the doc table"
    );
    let undocumented: Vec<&&str> = reg.difference(&doc).collect();
    let unregistered: Vec<&&str> = doc.difference(&reg).collect();
    assert!(
        undocumented.is_empty() && unregistered.is_empty(),
        "{what}: registry without doc row: {undocumented:?}; doc row without registry: {unregistered:?}"
    );
}

#[test]
fn trigger_kinds_match_the_doc_table() {
    let names = TriggerKind::ALL.into_iter().map(TriggerKind::name).collect();
    assert_same_names(names, DOCUMENTED_TRIGGER_KINDS, "triggers");
}

#[test]
fn condition_kinds_match_the_doc_table() {
    let names = ConditionKind::ALL
        .into_iter()
        .map(ConditionKind::name)
        .collect();
    assert_same_names(names, DOCUMENTED_CONDITION_KINDS, "conditions");
}

#[test]
fn action_kinds_match_the_doc_table() {
    let names = ActionKind::ALL.into_iter().map(ActionKind::name).collect();
    assert_same_names(names, DOCUMENTED_ACTION_KINDS, "actions");
}

#[test]
fn value_kinds_match_the_doc_table() {
    let names = ValueKind::ALL.into_iter().map(ValueKind::name).collect();
    assert_same_names(names, DOCUMENTED_VALUE_KINDS, "values");
}

#[test]
fn kind_names_are_disjoint_enough_to_tag_json() {
    let triggers: BTreeSet<&str> = TriggerKind::ALL.into_iter().map(TriggerKind::name).collect();
    let conditions: BTreeSet<&str> = ConditionKind::ALL
        .into_iter()
        .map(ConditionKind::name)
        .collect();
    let shared: Vec<&&str> = triggers.intersection(&conditions).collect();
    assert_eq!(shared, [&"input_occupancy"], "trigger/condition name overlap changed");
}
