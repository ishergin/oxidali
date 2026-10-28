use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_PROBE_PERIOD: Duration = Duration::from_secs(60);

pub const RESWEEP_AFTER_YES: u32 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollerConfig {
    pub probe_period: Duration,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            probe_period: DEFAULT_PROBE_PERIOD,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PowerCycleWatch {
    yes_since_sweep: HashMap<u8, u32>,
}

impl PowerCycleWatch {
    // IEC 62386-102 §11.5.15
    pub(crate) fn sweep_due(&mut self, adapter_id: u8, power_cycle_seen: bool) -> bool {
        if !power_cycle_seen {
            self.yes_since_sweep.remove(&adapter_id);
            return false;
        }
        let Some(answers) = self.yes_since_sweep.get_mut(&adapter_id) else {
            self.yes_since_sweep.insert(adapter_id, 0);
            return true;
        };
        *answers = answers.saturating_add(1);
        if *answers < RESWEEP_AFTER_YES {
            return false;
        }
        *answers = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_yes_sweeps_and_a_standing_yes_waits_its_turn() {
        let mut watch = PowerCycleWatch::default();
        assert!(watch.sweep_due(0, true), "the first YES finds out who cycled");
        for answer in 1..RESWEEP_AFTER_YES {
            assert!(!watch.sweep_due(0, true), "YES number {answer} after a sweep is not news");
        }
        assert!(watch.sweep_due(0, true), "a standing YES re-sweeps once per {RESWEEP_AFTER_YES} answers");
        assert!(!watch.sweep_due(0, true));
    }

    #[test]
    fn a_no_rearms_the_watch_and_each_adapter_keeps_its_own() {
        let mut watch = PowerCycleWatch::default();
        assert!(watch.sweep_due(0, true));
        assert!(watch.sweep_due(1, true), "adapter 1's first YES is its own");
        assert!(!watch.sweep_due(0, true));
        assert!(!watch.sweep_due(0, false), "a NO never sweeps");
        assert!(watch.sweep_due(0, true), "a YES after a NO is a new cycle");
        assert!(!watch.sweep_due(1, true), "adapter 0's NO left adapter 1 on hold");
    }
}
