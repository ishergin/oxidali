use dali2rust_rules_model::limits::MAX_ACTIONS_EXPANDED;
use dali2rust_rules_model::{
    expanded_action_count, validate, Action, DefBlock, FlowAction, ModelError, Rule, RuleSet,
    StateAction, Trigger,
};

fn simple_action() -> Action {
    Action::State(StateAction::Log { text: "x".into() })
}

fn rule_with_actions(name: &str, actions: Vec<Action>) -> Rule {
    Rule {
        name: name.into(),
        enabled: true,
        cooldown_ms: 0,
        hold_hcl: true,
        triggers: vec![Trigger::HttpTrigger],
        conditions: vec![],
        actions,
    }
}

fn set_with(blocks: Vec<DefBlock>, rules: Vec<Rule>) -> RuleSet {
    RuleSet {
        lang_id: 1,
        blocks,
        rules,
    }
}

fn def(name: &str, actions: Vec<Action>) -> DefBlock {
    DefBlock {
        name: name.into(),
        actions,
    }
}

fn call(block: &str) -> Action {
    Action::Flow(FlowAction::Call {
        block: block.into(),
    })
}

fn repeat(count: u8, actions: Vec<Action>) -> Action {
    Action::Flow(FlowAction::Repeat { count, actions })
}

#[test]
fn repeat_eight_of_a_five_action_def_is_rejected() {
    let blocks = vec![def("five", (0..5).map(|_| simple_action()).collect())];
    let rules = vec![rule_with_actions("over", vec![repeat(8, vec![call("five")])])];
    let set = set_with(blocks, rules);
    match validate(&set) {
        Err(ModelError::ExpandedActionsExceeded { rule, count }) => {
            assert_eq!(rule, "over");
            assert_eq!(count, 40);
        }
        other => panic!("expected ExpandedActionsExceeded, got {other:?}"),
    }
}

#[test]
fn exactly_thirty_two_expanded_actions_are_accepted() {
    let blocks = vec![def("four", (0..4).map(|_| simple_action()).collect())];
    let rules = vec![rule_with_actions("edge", vec![repeat(8, vec![call("four")])])];
    let set = set_with(blocks, rules);
    assert_eq!(
        expanded_action_count(&set.rules[0], &set.blocks).unwrap(),
        MAX_ACTIONS_EXPANDED
    );
    validate(&set).expect("32 expanded actions are the ceiling, not past it");
}

#[test]
fn a_def_calling_a_def_is_depth_two_and_legal() {
    let blocks = vec![
        def("inner", vec![simple_action()]),
        def("outer", vec![call("inner"), simple_action()]),
    ];
    let rules = vec![rule_with_actions("ok", vec![call("outer")])];
    validate(&set_with(blocks, rules)).expect("call depth 2 is the limit, not past it");
}

#[test]
fn call_depth_three_is_rejected() {
    let blocks = vec![
        def("a", vec![call("b")]),
        def("b", vec![call("c")]),
        def("c", vec![simple_action()]),
    ];
    let rules = vec![rule_with_actions("deep", vec![call("a")])];
    match validate(&set_with(blocks, rules)) {
        Err(ModelError::CallDepthExceeded { rule, block }) => {
            assert_eq!(rule, "deep");
            assert_eq!(block, "c");
        }
        other => panic!("expected CallDepthExceeded, got {other:?}"),
    }
}

#[test]
fn recursion_cannot_hide_behind_depth() {
    let blocks = vec![def("loop", vec![call("loop")])];
    let rules = vec![rule_with_actions("rec", vec![call("loop")])];
    assert!(matches!(
        validate(&set_with(blocks, rules)),
        Err(ModelError::CallDepthExceeded { .. })
    ));
}

#[test]
fn repeat_smuggled_through_a_def_is_still_nested_repeat() {
    let blocks = vec![def("looped", vec![repeat(2, vec![simple_action()])])];
    let rules = vec![rule_with_actions("nest", vec![repeat(2, vec![call("looped")])])];
    assert!(matches!(
        validate(&set_with(blocks, rules)),
        Err(ModelError::NestedRepeat { .. })
    ));
}

#[test]
fn deleting_a_def_with_live_references_names_the_rules() {
    let rules = vec![
        rule_with_actions("first", vec![call("gone")]),
        rule_with_actions("second", vec![call("gone")]),
    ];
    match validate(&set_with(vec![], rules)) {
        Err(ModelError::UnresolvedBlock { block, referenced_by }) => {
            assert_eq!(block, "gone");
            assert_eq!(referenced_by, ["first", "second"]);
        }
        other => panic!("expected UnresolvedBlock, got {other:?}"),
    }
}

#[test]
fn duplicate_rule_names_are_rejected() {
    let rules = vec![
        rule_with_actions("same", vec![simple_action()]),
        rule_with_actions("same", vec![simple_action()]),
    ];
    assert!(matches!(
        validate(&set_with(vec![], rules)),
        Err(ModelError::DuplicateRuleName { .. })
    ));
}

#[test]
fn a_dangling_rule_reference_is_an_error() {
    let actions = vec![Action::State(StateAction::RuleDisable {
        rule: "phantom".into(),
    })];
    let rules = vec![rule_with_actions("refs", actions)];
    match validate(&set_with(vec![], rules)) {
        Err(ModelError::UnknownRuleReference { rule, referenced_by }) => {
            assert_eq!(rule, "phantom");
            assert_eq!(referenced_by, "refs");
        }
        other => panic!("expected UnknownRuleReference, got {other:?}"),
    }
}

#[test]
fn conditional_counts_both_branches_plus_itself() {
    let cond = dali2rust_rules_model::Condition::ControllerActive;
    let action = Action::Flow(FlowAction::Conditional {
        condition: cond,
        then_actions: vec![simple_action(), simple_action()],
        else_actions: vec![simple_action()],
    });
    let rule = rule_with_actions("branchy", vec![action]);
    assert_eq!(expanded_action_count(&rule, &[]).unwrap(), 4);
}

#[test]
fn after_counts_its_body_plus_itself() {
    let action = Action::Flow(FlowAction::After {
        delay_ms: dali2rust_rules_model::DurationMs(300_000),
        actions: vec![simple_action(), simple_action()],
    });
    let rule = rule_with_actions("delayed", vec![action]);
    assert_eq!(expanded_action_count(&rule, &[]).unwrap(), 3);
}
