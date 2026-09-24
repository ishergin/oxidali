use crate::action::Action;
use crate::condition::Condition;
use crate::trigger::Trigger;
use serde::Serialize;

pub const DEFAULT_COOLDOWN_INPUT_MS: u32 = 0;
pub const DEFAULT_COOLDOWN_STATE_MS: u32 = 200;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DefBlock {
    pub name: String,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub cooldown_ms: u32,
    pub hold_hcl: bool,
    pub triggers: Vec<Trigger>,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

pub fn default_cooldown_ms(triggers: &[Trigger]) -> u32 {
    if triggers.iter().any(|t| t.kind().is_input_class()) {
        DEFAULT_COOLDOWN_INPUT_MS
    } else {
        DEFAULT_COOLDOWN_STATE_MS
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleSet {
    pub lang_id: u8,
    pub blocks: Vec<DefBlock>,
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn empty(lang_id: u8) -> RuleSet {
        RuleSet {
            lang_id,
            blocks: Vec::new(),
            rules: Vec::new(),
        }
    }

    pub fn block(&self, name: &str) -> Option<&DefBlock> {
        self.blocks.iter().find(|b| b.name == name)
    }

    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    pub fn set_rule_enabled(&mut self, name: &str, enabled: bool) -> Option<()> {
        let rule = self.rules.iter_mut().find(|r| r.name == name)?;
        rule.enabled = enabled;
        Some(())
    }
}
