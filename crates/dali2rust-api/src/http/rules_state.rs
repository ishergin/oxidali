use dali2rust_rules_model::RuleSet;

#[derive(Debug, Clone, Default)]
pub struct RulesDocumentView {
    pub source: String,
    pub lang_id: u8,
    pub revision: u32,
    pub compiled: Option<RuleSet>,
    pub diagnostic: Option<String>,
}

pub trait RulesHttpState: Send + Sync {
    fn document(&self) -> RulesDocumentView;
    fn revision(&self) -> u32;
}
