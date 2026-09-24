use crate::cursor::Cursor;
use crate::lexer::{Pos, TokenKind};
use crate::parser::expr;
use crate::parser::refs;
use dali2rust_rules_model::limits::MAX_LEVEL;
use dali2rust_rules_model::{
    Action, CctSpec, CompileError, LevelSpec, LightAction, LightOp, ValueExpr,
};

const MAX_XY_1E4: u32 = 10_000;
const MAX_RGB: i64 = 255;
const MAX_CCT_K: i64 = 65_535;
const MAX_STEP: i64 = MAX_LEVEL as i64;

pub struct CallArg {
    pub name: Option<String>,
    pub value: ArgValue,
    pub pos: Pos,
}

pub enum ArgValue {
    Expr(ValueExpr),
    Signed(i64),
    Duration(u32),
    Bool(bool),
    Decimal1e4(u32),
}

fn arg_value(c: &mut Cursor<'_>) -> Result<ArgValue, CompileError> {
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Plus) | Some(TokenKind::Minus) => signed_value(c),
        Some(TokenKind::Int(_)) => {
            let (n, _) = c.expect_int("number")?;
            Ok(ArgValue::Expr(ValueExpr::Literal(n)))
        }
        Some(TokenKind::Duration(_)) => {
            let (ms, _) = c.expect_duration("duration")?;
            Ok(ArgValue::Duration(ms))
        }
        Some(TokenKind::Decimal1e4(v)) => {
            let v = *v;
            c.advance();
            Ok(ArgValue::Decimal1e4(v))
        }
        Some(TokenKind::Ident(word)) if word == "true" || word == "false" => {
            let b = word == "true";
            c.advance();
            Ok(ArgValue::Bool(b))
        }
        _ if expr::starts_reading(c) => Ok(ArgValue::Expr(expr::value_expr(c)?)),
        _ => Err(c.err_here("expected an argument value")),
    }
}

fn signed_value(c: &mut Cursor<'_>) -> Result<ArgValue, CompileError> {
    let negative = matches!(c.peek().map(|t| &t.kind), Some(TokenKind::Minus));
    c.advance();
    let (value, _) = c.expect_int("number after sign")?;
    Ok(ArgValue::Signed(if negative { -value } else { value }))
}

pub fn call_args(c: &mut Cursor<'_>) -> Result<ArgBag, CompileError> {
    let mut args = Vec::new();
    if !c.accept(&TokenKind::RParen) {
        loop {
            args.push(one_arg(c)?);
            if !c.accept(&TokenKind::Comma) {
                break;
            }
        }
        c.expect(&TokenKind::RParen, "`)`")?;
    }
    Ok(ArgBag { args })
}

fn one_arg(c: &mut Cursor<'_>) -> Result<CallArg, CompileError> {
    let pos = c.here();
    let named = match (c.peek().map(|t| &t.kind), c.peek_second().map(|t| &t.kind)) {
        (Some(TokenKind::Ident(key)), Some(TokenKind::Assign)) => Some(key.clone()),
        _ => None,
    };
    if let Some(key) = named {
        c.advance();
        c.advance();
        let value = arg_value(c)?;
        return Ok(CallArg { name: Some(key), value, pos });
    }
    Ok(CallArg { name: None, value: arg_value(c)?, pos })
}

pub struct ArgBag {
    args: Vec<CallArg>,
}

impl ArgBag {
    pub fn take_named(&mut self, key: &str) -> Option<CallArg> {
        let i = self.args.iter().position(|a| a.name.as_deref() == Some(key))?;
        Some(self.args.remove(i))
    }

    pub fn take_positional(&mut self) -> Option<CallArg> {
        let i = self.args.iter().position(|a| a.name.is_none())?;
        Some(self.args.remove(i))
    }

    pub fn finish(self, context: &str) -> Result<(), CompileError> {
        match self.args.first() {
            None => Ok(()),
            Some(arg) => {
                let what = arg.name.as_deref().unwrap_or("positional argument");
                Err(arg.pos.err(format!("unexpected {what} for {context}")))
            }
        }
    }
}

fn expr_of(arg: CallArg, what: &str) -> Result<(ValueExpr, Pos), CompileError> {
    match arg.value {
        ArgValue::Expr(expr) => Ok((expr, arg.pos)),
        _ => Err(arg.pos.err(format!("{what} must be a number or a value"))),
    }
}

fn bounded_expr(arg: CallArg, what: &str, min: i64, max: i64) -> Result<ValueExpr, CompileError> {
    let (expr, pos) = expr_of(arg, what)?;
    if let ValueExpr::Literal(n) = &expr {
        if *n < min || *n > max {
            return Err(pos.err(format!("{what} must be {min}..={max}, got {n}")));
        }
    }
    Ok(expr)
}

fn literal_in(arg: CallArg, what: &str, min: i64, max: i64) -> Result<(i64, Pos), CompileError> {
    let pos = arg.pos;
    match arg.value {
        ArgValue::Expr(ValueExpr::Literal(n)) if (min..=max).contains(&n) => Ok((n, pos)),
        ArgValue::Expr(ValueExpr::Literal(n)) => {
            Err(pos.err(format!("{what} must be {min}..={max}, got {n}")))
        }
        _ => Err(pos.err(format!("{what} must be a literal {min}..={max}"))),
    }
}

fn signed_step(arg: Option<CallArg>, verb: &str, at: Pos) -> Result<i16, CompileError> {
    let Some(arg) = arg else {
        return Err(at.err(format!("{verb} needs a signed step like +8 or -8")));
    };
    match arg.value {
        ArgValue::Signed(n) if n != 0 && n.abs() <= MAX_STEP => Ok(n as i16),
        ArgValue::Signed(n) => Err(arg.pos.err(format!("step {n} out of range ±1..=254"))),
        _ => Err(arg.pos.err(format!("{verb} needs a signed step like +8 or -8"))),
    }
}

fn optional_level(bag: &mut ArgBag) -> Result<Option<ValueExpr>, CompileError> {
    let arg = bag.take_named("level").or_else(|| bag.take_positional());
    match arg {
        None => Ok(None),
        Some(arg) => Ok(Some(bounded_expr(arg, "level", 0, i64::from(MAX_LEVEL))?)),
    }
}

fn reject_fade(bag: &mut ArgBag) -> Result<(), CompileError> {
    match bag.take_named("fade") {
        None => Ok(()),
        Some(arg) => Err(arg.pos.err(
            "fade is not settable per command: the duration lives in the gear \
             (IEC 62386-102 §9.5). Write it once with \
             POST /api/v1/adapters/{id}/physical-devices/{sa}/write-attributes \
             (common_102.fade_time_ms) and every command to that gear uses it",
        )),
    }
}

pub(crate) const HOLD_HCL_FALSE_REFUSAL: &str =
    "hold_hcl=false is not implemented: the flag has no carrier to the \
     registry's commit_source, so a light write from a rule always suspends \
     the schedule for that target (ISSUE-96). Drop the modifier — the default \
     hold_hcl=true is what happens";

fn hold_hcl_of(bag: &mut ArgBag) -> Result<Option<bool>, CompileError> {
    match bag.take_named("hold_hcl") {
        None => Ok(None),
        Some(CallArg { value: ArgValue::Bool(true), .. }) => Ok(Some(true)),
        Some(CallArg { value: ArgValue::Bool(false), pos, .. }) => {
            Err(pos.err(HOLD_HCL_FALSE_REFUSAL))
        }
        Some(arg) => Err(arg.pos.err("hold_hcl must be true or false")),
    }
}

fn level_spec(bag: &mut ArgBag, at: Pos) -> Result<LevelSpec, CompileError> {
    let Some(arg) = bag.take_named("level").or_else(|| bag.take_positional()) else {
        return Err(at.err("level needs an argument: 0..254, ±N, or a value"));
    };
    if let ArgValue::Signed(_) = arg.value {
        let delta = signed_step(Some(arg), ".level", at)?;
        return Ok(LevelSpec::Relative { delta });
    }
    Ok(LevelSpec::Absolute(bounded_expr(arg, "level", 0, i64::from(MAX_LEVEL))?))
}

fn cct_spec(bag: &mut ArgBag, at: Pos) -> Result<CctSpec, CompileError> {
    let Some(arg) = bag.take_positional() else {
        return Err(at.err("cct needs kelvins: 2700, ±K, or a value"));
    };
    if let ArgValue::Signed(n) = arg.value {
        if n == 0 || n.abs() > MAX_CCT_K {
            return Err(arg.pos.err(format!("cct step {n} out of range")));
        }
        return Ok(CctSpec::Relative { delta_k: n as i32 });
    }
    Ok(CctSpec::Absolute(bounded_expr(arg, "cct", 1, MAX_CCT_K)?))
}

fn xy_coord(bag: &mut ArgBag, what: &str, at: Pos) -> Result<u16, CompileError> {
    let Some(arg) = bag.take_positional() else {
        return Err(at.err(format!("xy needs two coordinates; missing {what}")));
    };
    let (value, pos) = match arg.value {
        ArgValue::Decimal1e4(v) => (v, arg.pos),
        ArgValue::Expr(ValueExpr::Literal(n)) if n == 0 || n == 1 => {
            ((n as u32) * MAX_XY_1E4, arg.pos)
        }
        _ => return Err(arg.pos.err(format!("{what} must be a decimal 0..=1.0"))),
    };
    if value > MAX_XY_1E4 {
        return Err(pos.err(format!("{what} must be 0..=1.0")));
    }
    Ok(value as u16)
}

fn rgb_channel(bag: &mut ArgBag, what: &str, at: Pos) -> Result<u8, CompileError> {
    let Some(arg) = bag.take_positional() else {
        return Err(at.err(format!("rgb needs three channels; missing {what}")));
    };
    let (n, _) = literal_in(arg, what, 0, MAX_RGB)?;
    Ok(n as u8)
}

fn light_op(verb: &str, pos: Pos, bag: &mut ArgBag) -> Result<LightOp, CompileError> {
    match verb {
        "on" => Ok(LightOp::On { level: optional_level(bag)? }),
        "off" => Ok(LightOp::Off),
        "toggle" => Ok(LightOp::Toggle { level: optional_level(bag)? }),
        "level" => Ok(LightOp::Level { level: level_spec(bag, pos)? }),
        "dim" => Ok(LightOp::Dim { delta: signed_step(bag.take_positional(), ".dim", pos)? }),
        "dim_hold" => Ok(LightOp::DimHold {
            rate_per_s: signed_step(bag.take_positional(), ".dim_hold", pos)?,
        }),
        "cct" => Ok(LightOp::Cct { cct: cct_spec(bag, pos)? }),
        "xy" => Ok(LightOp::Xy {
            x_1e4: xy_coord(bag, "x", pos)?,
            y_1e4: xy_coord(bag, "y", pos)?,
        }),
        "rgb" => Ok(LightOp::Rgb {
            r: rgb_channel(bag, "r", pos)?,
            g: rgb_channel(bag, "g", pos)?,
            b: rgb_channel(bag, "b", pos)?,
        }),
        "last_active" => Ok(LightOp::LastActive),
        "stop_fade" => Ok(LightOp::StopFade),
        _ => Err(pos.err(format!("unknown light action \"{verb}\""))),
    }
}

pub fn light_action(c: &mut Cursor<'_>) -> Result<Action, CompileError> {
    let target = refs::light_target(c)?;
    c.expect(&TokenKind::Dot, "`.` and a light action")?;
    let (verb, pos) = c.expect_ident("a light action")?;
    c.expect(&TokenKind::LParen, "`(`")?;
    let mut bag = call_args(c)?;
    let op = light_op(&verb, pos, &mut bag)?;
    reject_fade(&mut bag)?;
    let hold_hcl = hold_hcl_of(&mut bag)?;
    bag.finish(&format!(".{verb}"))?;
    Ok(Action::Light(LightAction { op, target, hold_hcl }))
}
