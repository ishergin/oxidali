use crate::action::{Action, FlowAction, SceneAction, StateAction};
use crate::condition::{Condition, VarOperand};
use crate::error::ModelError;
use crate::rule::{DefBlock, Rule, RuleSet};
use crate::trigger::Trigger;
use crate::value::ValueExpr;

pub const MAX_RULES: usize = 64;
pub const MAX_BLOCKS: usize = 16;
pub const MIN_TRIGGERS_PER_RULE: usize = 1;
pub const MAX_TRIGGERS_PER_RULE: usize = 4;
pub const MAX_CONDITIONS_PER_RULE: usize = 4;
pub const MIN_ACTIONS_PER_RULE: usize = 1;
pub const MAX_ACTIONS_PER_RULE: usize = 8;
pub const MAX_ACTIONS_EXPANDED: usize = 32;
pub const MAX_BLOCK_CALL_DEPTH: usize = 2;
pub const MAX_REPEAT: u8 = 8;
pub const MAX_NAME_BYTES: usize = 48;
pub const MAX_VAR_TEXT_BYTES: usize = 16;
pub const MAX_MQTT_TOPIC_BYTES: usize = 48;
pub const MAX_MQTT_PAYLOAD_BYTES: usize = 48;
pub const MAX_RULES_SOURCE_BYTES: usize = 12240;
pub const MIN_EVERY_PERIOD_MS: u32 = 1000;
pub const MAX_SCENE_CYCLE_ENTRIES: usize = 16;
pub const MAX_SCENE: u8 = 15;
pub const MAX_LEVEL: u8 = 254;
pub const MAX_SHORT_ADDRESS: u8 = 63;
pub const MAX_INSTANCE_NUMBER: u8 = 31;
pub const MAX_INSTANCE_GROUP: u8 = 31;
pub const MAX_INPUT_VALUE_10BIT: u16 = 1023;

#[derive(Clone, Copy)]
struct Ctx<'a> {
    rule: &'a str,
    call_depth: usize,
    in_repeat: bool,
    in_conditional: bool,
    in_after: bool,
}

impl<'a> Ctx<'a> {
    fn root(rule: &'a str) -> Ctx<'a> {
        Ctx {
            rule,
            call_depth: 0,
            in_repeat: false,
            in_conditional: false,
            in_after: false,
        }
    }
}

pub fn validate(set: &RuleSet) -> Result<(), ModelError> {
    check_document_counts(set)?;
    check_unique_names(set)?;
    check_block_references(set)?;
    check_rule_references(set)?;
    for block in &set.blocks {
        check_block_shape(block)?;
    }
    for rule in &set.rules {
        check_rule_shape(rule)?;
        check_rule_payloads(rule)?;
        let count = expanded_action_count(rule, &set.blocks)?;
        if count > MAX_ACTIONS_EXPANDED {
            return Err(ModelError::ExpandedActionsExceeded {
                rule: rule.name.clone(),
                count,
            });
        }
    }
    Ok(())
}

pub fn expanded_action_count(rule: &Rule, blocks: &[DefBlock]) -> Result<usize, ModelError> {
    count_actions(&rule.actions, blocks, Ctx::root(&rule.name))
}

fn count_actions(actions: &[Action], blocks: &[DefBlock], ctx: Ctx<'_>) -> Result<usize, ModelError> {
    let mut total = 0usize;
    for action in actions {
        total = total.saturating_add(count_action(action, blocks, ctx)?);
    }
    Ok(total)
}

fn count_action(action: &Action, blocks: &[DefBlock], ctx: Ctx<'_>) -> Result<usize, ModelError> {
    let Action::Flow(flow) = action else {
        return Ok(1);
    };
    match flow {
        FlowAction::Repeat { count, actions } => count_repeat(*count, actions, blocks, ctx),
        FlowAction::Conditional { then_actions, else_actions, .. } => {
            count_conditional(then_actions, else_actions, blocks, ctx)
        }
        FlowAction::After { actions, .. } => count_after(actions, blocks, ctx),
        FlowAction::Call { block } => count_call(block, blocks, ctx),
        _ => Ok(1),
    }
}

fn count_repeat(
    count: u8,
    actions: &[Action],
    blocks: &[DefBlock],
    ctx: Ctx<'_>,
) -> Result<usize, ModelError> {
    if ctx.in_repeat {
        return Err(ModelError::NestedRepeat { rule: ctx.rule.into() });
    }
    if count == 0 || count > MAX_REPEAT {
        return Err(ModelError::RepeatCountOutOfRange { rule: ctx.rule.into(), count });
    }
    let inner = count_actions(actions, blocks, Ctx { in_repeat: true, ..ctx })?;
    Ok(inner.saturating_mul(count as usize))
}

fn count_conditional(
    then_actions: &[Action],
    else_actions: &[Action],
    blocks: &[DefBlock],
    ctx: Ctx<'_>,
) -> Result<usize, ModelError> {
    if ctx.in_conditional {
        return Err(ModelError::NestedConditional { rule: ctx.rule.into() });
    }
    let nested = Ctx { in_conditional: true, ..ctx };
    let then_count = count_actions(then_actions, blocks, nested)?;
    let else_count = count_actions(else_actions, blocks, nested)?;
    Ok(1usize.saturating_add(then_count).saturating_add(else_count))
}

fn count_after(actions: &[Action], blocks: &[DefBlock], ctx: Ctx<'_>) -> Result<usize, ModelError> {
    if ctx.in_after {
        return Err(ModelError::NestedAfter { rule: ctx.rule.into() });
    }
    let inner = count_actions(actions, blocks, Ctx { in_after: true, ..ctx })?;
    Ok(1usize.saturating_add(inner))
}

fn count_call(block: &str, blocks: &[DefBlock], ctx: Ctx<'_>) -> Result<usize, ModelError> {
    if ctx.call_depth + 1 > MAX_BLOCK_CALL_DEPTH {
        return Err(ModelError::CallDepthExceeded {
            rule: ctx.rule.into(),
            block: block.into(),
        });
    }
    let Some(def) = blocks.iter().find(|b| b.name == block) else {
        return Err(ModelError::UnresolvedBlock {
            block: block.into(),
            referenced_by: vec![ctx.rule.into()],
        });
    };
    count_actions(&def.actions, blocks, Ctx { call_depth: ctx.call_depth + 1, ..ctx })
}

fn check_document_counts(set: &RuleSet) -> Result<(), ModelError> {
    if set.rules.len() > MAX_RULES {
        return Err(ModelError::TooManyRules { count: set.rules.len() });
    }
    if set.blocks.len() > MAX_BLOCKS {
        return Err(ModelError::TooManyBlocks { count: set.blocks.len() });
    }
    Ok(())
}

fn check_name_bytes(name: &str) -> Result<(), ModelError> {
    if name.len() > MAX_NAME_BYTES {
        return Err(ModelError::NameTooLong { name: name.into() });
    }
    Ok(())
}

fn check_unique_names(set: &RuleSet) -> Result<(), ModelError> {
    let mut rules: Vec<&str> = Vec::with_capacity(set.rules.len());
    for rule in &set.rules {
        check_name_bytes(&rule.name)?;
        if rules.contains(&rule.name.as_str()) {
            return Err(ModelError::DuplicateRuleName { name: rule.name.clone() });
        }
        rules.push(&rule.name);
    }
    let mut blocks: Vec<&str> = Vec::with_capacity(set.blocks.len());
    for block in &set.blocks {
        check_name_bytes(&block.name)?;
        if blocks.contains(&block.name.as_str()) {
            return Err(ModelError::DuplicateBlockName { name: block.name.clone() });
        }
        blocks.push(&block.name);
    }
    Ok(())
}

fn check_rule_shape(rule: &Rule) -> Result<(), ModelError> {
    let name = || rule.name.clone();
    if rule.triggers.len() < MIN_TRIGGERS_PER_RULE {
        return Err(ModelError::NoTriggers { rule: name() });
    }
    if rule.triggers.len() > MAX_TRIGGERS_PER_RULE {
        return Err(ModelError::TooManyTriggers { rule: name(), count: rule.triggers.len() });
    }
    if rule.conditions.len() > MAX_CONDITIONS_PER_RULE {
        return Err(ModelError::TooManyConditions { rule: name(), count: rule.conditions.len() });
    }
    if rule.actions.len() < MIN_ACTIONS_PER_RULE {
        return Err(ModelError::NoActions { owner: name() });
    }
    if rule.actions.len() > MAX_ACTIONS_PER_RULE {
        return Err(ModelError::TooManyActions { rule: name(), count: rule.actions.len() });
    }
    check_triggers(rule)
}

fn check_triggers(rule: &Rule) -> Result<(), ModelError> {
    for trigger in &rule.triggers {
        if let Trigger::Every { period_ms } = trigger {
            if period_ms.0 < MIN_EVERY_PERIOD_MS {
                return Err(ModelError::EveryPeriodTooShort {
                    rule: rule.name.clone(),
                    period_ms: period_ms.0,
                });
            }
        }
    }
    Ok(())
}

fn check_block_shape(block: &DefBlock) -> Result<(), ModelError> {
    if block.actions.is_empty() {
        return Err(ModelError::NoActions { owner: block.name.clone() });
    }
    if block.actions.len() > MAX_ACTIONS_EXPANDED {
        return Err(ModelError::TooManyBlockActions {
            block: block.name.clone(),
            count: block.actions.len(),
        });
    }
    for action in &block.actions {
        check_action_payload(&block.name, action)?;
    }
    Ok(())
}

fn check_rule_payloads(rule: &Rule) -> Result<(), ModelError> {
    for condition in &rule.conditions {
        check_condition_payload(&rule.name, condition)?;
    }
    for action in &rule.actions {
        check_action_payload(&rule.name, action)?;
    }
    Ok(())
}

fn check_condition_payload(rule: &str, condition: &Condition) -> Result<(), ModelError> {
    if let Condition::VarCompare { value: VarOperand::Text(text), .. } = condition {
        if text.len() > MAX_VAR_TEXT_BYTES {
            return Err(ModelError::VarTextTooLong { rule: rule.into(), text: text.clone() });
        }
    }
    Ok(())
}

fn check_action_payload(owner: &str, action: &Action) -> Result<(), ModelError> {
    match action {
        Action::Scene(scene) => check_scene_payload(owner, scene),
        Action::State(state) => check_state_payload(owner, state),
        Action::Flow(flow) => check_flow_payload(owner, flow),
        _ => Ok(()),
    }
}

fn check_scene_payload(owner: &str, scene: &SceneAction) -> Result<(), ModelError> {
    let literal_scene = |expr: &ValueExpr| match expr {
        ValueExpr::Literal(n) if *n < 0 || *n > MAX_SCENE as i64 => {
            Err(ModelError::SceneOutOfRange { rule: owner.into(), scene: (*n).clamp(0, 255) as u8 })
        }
        _ => Ok(()),
    };
    match scene {
        SceneAction::Recall { scene, .. } | SceneAction::Apply { scene } => literal_scene(scene),
        SceneAction::Cycle { scenes } => {
            if scenes.len() > MAX_SCENE_CYCLE_ENTRIES {
                return Err(ModelError::SceneCycleTooLong { rule: owner.into(), count: scenes.len() });
            }
            match scenes.iter().find(|s| **s > MAX_SCENE) {
                Some(s) => Err(ModelError::SceneOutOfRange { rule: owner.into(), scene: *s }),
                None => Ok(()),
            }
        }
    }
}

fn check_state_payload(owner: &str, state: &StateAction) -> Result<(), ModelError> {
    match state {
        StateAction::VarSet { value: crate::value::VarValue::Text(text), .. }
            if text.len() > MAX_VAR_TEXT_BYTES =>
        {
            Err(ModelError::VarTextTooLong { rule: owner.into(), text: text.clone() })
        }
        StateAction::MqttPublish { topic, .. } if topic.len() > MAX_MQTT_TOPIC_BYTES => {
            Err(ModelError::MqttTopicTooLong { rule: owner.into(), bytes: topic.len() })
        }
        StateAction::MqttPublish { payload, .. } if payload.len() > MAX_MQTT_PAYLOAD_BYTES => {
            Err(ModelError::MqttPayloadTooLong { rule: owner.into(), bytes: payload.len() })
        }
        _ => Ok(()),
    }
}

fn check_flow_payload(owner: &str, flow: &FlowAction) -> Result<(), ModelError> {
    match flow {
        FlowAction::After { actions, .. } | FlowAction::Repeat { actions, .. } => {
            for action in actions {
                check_action_payload(owner, action)?;
            }
            Ok(())
        }
        FlowAction::Conditional { condition, then_actions, else_actions } => {
            check_condition_payload(owner, condition)?;
            for action in then_actions.iter().chain(else_actions) {
                check_action_payload(owner, action)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_block_references(set: &RuleSet) -> Result<(), ModelError> {
    let mut missing: Option<(String, Vec<String>)> = None;
    let mut visit = |owner: &str, block: &str| {
        if set.block(block).is_some() {
            return;
        }
        match &mut missing {
            Some((name, refs)) if name == block => {
                if !refs.iter().any(|r| r == owner) {
                    refs.push(owner.into());
                }
            }
            Some(_) => {}
            None => missing = Some((block.into(), vec![owner.into()])),
        }
    };
    for rule in &set.rules {
        walk_calls(&rule.actions, &mut |b| visit(&rule.name, b));
    }
    for def in &set.blocks {
        walk_calls(&def.actions, &mut |b| visit(&def.name, b));
    }
    match missing {
        Some((block, referenced_by)) => Err(ModelError::UnresolvedBlock { block, referenced_by }),
        None => Ok(()),
    }
}

fn walk_calls(actions: &[Action], visit: &mut dyn FnMut(&str)) {
    for action in actions {
        if let Action::Flow(flow) = action {
            match flow {
                FlowAction::Call { block } => visit(block),
                FlowAction::After { actions, .. } | FlowAction::Repeat { actions, .. } => {
                    walk_calls(actions, visit);
                }
                FlowAction::Conditional { then_actions, else_actions, .. } => {
                    walk_calls(then_actions, visit);
                    walk_calls(else_actions, visit);
                }
                _ => {}
            }
        }
    }
}

fn check_rule_references(set: &RuleSet) -> Result<(), ModelError> {
    for rule in &set.rules {
        for trigger in &rule.triggers {
            if let Trigger::RuleFails { rule: target } = trigger {
                require_rule(set, target, &rule.name)?;
            }
        }
        for condition in &rule.conditions {
            check_condition_rule_ref(set, condition, &rule.name)?;
        }
        walk_rule_refs(&rule.actions, &mut |target| require_rule(set, target, &rule.name))?;
    }
    for def in &set.blocks {
        walk_rule_refs(&def.actions, &mut |target| require_rule(set, target, &def.name))?;
    }
    Ok(())
}

fn check_condition_rule_ref(set: &RuleSet, condition: &Condition, owner: &str) -> Result<(), ModelError> {
    match condition {
        Condition::RuleEnabled { rule } | Condition::LastFiredOlderThan { rule, .. } => {
            require_rule(set, rule, owner)
        }
        _ => Ok(()),
    }
}

fn require_rule(set: &RuleSet, target: &str, owner: &str) -> Result<(), ModelError> {
    if set.rule(target).is_none() {
        return Err(ModelError::UnknownRuleReference {
            rule: target.into(),
            referenced_by: owner.into(),
        });
    }
    Ok(())
}

fn walk_rule_refs(
    actions: &[Action],
    visit: &mut dyn FnMut(&str) -> Result<(), ModelError>,
) -> Result<(), ModelError> {
    for action in actions {
        match action {
            Action::State(StateAction::RuleEnable { rule })
            | Action::State(StateAction::RuleDisable { rule }) => visit(rule)?,
            Action::Flow(FlowAction::After { actions, .. })
            | Action::Flow(FlowAction::Repeat { actions, .. }) => walk_rule_refs(actions, visit)?,
            Action::Flow(FlowAction::Conditional { then_actions, else_actions, .. }) => {
                walk_rule_refs(then_actions, visit)?;
                walk_rule_refs(else_actions, visit)?;
            }
            _ => {}
        }
    }
    Ok(())
}
