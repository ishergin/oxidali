use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_PROBE_PERIOD: Duration = Duration::from_secs(60);

pub const RESWEEP_AFTER_YES: u32 = 60;

pub const SWEEP_READ_ATTEMPTS: u8 = 3;

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
    failed_reads: HashMap<(u8, u8), u8>,
}

impl PowerCycleWatch {
    // IEC 62386-102 §11.5.15
    pub(crate) fn answer(&mut self, adapter_id: u8, power_cycle_seen: bool, listed: ShortSet) -> Option<SweepOrder> {
        let order = self.order(adapter_id, power_cycle_seen, listed)?;
        self.owe(adapter_id, order.owed);
        (order.owed != 0).then_some(order)
    }

    pub(crate) fn news(&mut self, adapter_id: u8, power_cycle_seen: bool) -> bool {
        let Some(order) = self.order(adapter_id, power_cycle_seen, 0) else {
            return false;
        };
        self.owe(adapter_id, 0);
        order.fresh
    }

    fn order(&mut self, adapter_id: u8, power_cycle_seen: bool, listed: ShortSet) -> Option<SweepOrder> {
        if !power_cycle_seen {
            self.adapters.remove(&adapter_id);
            self.failed_reads.retain(|&(adapter, _), _| adapter != adapter_id);
            return None;
        }
        match self.adapters.get(&adapter_id).copied() {
            Some(AdapterWatch::Sweeping { owed }) => Some(SweepOrder { fresh: false, owed: owed & listed }),
            Some(AdapterWatch::HeldOff { answers }) if answers.saturating_add(1) < RESWEEP_AFTER_YES => {
                self.adapters.insert(adapter_id, AdapterWatch::HeldOff { answers: answers.saturating_add(1) });
                None
            }
            _ => Some(SweepOrder { fresh: true, owed: listed }),
        }
    }

    pub(crate) fn keep_only(&mut self, adapters: &[u8]) {
        self.adapters.retain(|adapter_id, _| adapters.contains(adapter_id));
        self.failed_reads.retain(|(adapter_id, _), _| adapters.contains(adapter_id));
    }

    pub(crate) fn owes(&self, adapter_id: u8, short_address: u8) -> bool {
        matches!(
            self.adapters.get(&adapter_id),
            Some(AdapterWatch::Sweeping { owed }) if owed & short_bit(short_address) != 0
        )
    }

    pub(crate) fn fail(&mut self, adapter_id: u8, short_address: u8) {
        if !self.owes(adapter_id, short_address) {
            return;
        }
        let failed = self.failed_reads.entry((adapter_id, short_address)).or_default();
        *failed = failed.saturating_add(1);
        if *failed >= SWEEP_READ_ATTEMPTS {
            self.settle(adapter_id, short_address);
        }
    }

    pub(crate) fn settle(&mut self, adapter_id: u8, short_address: u8) {
        if let Some(AdapterWatch::Sweeping { owed }) = self.adapters.get(&adapter_id).copied() {
            self.owe(adapter_id, owed & !short_bit(short_address));
        }
    }

    fn owe(&mut self, adapter_id: u8, owed: ShortSet) {
        self.failed_reads
            .retain(|&(adapter, short), _| adapter != adapter_id || owed & short_bit(short) != 0);
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
    fn the_hold_off_counts_only_from_a_sweep_every_target_settled_or_gave_up() {
        let mut watch = PowerCycleWatch::default();
        assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2));
        watch.settle(0, 1);
        for _ in 0..=RESWEEP_AFTER_YES {
            assert_eq!(
                watch.answer(0, true, GEAR_1_AND_2),
                resumed(short_bit(2)),
                "a target the sweep still owes and whose read has not failed is asked for on every YES, however many"
            );
        }
        for attempt in 1..SWEEP_READ_ATTEMPTS {
            watch.fail(0, 2);
            assert_eq!(
                watch.answer(0, true, GEAR_1_AND_2),
                resumed(short_bit(2)),
                "failed read {attempt} is retried on the next YES"
            );
        }
        watch.fail(0, 2);
        assert_eq!(
            watch.answer(0, true, GEAR_1_AND_2),
            None,
            "gear 2 failed {SWEEP_READ_ATTEMPTS} reads in a row: the sweep is over and the hold-off has begun"
        );
    }

    type SweepEnd = (&'static str, fn(&mut PowerCycleWatch));

    #[test]
    fn a_new_sweep_starts_its_targets_with_no_failed_reads() {
        let ends: [SweepEnd; 3] = [
            ("the target settled and the hold-off ran out", |watch| {
                watch.settle(0, 2);
                for _ in 1..RESWEEP_AFTER_YES {
                    assert_eq!(watch.answer(0, true, GEAR_1_AND_2), None);
                }
            }),
            ("a NO", |watch| assert_eq!(watch.answer(0, false, GEAR_1_AND_2), None)),
            ("the adapter left the probe round", |watch| watch.keep_only(&[])),
        ];
        for (end, close) in ends {
            let mut watch = PowerCycleWatch::default();
            assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2));
            watch.settle(0, 1);
            for _ in 1..SWEEP_READ_ATTEMPTS {
                watch.fail(0, 2);
            }
            close(&mut watch);
            assert_eq!(watch.answer(0, true, GEAR_1_AND_2), fresh(GEAR_1_AND_2), "{end}");
            watch.fail(0, 2);
            assert_eq!(
                watch.answer(0, true, GEAR_1_AND_2),
                resumed(GEAR_1_AND_2),
                "{end}: the failed reads of the last sweep do not count in this one"
            );
        }
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
