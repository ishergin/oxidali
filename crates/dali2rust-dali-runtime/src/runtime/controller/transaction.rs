use super::*;

const CLOCK_SKEW_GUARD_TICKS: u32 = 3;

// IEC 62386-101 §9.2
const TRANSACTION_BUDGET_MS: u64 = 400;

// IEC 62386-101 §9.2, Table 22
pub(super) const BUS_RELEASE_SETTLE_US: u32 = 22_000;

impl<T: DaliTransport + Send> DaliController<T> {
    fn next_frame_priority(&self) -> DaliPriority {
        if self.tx.depth > 0 && self.tx.first_frame_sent {
            DaliPriority::Transaction
        } else {
            self.wire_class.first_frame()
        }
    }

    pub(super) fn next_settle(&mut self) -> (DaliPriority, u32) {
        let priority = self.next_frame_priority();
        if !self.release_due() {
            return (priority, self.random_settle_within(priority));
        }
        self.last_release_ms = self.clock.monotonic_ms();
        self.busy_since_release = false;
        self.wire_counters.bus_releases.fetch_add(1, Relaxed);
        (DaliPriority::PeriodicQuery, BUS_RELEASE_SETTLE_US)
    }

    // IEC 62386-101 Table 22
    fn random_settle_within(&mut self, priority: DaliPriority) -> u32 {
        let lead = u32::from(TX_ARM_LEAD_TICKS);
        let lo = priority.min_settle_us().div_ceil(PHY_TICK_US) + CLOCK_SKEW_GUARD_TICKS;
        let hi = priority.max_settle_us() / PHY_TICK_US;
        debug_assert!(lo <= hi, "every Table 22 band is at least one tick wide");
        debug_assert!(lo >= lead, "the lead must fit under the band's floor");
        let span = hi.saturating_sub(lo).saturating_add(1);
        let on_wire = lo + (self.next_retry_jitter() % u64::from(span)) as u32;
        on_wire.saturating_sub(lead) * PHY_TICK_US
    }

    fn release_due(&self) -> bool {
        if !self.busy_since_release {
            return false;
        }
        if self.tx.depth > 0 && self.tx.first_frame_sent {
            return false;
        }
        self.clock
            .monotonic_ms()
            .saturating_sub(self.last_release_ms)
            >= TRANSACTION_BUDGET_MS
    }

    pub(super) fn must_yield(&self) -> bool {
        if self.tx.depth > 0 && self.tx.first_frame_sent {
            return false;
        }
        let Some(lease) = self.lease.as_ref() else {
            return false;
        };
        match lease.granularity() {
            YieldGranularity::Never => false,
            YieldGranularity::Step if !self.at_step_boundary => false,
            _ => lease.should_yield(),
        }
    }

    pub(super) fn run_transaction<R>(&mut self, exempt: bool, run: impl FnOnce(&mut Self) -> R) -> R {
        if self.tx.depth == 0 {
            self.tx = TransactionState {
                depth: 0,
                exempt,
                ..TransactionState::default()
            };
        }
        self.tx.depth = self.tx.depth.saturating_add(1);
        let result = run(self);
        self.tx.depth = self.tx.depth.saturating_sub(1);
        if self.tx.depth == 0 {
            self.close_outermost();
        }
        result
    }

    fn close_outermost(&mut self) {
        if !self.tx.first_frame_sent {
            self.tx = TransactionState::default();
            return;
        }
        self.wire_counters
            .transactions_completed
            .fetch_add(1, Relaxed);
        let wire_ms = self
            .tx
            .last_frame_at_ms
            .saturating_sub(self.tx.first_frame_at_ms);
        if wire_ms > TRANSACTION_BUDGET_MS {
            let counter = if self.tx.exempt {
                &self.wire_counters.transaction_should_exceedances
            } else {
                &self.wire_counters.transaction_budget_exceeded
            };
            counter.fetch_add(1, Relaxed);
        }
        self.tx = TransactionState::default();
    }

    pub(super) fn close_transaction_for_yield(&mut self) {
        let depth = self.tx.depth;
        let exempt = self.tx.exempt;
        self.close_outermost();
        self.tx.depth = depth;
        self.tx.exempt = exempt;
    }

    pub(super) fn yield_pending(&self) -> bool {
        let Some(lease) = self.lease.as_ref() else {
            return false;
        };
        match lease.granularity() {
            YieldGranularity::Never => false,
            _ => lease.should_yield(),
        }
    }

    pub(super) fn note_unit_reopened(&mut self) {
        if self.tx.depth > 0 && self.tx.first_frame_sent {
            self.wire_counters.transaction_reopened.fetch_add(1, Relaxed);
        }
    }

    pub(super) fn credit_natural_release(&mut self) {
        let quiet = self
            .session
            .min_wait_before_next(BUS_RELEASE_SETTLE_US, self.clock.as_ref());
        if quiet.is_zero() {
            self.last_release_ms = self.clock.monotonic_ms();
            self.busy_since_release = false;
        }
    }

    pub(super) fn note_settle_window(&self, priority: DaliPriority) {
        if priority != DaliPriority::Transaction {
            return;
        }
        let Some(ticks) = self.last_tx_settle_ticks() else {
            return;
        };
        let settle_us = u32::from(ticks) * PHY_TICK_US;
        if settle_us > DaliPriority::Transaction.max_settle_us() {
            self.wire_counters.p1_window_late.fetch_add(1, Relaxed);
        }
    }

    pub(super) fn last_tx_settle_ticks(&self) -> Option<u16> {
        self.transport
            .lock()
            .expect("DaliController: transport mutex poisoned (previous panic in transport code)")
            .last_tx_settle_ticks()
    }
}
