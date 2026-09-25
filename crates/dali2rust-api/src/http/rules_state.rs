use dali2rust_rules_model::RuleSet;

#[derive(Debug, Clone, Default)]
pub struct RulesDocumentView {
    pub source: String,
    pub lang_id: u8,
    pub revision: u32,
    pub compiled: Option<RuleSet>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRuntimeView {
    pub name: String,
    pub fire_count: u32,
    pub last_fired_at_ms: u64,
    pub last_latency_ms: u16,
    pub last_outcome: &'static str,
    pub last_error: Option<&'static str>,
}

pub trait RulesHttpState: Send + Sync {
    fn document(&self) -> RulesDocumentView;
    fn revision(&self) -> u32;
    fn rule_runtime(&self) -> Vec<RuleRuntimeView>;
}
