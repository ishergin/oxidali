use crate::cursor::Cursor;
use crate::lexer::TokenKind;
use crate::parser::action_light::{self, call_args, ArgBag, ArgValue};
use crate::parser::condition;
use crate::parser::expr;
use crate::parser::refs;
use crate::parser::trigger::quoted_ref;
use dali2rust_rules_model::limits::{
    MAX_INSTANCE_GROUP, MAX_MQTT_PAYLOAD_BYTES, MAX_MQTT_TOPIC_BYTES, MAX_REPEAT, MAX_SCENE,
    MAX_SCENE_CYCLE_ENTRIES, MAX_VAR_TEXT_BYTES,
};
use dali2rust_rules_model::{
    Action, ActionKind, CompileError, DurationMs, FlowAction, HclAction, InputAction, SceneAction,
    StateAction, ValueExpr, VarValue,
};

pub const PARSED_ACTION_KINDS: &[ActionKind] = &[
    ActionKind::LightOn,
    ActionKind::LightOff,
    ActionKind::LightToggle,
    ActionKind::LightLevel,
    ActionKind::LightDim,
    ActionKind::LightDimHold,
    ActionKind::LightCct,
    ActionKind::LightXy,
    ActionKind::LightRgb,
    ActionKind::LightLastActive,
    ActionKind::LightStopFade,
    ActionKind::SceneCycle,
    ActionKind::SceneRecall,
    ActionKind::SceneApply,
    ActionKind::HclResume,
    ActionKind::HclHold,
    ActionKind::HclEnable,
    ActionKind::HclDisable,
    ActionKind::InputFeedbackOn,
    ActionKind::InputFeedbackOff,
    ActionKind::InputCancelHold,
    ActionKind::InputCatchMovement,
    ActionKind::PanelSelect,
    ActionKind::Wait,
    ActionKind::After,
    ActionKind::TimerStart,
    ActionKind::TimerRestart,
    ActionKind::TimerCancel,
    ActionKind::Call,
    ActionKind::Repeat,
    ActionKind::Conditional,
    ActionKind::VarSet,
    ActionKind::VarAdd,
    ActionKind::RuleEnable,
    ActionKind::RuleDisable,
    ActionKind::MqttPublish,
    ActionKind::Log,
    ActionKind::StatCount,
];

#[derive(Debug, Clone, Copy, Default)]
pub struct Nesting {
    pub in_repeat: bool,
    pub in_conditional: bool,
    pub in_after: bool,
}

pub fn action(c: &mut Cursor<'_>, nest: Nesting) -> Result<Action, CompileError> {
    let pos = c.here();
    let Some(word) = peek_ident(c) else {
        return Err(pos.err("expected an action"));
    };
    match word.as_str() {
        "lamp" | "group" | "broadcast" => action_light::light_action(c),
        "scene" => skip_then(c, scene_action),
        "hcl" => skip_then(c, hcl_action),
        "input" => skip_then(c, input_action),
        "panel_select" => skip_then(c, panel_select),
        "wait" => skip_then(c, wait_action),
        "after" => after_action(c, nest),
        "timer" => skip_then(c, timer_action),
        "call" => skip_then(c, call_action),
        "repeat" => repeat_action(c, nest),
        "if" => conditional_action(c, nest),
        "var" => skip_then(c, var_action),
        "rule" => skip_then(c, rule_action),
        "mqtt" => skip_then(c, mqtt_action),
        "log" => skip_then(c, log_action),
        "stat" => skip_then(c, stat_action),
        _ => Err(pos.err(format!("unknown action \"{word}\""))),
    }
}

fn peek_ident(c: &Cursor<'_>) -> Option<String> {
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Ident(word)) => Some(word.clone()),
        _ => None,
    }
}

fn skip_then(
    c: &mut Cursor<'_>,
    parse: fn(&mut Cursor<'_>) -> Result<Action, CompileError>,
) -> Result<Action, CompileError> {
    c.advance();
    parse(c)
}

pub fn actions_until_rbrace(c: &mut Cursor<'_>, nest: Nesting) -> Result<Vec<Action>, CompileError> {
    let mut actions = Vec::new();
    while !c.accept(&TokenKind::RBrace) {
        if c.peek().is_none() {
            return Err(c.err_here("expected `}`"));
        }
        actions.push(action(c, nest)?);
    }
    Ok(actions)
}

fn scene_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    if c.accept(&TokenKind::Dot) {
        c.expect_kw("cycle")?;
        return scene_cycle(c);
    }
    c.expect(&TokenKind::LParen, "`(` after scene")?;
    let scene = scene_spec(c)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    c.expect(&TokenKind::Dot, "`.recall` or `.apply`")?;
    let (verb, pos) = c.expect_ident("recall or apply")?;
    match verb.as_str() {
        "recall" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let target = if c.accept(&TokenKind::RParen) {
                None
            } else {
                let t = refs::light_target(c)?;
                c.expect(&TokenKind::RParen, "`)`")?;
                Some(t)
            };
            Ok(Action::Scene(SceneAction::Recall { scene, target }))
        }
        "apply" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            c.expect(&TokenKind::RParen, "`)`")?;
            Ok(Action::Scene(SceneAction::Apply { scene }))
        }
        _ => Err(pos.err(format!("unknown scene action \"{verb}\""))),
    }
}

fn scene_spec(c: &mut Cursor<'_>) -> Result<ValueExpr, CompileError> {
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Int(_)) => {
            let (n, _) = c.expect_int_in("scene", 0, i64::from(MAX_SCENE))?;
            Ok(ValueExpr::Literal(n))
        }
        Some(TokenKind::Str(_)) => {
            let (name, pos) = refs::checked_name(c, "scene name")?;
            match c.resolver.resolve_scene(&name) {
                Some(scene) => Ok(ValueExpr::Literal(i64::from(scene))),
                None => Err(pos.err(format!("unknown scene \"{name}\""))),
            }
        }
        _ => expr::value_expr(c),
    }
}

fn scene_cycle(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after cycle")?;
    let mut scenes = Vec::new();
    loop {
        let (scene, pos) = c.expect_int_in("scene", 0, i64::from(MAX_SCENE))?;
        scenes.push(scene as u8);
        if scenes.len() > MAX_SCENE_CYCLE_ENTRIES {
            return Err(pos.err(format!("scene.cycle lists more than {MAX_SCENE_CYCLE_ENTRIES} scenes")));
        }
        if !c.accept(&TokenKind::Comma) {
            break;
        }
    }
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(Action::Scene(SceneAction::Cycle { scenes }))
}

fn hcl_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    c.expect(&TokenKind::Dot, "`.` and an hcl action")?;
    let (verb, pos) = c.expect_ident("an hcl action")?;
    match verb.as_str() {
        "resume" | "hold" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let target = refs::light_target(c)?;
            c.expect(&TokenKind::RParen, "`)`")?;
            Ok(Action::Hcl(if verb == "resume" {
                HclAction::Resume { target }
            } else {
                HclAction::Hold { target }
            }))
        }
        "enable" | "disable" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let (schedule, _) = refs::checked_name(c, "schedule name")?;
            c.expect(&TokenKind::RParen, "`)`")?;
            Ok(Action::Hcl(if verb == "enable" {
                HclAction::Enable { schedule }
            } else {
                HclAction::Disable { schedule }
            }))
        }
        _ => Err(pos.err(format!("unknown hcl action \"{verb}\""))),
    }
}

fn input_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let input = refs::input_ref(c)?;
    c.expect(&TokenKind::Dot, "`.` and an input action")?;
    let (verb, pos) = c.expect_ident("an input action")?;
    match verb.as_str() {
        "feedback" => {
            c.expect(&TokenKind::Dot, "`.on()` or `.off()`")?;
            let (state, pos) = c.expect_ident("on or off")?;
            empty_parens(c)?;
            match state.as_str() {
                "on" => Ok(Action::Input(InputAction::FeedbackOn { input })),
                "off" => Ok(Action::Input(InputAction::FeedbackOff { input })),
                _ => Err(pos.err(format!("expected on or off, got \"{state}\""))),
            }
        }
        "cancel_hold" => {
            empty_parens(c)?;
            Ok(Action::Input(InputAction::CancelHold { input }))
        }
        "catch_movement" => {
            empty_parens(c)?;
            Ok(Action::Input(InputAction::CatchMovement { input }))
        }
        _ => Err(pos.err(format!("unknown input action \"{verb}\""))),
    }
}

fn empty_parens(c: &mut Cursor<'_>) -> Result<(), CompileError> {
    c.expect(&TokenKind::LParen, "`(`")?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(())
}

fn panel_select(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let at = c.expect(&TokenKind::LParen, "`(` after panel_select")?;
    let mut bag = call_args(c)?;
    let group = named_literal(&mut bag, "group", 0, i64::from(MAX_INSTANCE_GROUP), at)?;
    let selected = named_bounded_expr(&mut bag, "selected", 0, i64::from(MAX_INSTANCE_GROUP), at)?;
    let adapter_id = panel_adapter(c, &mut bag)?;
    bag.finish("panel_select")?;
    Ok(Action::Input(InputAction::PanelSelect {
        adapter_id,
        group: group as u8,
        selected,
    }))
}

fn named_literal(
    bag: &mut ArgBag,
    key: &str,
    min: i64,
    max: i64,
    at: crate::lexer::Pos,
) -> Result<i64, CompileError> {
    let Some(arg) = bag.take_named(key) else {
        return Err(at.err(format!("expected {key}=…")));
    };
    match arg.value {
        ArgValue::Expr(ValueExpr::Literal(n)) if (min..=max).contains(&n) => Ok(n),
        ArgValue::Expr(ValueExpr::Literal(n)) => {
            Err(arg.pos.err(format!("{key} must be {min}..={max}, got {n}")))
        }
        _ => Err(arg.pos.err(format!("{key} must be a literal {min}..={max}"))),
    }
}

fn named_bounded_expr(
    bag: &mut ArgBag,
    key: &str,
    min: i64,
    max: i64,
    at: crate::lexer::Pos,
) -> Result<ValueExpr, CompileError> {
    let Some(arg) = bag.take_named(key) else {
        return Err(at.err(format!("expected {key}=…")));
    };
    match arg.value {
        ArgValue::Expr(expr) => {
            if let ValueExpr::Literal(n) = &expr {
                if *n < min || *n > max {
                    return Err(arg.pos.err(format!("{key} must be {min}..={max}, got {n}")));
                }
            }
            Ok(expr)
        }
        _ => Err(arg.pos.err(format!("{key} must be a number or a value"))),
    }
}

fn panel_adapter(c: &Cursor<'_>, bag: &mut ArgBag) -> Result<u8, CompileError> {
    let Some(arg) = bag.take_named("adapter") else {
        return Ok(c.resolver.primary_adapter());
    };
    match arg.value {
        ArgValue::Expr(ValueExpr::Literal(n)) if (0..=255).contains(&n) => {
            let adapter = n as u8;
            if !c.resolver.adapter_exists(adapter) {
                return Err(arg.pos.err(format!("unknown adapter {adapter}")));
            }
            Ok(adapter)
        }
        _ => Err(arg.pos.err("adapter must be a literal 0..=255")),
    }
}

fn wait_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let (ms, pos) = c.expect_duration("wait")?;
    if ms == 0 {
        return Err(pos.err("wait needs a positive duration"));
    }
    Ok(Action::Flow(FlowAction::Wait { duration_ms: DurationMs(ms) }))
}

fn after_action(c: &mut Cursor<'_>, nest: Nesting) -> Result<Action, CompileError> {
    let pos = c.here();
    c.advance();
    if nest.in_after {
        return Err(pos.err("nested after blocks are not allowed (depth 1)"));
    }
    let (ms, dpos) = c.expect_duration("after delay")?;
    if ms == 0 {
        return Err(dpos.err("after needs a positive delay"));
    }
    c.expect_kw("do")?;
    c.expect(&TokenKind::LBrace, "`{`")?;
    let actions = actions_until_rbrace(c, Nesting { in_after: true, ..nest })?;
    Ok(Action::Flow(FlowAction::After { delay_ms: DurationMs(ms), actions }))
}

fn timer_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let timer = quoted_ref(c, "timer name")?;
    c.expect(&TokenKind::Dot, "`.start`, `.restart` or `.cancel`")?;
    let (verb, pos) = c.expect_ident("a timer action")?;
    match verb.as_str() {
        "start" | "restart" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let (ms, _) = c.expect_duration("timer duration")?;
            c.expect(&TokenKind::RParen, "`)`")?;
            let duration_ms = DurationMs(ms);
            Ok(Action::Flow(if verb == "start" {
                FlowAction::TimerStart { timer, duration_ms }
            } else {
                FlowAction::TimerRestart { timer, duration_ms }
            }))
        }
        "cancel" => {
            empty_parens(c)?;
            Ok(Action::Flow(FlowAction::TimerCancel { timer }))
        }
        _ => Err(pos.err(format!("unknown timer action \"{verb}\""))),
    }
}

fn call_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let block = quoted_ref(c, "block name")?;
    Ok(Action::Flow(FlowAction::Call { block }))
}

fn repeat_action(c: &mut Cursor<'_>, nest: Nesting) -> Result<Action, CompileError> {
    let pos = c.here();
    c.advance();
    if nest.in_repeat {
        return Err(pos.err("nested repeat is not allowed"));
    }
    let (count, _) = c.expect_int_in("repeat count", 1, i64::from(MAX_REPEAT))?;
    c.expect(&TokenKind::LBrace, "`{`")?;
    let actions = actions_until_rbrace(c, Nesting { in_repeat: true, ..nest })?;
    Ok(Action::Flow(FlowAction::Repeat { count: count as u8, actions }))
}

fn conditional_action(c: &mut Cursor<'_>, nest: Nesting) -> Result<Action, CompileError> {
    let pos = c.here();
    c.advance();
    if nest.in_conditional {
        return Err(pos.err("nested if/else is not allowed (depth 1)"));
    }
    let condition = condition::condition(c)?;
    c.expect(&TokenKind::LBrace, "`{`")?;
    let inner = Nesting { in_conditional: true, ..nest };
    let then_actions = actions_until_rbrace(c, inner)?;
    let else_actions = if c.accept_kw("else") {
        c.expect(&TokenKind::LBrace, "`{` after else")?;
        actions_until_rbrace(c, inner)?
    } else {
        Vec::new()
    };
    Ok(Action::Flow(FlowAction::Conditional { condition, then_actions, else_actions }))
}

fn var_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let name = quoted_ref(c, "var name")?;
    c.expect(&TokenKind::Dot, "`.set` or `.add`")?;
    let (verb, pos) = c.expect_ident("a var action")?;
    match verb.as_str() {
        "set" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let value = var_value(c)?;
            c.expect(&TokenKind::RParen, "`)`")?;
            Ok(Action::State(StateAction::VarSet { name, value }))
        }
        "add" => {
            c.expect(&TokenKind::LParen, "`(`")?;
            let delta = expr::value_expr(c)?;
            c.expect(&TokenKind::RParen, "`)`")?;
            Ok(Action::State(StateAction::VarAdd { name, delta }))
        }
        _ => Err(pos.err(format!("unknown var action \"{verb}\""))),
    }
}

fn var_value(c: &mut Cursor<'_>) -> Result<VarValue, CompileError> {
    if let Some(TokenKind::Str(_)) = c.peek().map(|t| &t.kind) {
        let (text, pos) = c.expect_string("var value")?;
        if text.len() > MAX_VAR_TEXT_BYTES {
            return Err(pos.err(format!("var text exceeds {MAX_VAR_TEXT_BYTES} bytes")));
        }
        return Ok(VarValue::Text(text));
    }
    match expr::value_expr(c)? {
        ValueExpr::Literal(n) => Ok(VarValue::Int(n)),
        _ => Err(c.err_here("var.set takes a string or an integer literal")),
    }
}

fn rule_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let rule = quoted_ref(c, "rule name")?;
    c.expect(&TokenKind::Dot, "`.enable()` or `.disable()`")?;
    let (verb, pos) = c.expect_ident("enable or disable")?;
    empty_parens(c)?;
    match verb.as_str() {
        "enable" => Ok(Action::State(StateAction::RuleEnable { rule })),
        "disable" => Ok(Action::State(StateAction::RuleDisable { rule })),
        _ => Err(pos.err(format!("unknown rule action \"{verb}\""))),
    }
}

fn mqtt_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    c.expect(&TokenKind::Dot, "`.publish`")?;
    c.expect_kw("publish")?;
    c.expect(&TokenKind::LParen, "`(`")?;
    let (topic, tpos) = c.expect_string("mqtt topic")?;
    if topic.len() > MAX_MQTT_TOPIC_BYTES {
        return Err(tpos.err(format!("mqtt topic exceeds {MAX_MQTT_TOPIC_BYTES} bytes")));
    }
    c.expect(&TokenKind::Comma, "`,` and a payload")?;
    let (payload, ppos) = c.expect_string("mqtt payload")?;
    if payload.len() > MAX_MQTT_PAYLOAD_BYTES {
        return Err(ppos.err(format!("mqtt payload exceeds {MAX_MQTT_PAYLOAD_BYTES} bytes")));
    }
    let retain = mqtt_retain(c)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(Action::State(StateAction::MqttPublish { topic, payload, retain }))
}

fn mqtt_retain(c: &mut Cursor<'_>) -> Result<bool, CompileError> {
    if !c.accept(&TokenKind::Comma) {
        return Ok(false);
    }
    c.expect_kw("retain")?;
    c.expect(&TokenKind::Assign, "`=` after retain")?;
    let (word, pos) = c.expect_ident("true or false")?;
    match word.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(pos.err(format!("expected true or false, got \"{word}\""))),
    }
}

fn log_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    c.expect(&TokenKind::LParen, "`(`")?;
    let (text, _) = c.expect_string("log text")?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(Action::State(StateAction::Log { text }))
}

fn stat_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let name = quoted_ref(c, "stat name")?;
    c.expect(&TokenKind::Dot, "`.count()`")?;
    c.expect_kw("count")?;
    empty_parens(c)?;
    Ok(Action::State(StateAction::StatCount { name }))
}
