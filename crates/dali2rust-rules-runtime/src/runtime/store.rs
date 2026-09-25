use std::sync::RwLock;

use dali2rust_rules_model::RuleSet;

use super::persistence::{record_enabled, RuleManifestEntry};
use super::rule_runtime::{record, retain_for, Firing, RuleRuntime};

#[derive(Debug, Clone, Default)]
pub struct RulesDocument {
    pub source: String,
    pub lang_id: u8,
    pub revision: u32,
    pub compiled: Option<RuleSet>,
    pub diagnostic: Option<String>,
    pub enable_table: Vec<RuleManifestEntry>,
}

#[derive(Debug, Default)]
pub struct RulesStore {
    inner: RwLock<RulesDocument>,
    runtime: RwLock<Vec<RuleRuntime>>,
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

    pub fn rule_runtime(&self) -> Vec<RuleRuntime> {
        self.runtime.read().expect("rules runtime poisoned").clone()
    }

    pub(crate) fn record_firing(&self, name: &str, firing: Firing) {
        record(&mut self.runtime.write().expect("rules runtime poisoned"), name, firing);
    }

    pub(crate) fn retain_runtime(&self, set: Option<&RuleSet>) {
        retain_for(&mut self.runtime.write().expect("rules runtime poisoned"), set);
    }

    pub(crate) fn replace(&self, doc: RulesDocument) {
        *self.inner.write().expect("rules store poisoned") = doc;
    }

    pub(crate) fn set_enabled(&self, name: &str, enabled: bool) -> Option<u32> {
        let mut g = self.inner.write().expect("rules store poisoned");
        let set = g.compiled.as_mut()?;
        set.set_rule_enabled(name, enabled)?;
        record_enabled(&mut g.enable_table, name, enabled);
        g.revision = g.revision.wrapping_add(1);
        Some(g.revision)
    }
}
