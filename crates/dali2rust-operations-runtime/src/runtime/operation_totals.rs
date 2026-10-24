#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationTotals {
    pub accepted: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub timed_out: u32,
    pub cancelled: u32,
}

impl OperationTotals {
    fn terminal(&self) -> u32 {
        self.succeeded
            .wrapping_add(self.failed)
            .wrapping_add(self.timed_out)
            .wrapping_add(self.cancelled)
    }

    #[must_use]
    pub fn in_flight(&self) -> u32 {
        self.accepted.wrapping_sub(self.terminal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEVEN_ENDED_OF_TEN: OperationTotals =
        OperationTotals { accepted: 10, succeeded: 1, failed: 2, timed_out: 3, cancelled: 1 };

    #[test]
    fn what_is_in_flight_is_what_was_accepted_and_has_not_ended() {
        assert_eq!(SEVEN_ENDED_OF_TEN.terminal(), 7);
        assert_eq!(SEVEN_ENDED_OF_TEN.in_flight(), 3);
    }

    #[test]
    fn the_count_survives_every_counter_wrapping_together() {
        let totals = OperationTotals { accepted: 2, succeeded: u32::MAX, failed: 1, timed_out: 1, cancelled: 0 };
        assert_eq!(totals.terminal(), 1, "2^32 + 1 endings wrap to 1");
        assert_eq!(totals.in_flight(), 1, "2^32 + 2 accepted less 2^32 + 1 ended is one");
    }
}
