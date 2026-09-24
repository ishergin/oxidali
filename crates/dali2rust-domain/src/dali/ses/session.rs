use core::time::Duration;

use crate::dali::commands::{DaliCommand, SpecialCommand};
use dali2rust_platform::clock::Clock;

// IEC 62386-101 Table 22
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum DaliPriority {
    Transaction = 1,
    UserAction = 2,
    Configuration = 3,
    Automatic = 4,
    PeriodicQuery = 5,
}

impl DaliPriority {
    pub const fn min_settle_us(self) -> u32 {
        match self {
            Self::Transaction => 13_500,
            Self::UserAction => 14_900,
            Self::Configuration => 16_300,
            Self::Automatic => 17_900,
            Self::PeriodicQuery => 19_500,
        }
    }

    pub const fn max_settle_us(self) -> u32 {
        match self {
            Self::Transaction => 14_700,
            Self::UserAction => 16_100,
            Self::Configuration => 17_700,
            Self::Automatic => 19_300,
            Self::PeriodicQuery => 21_100,
        }
    }

    pub const fn contains_settle_us(self, settle_us: u32) -> bool {
        settle_us >= self.min_settle_us() && settle_us <= self.max_settle_us()
    }
}

// IEC 62386-101 §9.2
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TransactionPriority {
    UserAction = 2,
    Configuration = 3,
    Automatic = 4,
    PeriodicQuery = 5,
}

impl TransactionPriority {
    pub const fn first_frame(self) -> DaliPriority {
        match self {
            Self::UserAction => DaliPriority::UserAction,
            Self::Configuration => DaliPriority::Configuration,
            Self::Automatic => DaliPriority::Automatic,
            Self::PeriodicQuery => DaliPriority::PeriodicQuery,
        }
    }
}

// IEC 62386-103 §9.13.1
pub fn command_priority(cmd: &DaliCommand) -> TransactionPriority {
    let configures = match cmd {
        DaliCommand::Standard { command, .. } => command.requires_repeat(),
        DaliCommand::Extended { command, .. } => command.requires_repeat(),
        DaliCommand::Special(command) => is_configuration_special(*command),
    };
    if configures {
        TransactionPriority::Configuration
    } else {
        TransactionPriority::UserAction
    }
}

fn is_configuration_special(command: SpecialCommand) -> bool {
    matches!(
        command,
        SpecialCommand::WriteMemoryLocation(_) | SpecialCommand::WriteMemoryLocationNoReply(_)
    )
}

#[derive(Debug)]
pub struct DaliSession {
    last_transmission_ms: Option<u64>,
}

impl DaliSession {
    pub fn new() -> Self {
        Self {
            last_transmission_ms: None,
        }
    }

    pub fn min_wait_before_next(&self, min_settle_us: u32, clock: &dyn Clock) -> Duration {
        match self.last_transmission_ms {
            Some(last_ms) => {
                let now = clock.monotonic_ms();
                let elapsed_us = now.saturating_sub(last_ms).saturating_mul(1_000);
                let min_settle_us = u64::from(min_settle_us);
                if elapsed_us < min_settle_us {
                    Duration::from_micros(min_settle_us - elapsed_us)
                } else {
                    Duration::ZERO
                }
            }
            None => Duration::ZERO,
        }
    }

    pub fn record_transmission(&mut self, clock: &dyn Clock) {
        self.last_transmission_ms = Some(clock.monotonic_ms());
    }
}

impl Default for DaliSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::commands::StandardCommand;
    use crate::test_support::FakeClock;

    const SETTLE_P3_US: u32 = DaliPriority::Configuration.min_settle_us();

    #[test]
    fn session_waits_between_transmissions() {
        let clock = FakeClock::new(0);
        let mut session = DaliSession::new();
        assert_eq!(
            session.min_wait_before_next(SETTLE_P3_US, &clock),
            Duration::ZERO
        );

        session.record_transmission(&clock);
        let wait = session.min_wait_before_next(SETTLE_P3_US, &clock);
        assert!(wait <= Duration::from_micros(u64::from(SETTLE_P3_US)));
        assert!(wait > Duration::ZERO);
    }

    #[test]
    fn session_wait_decreases_after_advance() {
        let clock = FakeClock::new(0);
        let mut session = DaliSession::new();
        session.record_transmission(&clock);
        clock.advance(17);
        assert_eq!(
            session.min_wait_before_next(SETTLE_P3_US, &clock),
            Duration::ZERO
        );
    }

    #[test]
    fn session_does_not_panic_when_clock_wraps_down() {
        let clock = FakeClock::new(1_000_000);
        let mut session = DaliSession::new();
        session.record_transmission(&clock);
        let wait = session.min_wait_before_next(SETTLE_P3_US, &clock);
        assert!(wait <= Duration::from_micros(u64::from(SETTLE_P3_US)));
    }

    #[test]
    fn settling_windows_match_table_22() {
        let table = [
            (DaliPriority::Transaction, 13_500, 14_700),
            (DaliPriority::UserAction, 14_900, 16_100),
            (DaliPriority::Configuration, 16_300, 17_700),
            (DaliPriority::Automatic, 17_900, 19_300),
            (DaliPriority::PeriodicQuery, 19_500, 21_100),
        ];
        for (priority, min, max) in table {
            assert_eq!(priority.min_settle_us(), min, "{priority:?} minimum");
            assert_eq!(priority.max_settle_us(), max, "{priority:?} maximum");
        }
    }

    #[test]
    fn adjacent_settling_windows_are_disjoint() {
        let ladder = [
            DaliPriority::Transaction,
            DaliPriority::UserAction,
            DaliPriority::Configuration,
            DaliPriority::Automatic,
            DaliPriority::PeriodicQuery,
        ];
        for pair in ladder.windows(2) {
            let (lower, higher) = (pair[0], pair[1]);
            assert!(
                lower.max_settle_us() < higher.min_settle_us(),
                "{lower:?} window must end before {higher:?} begins"
            );
            assert_eq!(
                higher.min_settle_us() - lower.max_settle_us(),
                200,
                "{lower:?}→{higher:?} margin"
            );
        }
    }

    #[test]
    fn a_window_contains_exactly_its_own_band() {
        let p1 = DaliPriority::Transaction;
        assert!(p1.contains_settle_us(13_500));
        assert!(p1.contains_settle_us(14_700));
        assert!(!p1.contains_settle_us(13_499));
        assert!(!p1.contains_settle_us(14_900));
    }

    #[test]
    fn no_transaction_class_opens_with_priority_one() {
        for class in [
            TransactionPriority::UserAction,
            TransactionPriority::Configuration,
            TransactionPriority::Automatic,
            TransactionPriority::PeriodicQuery,
        ] {
            assert_ne!(
                class.first_frame(),
                DaliPriority::Transaction,
                "{class:?} must not open a transaction at priority 1"
            );
        }
    }

    #[test]
    fn the_raw_diagnostic_path_separates_configuration_from_control() {
        let address = crate::dali::types::DaliAddress::short(1).unwrap();
        let query = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryStatus,
        };
        let set_scene = DaliCommand::Standard {
            address,
            command: StandardCommand::Reset,
        };
        let compare = DaliCommand::Special(SpecialCommand::Compare);
        let memory_write = DaliCommand::Special(SpecialCommand::WriteMemoryLocation(0x12));

        assert_eq!(command_priority(&query), TransactionPriority::UserAction);
        assert_eq!(
            command_priority(&set_scene),
            TransactionPriority::Configuration
        );
        assert_eq!(command_priority(&compare), TransactionPriority::UserAction);
        assert_eq!(
            command_priority(&memory_write),
            TransactionPriority::Configuration
        );
    }
}
