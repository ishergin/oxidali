use crate::cursor::Cursor;
use crate::lexer::TokenKind;
use crate::parser::refs;
use dali2rust_rules_model::{CompileError, Reading, ValueExpr, ValueKind};

pub const PARSED_VALUE_KINDS: &[ValueKind] = &ValueKind::ALL;

pub fn starts_reading(c: &Cursor<'_>) -> bool {
    matches!(
        c.peek().map(|t| &t.kind),
        Some(TokenKind::Ident(word))
            if matches!(word.as_str(), "lamp" | "group" | "input" | "time" | "sun" | "var" | "event")
    )
}

fn lamp_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    let lamp = refs::lamp_ref(c)?;
    c.expect(&TokenKind::Dot, "`.` and a lamp property")?;
    let (prop, pos) = c.expect_ident("lamp property")?;
    match prop.as_str() {
        "level" => Ok(Reading::LampLevel { lamp }),
        "cct" => Ok(Reading::LampCct { lamp }),
        "is_on" => Ok(Reading::LampIsOn { lamp }),
        "last_level" => Ok(Reading::LampLastLevel { lamp }),
        _ => Err(pos.err(format!("unknown lamp property \"{prop}\""))),
    }
}

fn group_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    let group = refs::group_ref(c)?;
    c.expect(&TokenKind::Dot, "`.` and a group property")?;
    let (prop, pos) = c.expect_ident("group property")?;
    match prop.as_str() {
        "any_on" => Ok(Reading::GroupAnyOn { group }),
        "all_off" => Ok(Reading::GroupAllOff { group }),
        "member_count" => Ok(Reading::GroupMemberCount { group }),
        _ => Err(pos.err(format!("unknown group property \"{prop}\""))),
    }
}

fn input_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    let input = refs::input_ref(c)?;
    c.expect(&TokenKind::Dot, "`.` and an input property")?;
    let (prop, pos) = c.expect_ident("input property")?;
    match prop.as_str() {
        "occupied" => Ok(Reading::InputOccupied { input }),
        "light" => Ok(Reading::InputLight { input }),
        "position" => Ok(Reading::InputPosition { input }),
        "last_event_age" => Ok(Reading::InputLastEventAge { input }),
        _ => Err(pos.err(format!("unknown input property \"{prop}\""))),
    }
}

fn time_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    c.expect(&TokenKind::Dot, "`.` and a time property")?;
    let (prop, pos) = c.expect_ident("time property")?;
    match prop.as_str() {
        "now" => Ok(Reading::TimeNow),
        "hour" => Ok(Reading::TimeHour),
        "minute" => Ok(Reading::TimeMinute),
        "weekday" => Ok(Reading::TimeWeekday),
        _ => Err(pos.err(format!("unknown time property \"{prop}\""))),
    }
}

fn sun_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    c.expect(&TokenKind::Dot, "`.` and a sun property")?;
    let (prop, pos) = c.expect_ident("sun property")?;
    match prop.as_str() {
        "rise" => Ok(Reading::SunRise),
        "set" => Ok(Reading::SunSet),
        "is_up" => Ok(Reading::SunIsUp),
        _ => Err(pos.err(format!("unknown sun property \"{prop}\""))),
    }
}

fn event_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    c.expect(&TokenKind::Dot, "`.` and an event property")?;
    let (prop, pos) = c.expect_ident("event property")?;
    match prop.as_str() {
        "value" => Ok(Reading::EventValue),
        "device" => Ok(Reading::EventDevice),
        "instance" => Ok(Reading::EventInstance),
        "option" => Ok(Reading::EventOption),
        _ => Err(pos.err(format!("unknown event property \"{prop}\""))),
    }
}

fn var_reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after var")?;
    let (name, _) = refs::checked_name(c, "var name")?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(Reading::Var { name })
}

pub fn reading(c: &mut Cursor<'_>) -> Result<Reading, CompileError> {
    let (family, pos) = c.expect_ident("a value")?;
    match family.as_str() {
        "lamp" => lamp_reading(c),
        "group" => group_reading(c),
        "input" => input_reading(c),
        "time" => time_reading(c),
        "sun" => sun_reading(c),
        "var" => var_reading(c),
        "event" => event_reading(c),
        _ => Err(pos.err(format!("expected a value, got \"{family}\""))),
    }
}

fn signed_literal(c: &mut Cursor<'_>) -> Result<i64, CompileError> {
    let negative = if c.accept(&TokenKind::Minus) {
        true
    } else {
        c.accept(&TokenKind::Plus);
        false
    };
    let (value, _) = c.expect_int("number")?;
    Ok(if negative { -value } else { value })
}

fn offset_after_reading(c: &mut Cursor<'_>, reading: Reading) -> Result<ValueExpr, CompileError> {
    let sign = if c.accept(&TokenKind::Plus) {
        1i64
    } else if c.accept(&TokenKind::Minus) {
        -1i64
    } else {
        return Ok(ValueExpr::Reading(reading));
    };
    let (value, pos) = c.expect_int("offset literal")?;
    let delta = i32::try_from(sign * value)
        .map_err(|_| pos.err(format!("offset {value} out of range")))?;
    Ok(ValueExpr::Offset { reading, delta })
}

pub fn value_expr(c: &mut Cursor<'_>) -> Result<ValueExpr, CompileError> {
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Int(_)) | Some(TokenKind::Plus) | Some(TokenKind::Minus) => {
            Ok(ValueExpr::Literal(signed_literal(c)?))
        }
        _ if starts_reading(c) => {
            let r = reading(c)?;
            offset_after_reading(c, r)
        }
        _ => Err(c.err_here("expected a number or a value")),
    }
}
