pub mod action;
pub mod action_light;
pub mod condition;
pub mod expr;
pub mod refs;
pub mod timeval;
pub mod trigger;

use crate::cursor::Cursor;
use crate::lexer::{lex, Pos, TokenKind};
use action::Nesting;
use action_light::HOLD_HCL_FALSE_REFUSAL;
use dali2rust_rules_model::limits::{
    MAX_ACTIONS_PER_RULE, MAX_CONDITIONS_PER_RULE, MAX_TRIGGERS_PER_RULE,
};
use dali2rust_rules_model::rule::default_cooldown_ms;
use dali2rust_rules_model::{
    Action, CompileError, Condition, DefBlock, ModelError, NameResolver, Rule, RuleSet, Trigger,
};

pub struct SpanIndex {
    names: Vec<(String, Pos)>,
}

impl SpanIndex {
    fn insert(&mut self, name: &str, pos: Pos) {
        self.names.push((name.to_owned(), pos));
    }

    fn find(&self, name: &str) -> Option<Pos> {
        self.names
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, pos)| *pos)
    }

    pub fn locate(&self, error: &ModelError) -> CompileError {
        let pos = error
            .anchor()
            .and_then(|name| self.find(name))
            .unwrap_or(Pos { line: 1, column: 1 });
        pos.err(error.to_string())
    }
}

pub fn parse_document(
    source: &str,
    resolver: &dyn NameResolver,
    lang_id: u8,
) -> Result<(RuleSet, SpanIndex), CompileError> {
    let lexed = lex(source)?;
    let mut c = Cursor::new(lexed, resolver);
    let mut set = RuleSet::empty(lang_id);
    let mut spans = SpanIndex { names: Vec::new() };
    loop {
        if c.peek().is_none() {
            return Ok((set, spans));
        }
        if c.accept_kw("def") {
            let block = def_block(&mut c, &mut spans, &set)?;
            set.blocks.push(block);
        } else if c.accept_kw("rule") {
            let rule = rule_body(&mut c, &mut spans, &set)?;
            set.rules.push(rule);
        } else {
            return Err(c.err_here("expected `rule` or `def`"));
        }
    }
}

fn unique_name(
    c: &mut Cursor<'_>,
    spans: &mut SpanIndex,
    set: &RuleSet,
    what: &str,
    is_rule: bool,
) -> Result<String, CompileError> {
    let (name, pos) = refs::checked_name(c, what)?;
    let duplicate = if is_rule {
        set.rule(&name).is_some()
    } else {
        set.block(&name).is_some()
    };
    if duplicate {
        return Err(pos.err(format!("duplicate {what} \"{name}\"")));
    }
    spans.insert(&name, pos);
    Ok(name)
}

fn def_block(
    c: &mut Cursor<'_>,
    spans: &mut SpanIndex,
    set: &RuleSet,
) -> Result<DefBlock, CompileError> {
    let name = unique_name(c, spans, set, "block name", false)?;
    let open = c.expect(&TokenKind::LBrace, "`{`")?;
    let actions = action::actions_until_rbrace(c, Nesting::default())?;
    if actions.is_empty() {
        return Err(open.err(format!("block \"{name}\" has no actions")));
    }
    Ok(DefBlock { name, actions })
}

struct Modifiers {
    enabled: bool,
    cooldown_ms: Option<u32>,
    hold_hcl: bool,
}

fn modifiers(c: &mut Cursor<'_>) -> Result<Modifiers, CompileError> {
    let mut m = Modifiers { enabled: true, cooldown_ms: None, hold_hcl: true };
    loop {
        if c.accept_kw("enabled") {
            m.enabled = bool_word(c)?;
        } else if c.accept_kw("cooldown") {
            let (ms, _) = c.expect_duration("cooldown")?;
            m.cooldown_ms = Some(ms);
        } else if c.accept_kw("hold_hcl") {
            let pos = c.here();
            if !bool_word(c)? {
                return Err(pos.err(HOLD_HCL_FALSE_REFUSAL));
            }
            m.hold_hcl = true;
        } else {
            return Ok(m);
        }
    }
}

fn bool_word(c: &mut Cursor<'_>) -> Result<bool, CompileError> {
    let (word, pos) = c.expect_ident("true or false")?;
    match word.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(pos.err(format!("expected true or false, got \"{word}\""))),
    }
}

fn rule_body(
    c: &mut Cursor<'_>,
    spans: &mut SpanIndex,
    set: &RuleSet,
) -> Result<Rule, CompileError> {
    let name = unique_name(c, spans, set, "rule name", true)?;
    let m = modifiers(c)?;
    c.expect(&TokenKind::LBrace, "`{`")?;
    let triggers = when_section(c, &name)?;
    let conditions = if_section(c, &name)?;
    let actions = do_section(c, &name)?;
    c.expect(&TokenKind::RBrace, "`}`")?;
    Ok(Rule {
        cooldown_ms: m.cooldown_ms.unwrap_or_else(|| default_cooldown_ms(&triggers)),
        name,
        enabled: m.enabled,
        hold_hcl: m.hold_hcl,
        triggers,
        conditions,
        actions,
    })
}

fn when_section(c: &mut Cursor<'_>, rule: &str) -> Result<Vec<Trigger>, CompileError> {
    let mut triggers = Vec::new();
    loop {
        let pos = c.here();
        if !c.accept_kw("when") {
            break;
        }
        if triggers.len() == MAX_TRIGGERS_PER_RULE {
            return Err(pos.err(format!(
                "rule \"{rule}\" has more than {MAX_TRIGGERS_PER_RULE} `when` triggers"
            )));
        }
        triggers.push(trigger::trigger(c)?);
    }
    if triggers.is_empty() {
        return Err(c.err_here(format!("rule \"{rule}\" needs a `when` trigger")));
    }
    Ok(triggers)
}

fn if_section(c: &mut Cursor<'_>, rule: &str) -> Result<Vec<Condition>, CompileError> {
    let mut conditions = Vec::new();
    if !c.accept_kw("if") {
        return Ok(conditions);
    }
    conditions.push(condition::condition(c)?);
    loop {
        let pos = c.here();
        if !c.accept_kw("and") {
            return Ok(conditions);
        }
        if conditions.len() == MAX_CONDITIONS_PER_RULE {
            return Err(pos.err(format!(
                "rule \"{rule}\" has more than {MAX_CONDITIONS_PER_RULE} conditions"
            )));
        }
        conditions.push(condition::condition(c)?);
    }
}

fn do_section(c: &mut Cursor<'_>, rule: &str) -> Result<Vec<Action>, CompileError> {
    c.expect_kw("do")?;
    let mut actions = Vec::new();
    while !matches!(c.peek().map(|t| &t.kind), Some(TokenKind::RBrace) | None) {
        let pos = c.here();
        if actions.len() == MAX_ACTIONS_PER_RULE {
            return Err(pos.err(format!(
                "rule \"{rule}\" has more than {MAX_ACTIONS_PER_RULE} actions"
            )));
        }
        actions.push(action::action(c, Nesting::default())?);
    }
    if actions.is_empty() {
        return Err(c.err_here(format!("rule \"{rule}\" needs at least one action")));
    }
    Ok(actions)
}
