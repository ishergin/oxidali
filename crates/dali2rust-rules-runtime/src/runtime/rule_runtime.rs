use dali2rust_rules_model::limits::MAX_RULES;
use dali2rust_rules_model::RuleSet;

use super::engine::PartialReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOutcome {
    Ok,
    Partial,
    Failed,
    Refused,
}

impl RuleOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RuleOutcome::Ok => "ok",
            RuleOutcome::Partial => "partial",
            RuleOutcome::Failed => "failed",
            RuleOutcome::Refused => "refused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRuntime {
    pub name: String,
    pub fire_count: u32,
    pub last_fired_at_ms: u64,
    pub last_latency_ms: u16,
    pub last_outcome: RuleOutcome,
    pub last_error: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Firing {
    pub at_ms: u64,
    pub latency_ms: u16,
    pub outcome: RuleOutcome,
    pub error: Option<&'static str>,
}

pub(crate) fn classify(
    partial: Option<PartialReason>,
    executed: u8,
    failed: u8,
) -> (RuleOutcome, Option<&'static str>) {
    match partial {
        Some(PartialReason::ChainDepth) => (RuleOutcome::Refused, Some("chain_depth_exceeded")),
        Some(PartialReason::ConditionUnevaluable) => {
            (RuleOutcome::Refused, Some("condition_unevaluable"))
        }
        Some(PartialReason::EffectBudget) => (RuleOutcome::Partial, Some("effect_budget")),
        None if failed > 0 && executed == 0 => (RuleOutcome::Failed, Some("action_failed")),
        None if failed > 0 => (RuleOutcome::Partial, Some("action_failed")),
        None => (RuleOutcome::Ok, None),
    }
}

pub(crate) fn record(table: &mut Vec<RuleRuntime>, name: &str, firing: Firing) {
    let row = match table.iter().position(|row| row.name == name) {
        Some(index) => &mut table[index],
        None if table.len() < MAX_RULES => {
            table.push(RuleRuntime {
                name: name.to_owned(),
                fire_count: 0,
                last_fired_at_ms: 0,
                last_latency_ms: 0,
                last_outcome: RuleOutcome::Ok,
                last_error: None,
            });
            table.last_mut().expect("row just pushed")
        }
        None => return,
    };
    row.fire_count = row.fire_count.wrapping_add(1);
    row.last_fired_at_ms = firing.at_ms;
    row.last_latency_ms = firing.latency_ms;
    row.last_outcome = firing.outcome;
    row.last_error = firing.error;
}

pub(crate) fn retain_for(table: &mut Vec<RuleRuntime>, set: Option<&RuleSet>) {
    table.retain(|row| set.is_some_and(|set| set.rules.iter().any(|r| r.name == row.name)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_run_is_ok_and_names_no_error() {
        assert_eq!(classify(None, 2, 0), (RuleOutcome::Ok, None));
    }

    #[test]
    fn a_run_with_some_failed_actions_is_partial_and_all_failed_is_failed() {
        assert_eq!(classify(None, 1, 1).0, RuleOutcome::Partial);
        assert_eq!(classify(None, 0, 2).0, RuleOutcome::Failed);
    }

    #[test]
    fn an_engine_refusal_is_refused_with_its_reason() {
        assert_eq!(
            classify(Some(PartialReason::ChainDepth), 0, 0),
            (RuleOutcome::Refused, Some("chain_depth_exceeded"))
        );
        assert_eq!(
            classify(Some(PartialReason::ConditionUnevaluable), 0, 0).0,
            RuleOutcome::Refused
        );
        assert_eq!(
            classify(Some(PartialReason::EffectBudget), 3, 0),
            (RuleOutcome::Partial, Some("effect_budget"))
        );
    }

    #[test]
    fn a_second_firing_counts_and_replaces_the_last_outcome() {
        let mut table = Vec::new();
        let ok = Firing { at_ms: 10, latency_ms: 3, outcome: RuleOutcome::Ok, error: None };
        record(&mut table, "a", ok);
        let failed = Firing {
            at_ms: 20,
            latency_ms: 5,
            outcome: RuleOutcome::Failed,
            error: Some("action_failed"),
        };
        record(&mut table, "a", failed);
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].fire_count, 2);
        assert_eq!(table[0].last_fired_at_ms, 20);
        assert_eq!(table[0].last_outcome, RuleOutcome::Failed);
        assert_eq!(table[0].last_error, Some("action_failed"));
    }

    #[test]
    fn the_table_never_grows_past_the_rule_limit() {
        let mut table = Vec::new();
        let ok = Firing { at_ms: 1, latency_ms: 0, outcome: RuleOutcome::Ok, error: None };
        for i in 0..=MAX_RULES {
            record(&mut table, &format!("r{i}"), ok);
        }
        assert_eq!(table.len(), MAX_RULES);
    }
}
