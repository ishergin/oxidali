use dali2rust_contracts::msg::HclTargetScope;
use dali2rust_domain::registry::{HclOverrideReadPort, HclOverrideTargetView, HclOverrideView};

use super::overrides::SuspendedTarget;
use super::scheduler_worker::{lock_ledger, SharedOverrideLedger};

pub struct HclOverrideLedgerRead {
    ledger: SharedOverrideLedger,
}

impl HclOverrideLedgerRead {
    pub fn new(ledger: SharedOverrideLedger) -> Self {
        Self { ledger }
    }
}

impl HclOverrideReadPort for HclOverrideLedgerRead {
    fn hcl_override_view(&self, schedule_id: &str) -> HclOverrideView {
        let rows = lock_ledger(&self.ledger).suspended_targets(schedule_id);
        HclOverrideView {
            suspended: !rows.is_empty(),
            since_local_minutes: rows.iter().map(|row| row.since_local_minutes).min(),
            targets: rows.iter().map(target_view).collect(),
        }
    }
}

fn target_view(row: &SuspendedTarget) -> HclOverrideTargetView {
    HclOverrideTargetView {
        adapter_id: row.target.adapter_id,
        scope: row.target.scope,
        group_id: match row.target.scope {
            HclTargetScope::Broadcast => None,
            HclTargetScope::Group => Some(row.target.group_id),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::overrides::OverrideLedger;
    use crate::runtime::plan::TargetKey;
    use std::sync::{Arc, Mutex};

    fn ledger_with(entries: &[(TargetKey, u16)]) -> HclOverrideLedgerRead {
        let mut ledger = OverrideLedger::new();
        for (target, since) in entries {
            ledger.suspend("morning", *target, *since);
        }
        HclOverrideLedgerRead::new(Arc::new(Mutex::new(ledger)))
    }

    fn group(group_id: u8) -> TargetKey {
        TargetKey {
            adapter_id: 0,
            scope: HclTargetScope::Group,
            group_id,
        }
    }

    fn broadcast() -> TargetKey {
        TargetKey {
            adapter_id: 0,
            scope: HclTargetScope::Broadcast,
            group_id: 0,
        }
    }

    #[test]
    fn a_schedule_with_no_flags_reports_running() {
        let view = ledger_with(&[]).hcl_override_view("morning");
        assert_eq!(view, HclOverrideView::default());
    }

    #[test]
    fn the_reported_time_is_when_the_override_started() {
        let view = ledger_with(&[(group(3), 900), (group(5), 861)]).hcl_override_view("morning");
        assert!(view.suspended);
        assert_eq!(view.since_local_minutes, Some(861));
        assert_eq!(view.targets.len(), 2);
    }

    #[test]
    fn broadcast_reports_no_group_id() {
        let view = ledger_with(&[(broadcast(), 861)]).hcl_override_view("morning");
        assert_eq!(view.targets[0].group_id, None);
        assert_eq!(view.targets[0].scope, HclTargetScope::Broadcast);
    }

    #[test]
    fn another_schedule_is_unaffected() {
        let view = ledger_with(&[(group(3), 861)]).hcl_override_view("evening");
        assert!(!view.suspended);
        assert!(view.targets.is_empty());
    }
}
