use crate::cursor::Cursor;
use crate::lexer::TokenKind;
use crate::parser::expr;
use crate::parser::refs;
use crate::parser::timeval;
use crate::parser::trigger::{group_aggregate, quoted_ref};
use dali2rust_rules_model::limits::MAX_VAR_TEXT_BYTES;
use dali2rust_rules_model::{
    Cmp, CompileError, Condition, ConditionKind, DurationMs, HclState, VarOperand,
};

pub const PARSED_CONDITION_KINDS: &[ConditionKind] = &[
    ConditionKind::TimeInRange,
    ConditionKind::DayIn,
    ConditionKind::SunIs,
    ConditionKind::LampIs,
    ConditionKind::LampLevel,
    ConditionKind::LampCct,
    ConditionKind::GroupState,
    ConditionKind::InputOccupancy,
    ConditionKind::InputLight,
    ConditionKind::HclState,
    ConditionKind::DeviceOnline,
    ConditionKind::RuleEnabled,
    ConditionKind::LastFiredOlderThan,
    ConditionKind::TimerRunning,
    ConditionKind::ControllerActive,
    ConditionKind::VarCompare,
];

pub fn condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let (word, pos) = c.expect_ident("a condition")?;
    match word.as_str() {
        "time" => time_condition(c),
        "day" => day_condition(c),
        "sun" => sun_condition(c),
        "lamp" => lamp_condition(c),
        "group" => group_condition(c),
        "input" => input_condition(c),
        "hcl" => hcl_condition(c),
        "device" => device_condition(c),
        "rule" => rule_condition(c),
        "last_fired" => last_fired_condition(c),
        "timer" => timer_condition(c),
        "controller" => controller_condition(c),
        "var" => var_condition(c),
        _ => Err(pos.err(format!("unknown condition \"{word}\""))),
    }
}

fn time_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    c.expect_kw("in")?;
    let start = timeval::time_bound(c)?;
    c.expect(&TokenKind::DotDot, "`..` between time bounds")?;
    let end = timeval::time_bound(c)?;
    Ok(Condition::TimeInRange { start, end })
}

fn day_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    c.expect_kw("in")?;
    Ok(Condition::DayIn { days: timeval::day_set(c)? })
}

fn sun_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    c.expect_kw("is")?;
    let (word, pos) = c.expect_ident("up or down")?;
    match word.as_str() {
        "up" => Ok(Condition::SunIs { up: true }),
        "down" => Ok(Condition::SunIs { up: false }),
        _ => Err(pos.err(format!("expected up or down, got \"{word}\""))),
    }
}

fn lamp_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let lamp = refs::lamp_ref(c)?;
    if c.accept_kw("is") {
        let (word, pos) = c.expect_ident("on or off")?;
        return match word.as_str() {
            "on" => Ok(Condition::LampIs { lamp, on: true }),
            "off" => Ok(Condition::LampIs { lamp, on: false }),
            _ => Err(pos.err(format!("expected on or off, got \"{word}\""))),
        };
    }
    c.expect(&TokenKind::Dot, "`is` or a lamp property")?;
    let (prop, pos) = c.expect_ident("level or cct")?;
    let cmp = comparison(c)?;
    let value = expr::value_expr(c)?;
    match prop.as_str() {
        "level" => Ok(Condition::LampLevel { lamp, cmp, value }),
        "cct" => Ok(Condition::LampCct { lamp, cmp, value }),
        _ => Err(pos.err(format!("unknown lamp property \"{prop}\" in a condition"))),
    }
}

fn group_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let group = refs::group_ref(c)?;
    let state = group_aggregate(c)?;
    Ok(Condition::GroupState { group, state })
}

fn input_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let input = refs::input_ref(c)?;
    if c.accept_kw("is") {
        let (word, pos) = c.expect_ident("occupied or vacant")?;
        return match word.as_str() {
            "occupied" => Ok(Condition::InputOccupancy { input, occupied: true }),
            "vacant" => Ok(Condition::InputOccupancy { input, occupied: false }),
            _ => Err(pos.err(format!("expected occupied or vacant, got \"{word}\""))),
        };
    }
    c.expect(&TokenKind::Dot, "`is` or `.light`")?;
    c.expect_kw("light")?;
    let (word, pos) = c.expect_ident("above or below")?;
    let above = match word.as_str() {
        "above" => true,
        "below" => false,
        _ => return Err(pos.err(format!("expected above or below, got \"{word}\""))),
    };
    let threshold = expr::value_expr(c)?;
    Ok(Condition::InputLight { input, above, threshold })
}

fn hcl_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    c.expect_kw("is")?;
    let (word, pos) = c.expect_ident("enabled or overridden")?;
    let state = match word.as_str() {
        "enabled" => HclState::Enabled,
        "overridden" => HclState::Overridden,
        _ => return Err(pos.err(format!("expected enabled or overridden, got \"{word}\""))),
    };
    c.expect_kw("for")?;
    let target = refs::light_target(c)?;
    Ok(Condition::HclState { target, state })
}

fn device_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let device = refs::device_ref(c)?;
    c.expect_kw("is")?;
    c.expect_kw("online")?;
    Ok(Condition::DeviceOnline { device })
}

fn rule_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let rule = quoted_ref(c, "rule name")?;
    c.expect_kw("is")?;
    c.expect_kw("enabled")?;
    Ok(Condition::RuleEnabled { rule })
}

fn last_fired_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let rule = quoted_ref(c, "rule name")?;
    c.expect_kw("older")?;
    c.expect_kw("than")?;
    let (ms, _) = c.expect_duration("age")?;
    Ok(Condition::LastFiredOlderThan { rule, than_ms: DurationMs(ms) })
}

fn timer_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let timer = quoted_ref(c, "timer name")?;
    c.expect_kw("is")?;
    c.expect_kw("running")?;
    Ok(Condition::TimerRunning { timer })
}

fn controller_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    c.expect_kw("is")?;
    c.expect_kw("active")?;
    Ok(Condition::ControllerActive)
}

fn var_condition(c: &mut Cursor<'_>) -> Result<Condition, CompileError> {
    let name = quoted_ref(c, "var name")?;
    let cmp = comparison(c)?;
    let value = var_operand(c)?;
    Ok(Condition::VarCompare { name, cmp, value })
}

fn var_operand(c: &mut Cursor<'_>) -> Result<VarOperand, CompileError> {
    if let Some(TokenKind::Str(_)) = c.peek().map(|t| &t.kind) {
        let (text, pos) = c.expect_string("var value")?;
        if text.len() > MAX_VAR_TEXT_BYTES {
            return Err(pos.err(format!("var text exceeds {MAX_VAR_TEXT_BYTES} bytes")));
        }
        return Ok(VarOperand::Text(text));
    }
    Ok(VarOperand::Value(expr::value_expr(c)?))
}

pub(crate) fn comparison(c: &mut Cursor<'_>) -> Result<Cmp, CompileError> {
    let pos = c.here();
    let cmp = match c.peek().map(|t| &t.kind) {
        Some(TokenKind::EqEq) => Cmp::Eq,
        Some(TokenKind::Ne) => Cmp::Ne,
        Some(TokenKind::Ge) => Cmp::Ge,
        Some(TokenKind::Le) => Cmp::Le,
        Some(TokenKind::Gt) => Cmp::Gt,
        Some(TokenKind::Lt) => Cmp::Lt,
        _ => return Err(pos.err("expected a comparison (==, !=, >=, <=, >, <)")),
    };
    c.advance();
    Ok(cmp)
}
