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

pub(crate) type ShortSet = u64;

pub(crate) fn short_bit(short_address: u8) -> ShortSet {
    1u64.checked_shl(u32::from(short_address)).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdapterWatch {
    Sweeping { owed: ShortSet },
    HeldOff { answers: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SweepOrder {
    pub(crate) fresh: bool,
    pub(crate) owed: ShortSet,
}

#[derive(Debug, Default)]
pub(crate) struct PowerCycleWatch {
    adapters: HashMap<u8, AdapterWatch>,
}

impl PowerCycleWatch {
    // IEC 62386-102 §11.5.15
    pub(crate) fn answer(&mut self, adapter_id: u8, power_cycle_seen: bool, listed: ShortSet) -> Option<SweepOrder> {
        if !power_cycle_seen {
            self.adapters.remove(&adapter_id);
            return None;
        }
        let order = match self.adapters.get(&adapter_id).copied() {
            Some(AdapterWatch::Sweeping { owed }) => SweepOrder { fresh: false, owed: owed & listed },
            Some(AdapterWatch::HeldOff { answers }) if answers.saturating_add(1) < RESWEEP_AFTER_YES => {
                self.adapters.insert(adapter_id, AdapterWatch::HeldOff { answers: answers.saturating_add(1) });
                return None;
            }
            _ => SweepOrder { fresh: true, owed: listed },
        };
        self.owe(adapter_id, order.owed);
        (order.owed != 0).then_some(order)
    }

    pub(crate) fn settle(&mut self, adapter_id: u8, short_address: u8) {
        if let Some(AdapterWatch::Sweeping { owed }) = self.adapters.get(&adapter_id).copied() {
            self.owe(adapter_id, owed & !short_bit(short_address));
        }
    }

    fn owe(&mut self, adapter_id: u8, owed: ShortSet) {
        let watch = if owed == 0 {
            AdapterWatch::HeldOff { answers: 0 }
        } else {
            AdapterWatch::Sweeping { owed }
        };
        self.adapters.insert(adapter_id, watch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEAR_1_AND_2: ShortSet = 0b110;

    fn fresh(owed: ShortSet) -> Option<SweepOrder> {
        Some(SweepOrder { fresh: true, owed })
    }

    fn resumed(owed: ShortSet) -> Option<SweepOrder> {
        Some(SweepOrder { fresh: false, owed })
    }

    #[test]
    fn the_first_yes_sweeps_and_a_standing_yes_waits_its_turn_after_the_sweep() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2), "the first YES finds out who cycled");
        watch.settle(0, 1);
        watch.settle(0, 2);
        for answer in 1..RESWEEP_AFTER_YES {
            assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None, "YES number {answer} after a sweep is not news");
        }
        assert_eq!(
            watch.answer(0, true, GEAR_1_AND_2),
            fresh(GEAR_1_AND_2),
            "a standing YES re-sweeps once per {RESWEEP_AFTER_YES} answers"
        );
    }

    #[test]
    fn the_hold_off_counts_only_from_a_sweep_every_target_settled() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2));
        watch.settle(0, 1);
        for _ in 0..=RESWEEP_AFTER_YES {
            assert_eq!(
                watch.answer(0, true, GEAR_1_AND_2),
                resumed(short_bit(2)),
                "a target the sweep still owes is asked for on every YES, however many"
            );
        }
        watch.settle(0, 2);
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None, "the sweep is over and the hold-off has begun");
    }

    #[test]
    fn a_target_no_longer_listed_is_no_longer_owed() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2));
        assert_eq!(watch.answer(0, true, short_bit(1)), resumed(short_bit(1)));
        watch.settle(0, 1);
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None, "gear 2 was delisted, so the sweep ended with gear 1");
    }

    #[test]
    fn a_yes_with_nothing_listed_starts_the_hold_off() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, 0), None);
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None, "the hold-off runs");
    }

    #[test]
    fn a_settle_outside_a_sweep_changes_nothing() {
        let mut watch = PowerCycleWatch::default();
        watch.settle(0, 1);
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2), "an armed watch stays armed");
    }

    #[test]
    fn a_no_rearms_the_watch_and_each_adapter_keeps_its_own() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2));
        assert_eq!(watch.answer(1, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2), "adapter 1's first YES is its own");
        watch.settle(0, 1);
        watch.settle(0, 2);
        watch.settle(1, 1);
        watch.settle(1, 2);
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None);
        assert_eq!(watch.answer(0, false, GEAR_1_AND_2), None, "a NO never sweeps");
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2), "a YES after a NO is a new cycle");
        assert_eq!(watch.answer(1, true, GEAR_1_AND_2), None, "adapter 0's NO left adapter 1 on hold");
    }

    #[test]
    fn a_short_address_beyond_the_set_owes_nothing() {
        assert_eq!(short_bit(64), 0);
        assert_eq!(short_bit(63), 1 << 63);
    }
}
