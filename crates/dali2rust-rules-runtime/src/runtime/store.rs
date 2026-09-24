use std::sync::RwLock;

use dali2rust_rules_model::RuleSet;

#[derive(Debug, Clone, Default)]
pub struct RulesDocument {
    pub source: String,
    pub lang_id: u8,
    pub revision: u32,
    pub compiled: Option<RuleSet>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Default)]
pub struct RulesStore {
    inner: RwLock<RulesDocument>,
}

impl RulesStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn document(&self) -> RulesDocument {
        self.inner.read().expect("rules store poisoned").clone()
    }

    pub fn revision(&self) -> u32 {
        self.inner.read().expect("rules store poisoned").revision
    }

    pub(crate) fn replace(&self, doc: RulesDocument) {
        *self.inner.write().expect("rules store poisoned") = doc;
    }

    pub(crate) fn set_enabled(&self, name: &str, enabled: bool) -> Option<u32> {
        let mut g = self.inner.write().expect("rules store poisoned");
        let set = g.compiled.as_mut()?;
        set.set_rule_enabled(name, enabled)?;
        g.revision = g.revision.wrapping_add(1);
        Some(g.revision)
    }
}
