use core::cell::Cell;

use dali2rust_platform::dali::WirePriority;

const INTERACTIVE_BUDGET_MS: u32 = 400;

const ATTENDED_BUDGET_MS: u32 = 3_000;

const UNATTENDED_BUDGET_MS: u32 = 1_525;

const UNSCOPED_BUDGET_MS: u32 = ATTENDED_BUDGET_MS;

pub(crate) const fn budget_for(priority: WirePriority) -> u32 {
    match priority {
        WirePriority::Interactive => INTERACTIVE_BUDGET_MS,
        WirePriority::Attended => ATTENDED_BUDGET_MS,
        WirePriority::Unattended => UNATTENDED_BUDGET_MS,
    }
}

thread_local! {
    static REMAINING_MS: Cell<u32> = const { Cell::new(UNSCOPED_BUDGET_MS) };
    static COMMAND_BUDGET_MS: Cell<u32> = const { Cell::new(UNSCOPED_BUDGET_MS) };
    static SINGLETON_SPENT: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Series,
    Singleton,
}

pub(crate) fn with_budget<R>(priority: Option<WirePriority>, run: impl FnOnce() -> R) -> R {
    let armed = priority.map_or(UNSCOPED_BUDGET_MS, budget_for);
    let _restore = BudgetScope {
        series: REMAINING_MS.replace(armed),
        command: COMMAND_BUDGET_MS.replace(armed),
        singleton_spent: SINGLETON_SPENT.replace(false),
    };
    run()
}

struct BudgetScope {
    series: u32,
    command: u32,
    singleton_spent: bool,
}

impl Drop for BudgetScope {
    fn drop(&mut self) {
        REMAINING_MS.set(self.series);
        COMMAND_BUDGET_MS.set(self.command);
        SINGLETON_SPENT.set(self.singleton_spent);
    }
}

pub(crate) fn budget_ms(kind: Kind) -> u32 {
    match kind {
        Kind::Series => REMAINING_MS.get(),
        Kind::Singleton => COMMAND_BUDGET_MS.get(),
    }
}

pub(crate) fn charge_ms(kind: Kind, slept_ms: u32) {
    match kind {
        Kind::Series => REMAINING_MS.set(REMAINING_MS.get().saturating_sub(slept_ms)),
        Kind::Singleton => {
            debug_assert!(
                !SINGLETON_SPENT.replace(true),
                "a second Kind::Singleton publish in one command: the uncharged \
                 allowance is only sound for the ONE frame that closes a unit of \
                 work. Anything published mid-command is a Series."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interactive_budget_leaves_the_confirmation_deadline_intact() {
        const CONFIRMATION_TIMEOUT_MS: u32 = 2_000;
        let schedule_total: u32 = dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS
            .iter()
            .map(|d| *d as u32)
            .sum();
        assert_eq!(schedule_total, 1_525, "the schedule ADR-021 ships");
        assert!(
            schedule_total > CONFIRMATION_TIMEOUT_MS / 2,
            "if this stops being true the interactive cap is no longer needed"
        );
        assert!(
            budget_for(WirePriority::Interactive) * 4 < CONFIRMATION_TIMEOUT_MS,
            "an interactive unit must leave most of the deadline to the wire"
        );
    }

    #[test]
    fn a_series_shares_one_budget_rather_than_one_each() {
        with_budget(Some(WirePriority::Attended), || {
            assert_eq!(budget_ms(Kind::Series), ATTENDED_BUDGET_MS);
            for _ in 0..10 {
                charge_ms(Kind::Series, 500);
            }
            assert_eq!(budget_ms(Kind::Series), 0, "saturating, and spent by the series");
        });
    }

    #[test]
    fn a_spent_series_does_not_starve_the_frame_that_closes_the_operation() {
        with_budget(Some(WirePriority::Attended), || {
            for _ in 0..100 {
                charge_ms(Kind::Series, 1_000);
            }
            assert_eq!(budget_ms(Kind::Series), 0, "the series is out");
            assert_eq!(
                budget_ms(Kind::Singleton),
                ATTENDED_BUDGET_MS,
                "the terminal signal keeps its own allowance"
            );
        });
    }

    #[test]
    fn a_singleton_does_not_draw_down_the_series_ceiling() {
        with_budget(Some(WirePriority::Attended), || {
            charge_ms(Kind::Singleton, ATTENDED_BUDGET_MS);
            assert_eq!(budget_ms(Kind::Series), ATTENDED_BUDGET_MS);
        });
    }

    #[test]
    fn one_command_cannot_sleep_more_than_the_pool_plus_one_closing_schedule() {
        const SCHEDULE_MS: u32 = 1_525;
        for priority in [
            WirePriority::Interactive,
            WirePriority::Attended,
            WirePriority::Unattended,
        ] {
            let pool = budget_for(priority);
            with_budget(Some(priority), || {
                let mut spent = 0u32;
                for _ in 0..10 {
                    let offered = budget_ms(Kind::Series);
                    spent += offered;
                    charge_ms(Kind::Series, offered);
                }
                assert_eq!(spent, pool, "the series may spend the pool, and no more");
                let closing = budget_ms(Kind::Singleton);
                assert_eq!(closing, pool);
                charge_ms(Kind::Singleton, closing);
                assert!(
                    spent + closing <= pool + SCHEDULE_MS.max(pool),
                    "{priority:?}: one command slept {} ms",
                    spent + closing
                );
            });
        }
    }

    #[test]
    #[should_panic(expected = "a second Kind::Singleton publish in one command")]
    fn a_second_closing_publish_in_one_command_is_a_bug() {
        with_budget(Some(WirePriority::Attended), || {
            charge_ms(Kind::Singleton, 0);
            charge_ms(Kind::Singleton, 0);
        });
    }

    #[test]
    fn each_command_gets_its_own_closing_publish() {
        for _ in 0..3 {
            with_budget(Some(WirePriority::Attended), || {
                charge_ms(Kind::Singleton, 0);
            });
        }
    }

    #[test]
    fn the_budget_is_restored_when_a_command_ends() {
        with_budget(Some(WirePriority::Attended), || {
            charge_ms(Kind::Series, ATTENDED_BUDGET_MS);
            assert_eq!(budget_ms(Kind::Series), 0);
        });
        assert_eq!(
            budget_ms(Kind::Series),
            UNSCOPED_BUDGET_MS,
            "next command starts fresh"
        );
    }

    #[test]
    fn a_panicking_command_still_disarms_its_budget() {
        let panicked = std::panic::catch_unwind(|| {
            with_budget(Some(WirePriority::Interactive), || {
                charge_ms(Kind::Series, INTERACTIVE_BUDGET_MS);
                assert_eq!(budget_ms(Kind::Series), 0);
                panic!("a handler that dies mid-unit");
            });
        });
        assert!(panicked.is_err(), "the panic must actually have happened");
        assert_eq!(
            budget_ms(Kind::Series),
            UNSCOPED_BUDGET_MS,
            "the next unit of work must not inherit a spent budget"
        );
        assert_eq!(budget_ms(Kind::Singleton), UNSCOPED_BUDGET_MS);
    }

    #[test]
    fn every_priority_has_a_budget() {
        for priority in [
            WirePriority::Interactive,
            WirePriority::Attended,
            WirePriority::Unattended,
        ] {
            let budget = budget_for(priority);
            assert!(budget > 0, "{priority:?} must be able to retry at all");
            assert!(budget <= ATTENDED_BUDGET_MS, "{priority:?} budget is unbounded");
        }
        assert!(
            budget_for(WirePriority::Interactive) < budget_for(WirePriority::Unattended),
            "a blocked operator gets less patience than a background sweep, \
             because the operator is the one holding a deadline"
        );
    }
}
