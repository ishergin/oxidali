use super::*;

const US_PER_MS: u32 = 1_000;
const MS_CLOCK_TRUNCATION_GUARD_MS: u64 = 1;
// IEC 62386-101 Table 20
pub(super) const STRAY_HALF_LIFETIME_MS: u64 =
    RetryPolicy::SEND_TWICE_GREY_MAX_INTERVAL_US.div_ceil(US_PER_MS) as u64
        + MS_CLOCK_TRUNCATION_GUARD_MS;

#[derive(Clone, Copy)]
pub(super) struct StrayHalf {
    frame: WireFrame,
    expires_at_ms: u64,
}

impl<T: DaliTransport + Send> DaliController<T> {
    // IEC 62386-101 §9.3
    pub(super) fn judge_send_twice_pair(&mut self, half: WireFrame) -> Result<(), FrameError> {
        if self.captures_between_last_two_tx().is_some_and(|captures| captures > 0) {
            return Err(self.split_send_twice_pair(half));
        }
        self.judge_send_twice_interval(half)
    }

    // IEC 62386-101 §9.3
    fn split_send_twice_pair(&mut self, half: WireFrame) -> FrameError {
        self.wire_counters.send_twice_split.fetch_add(1, Relaxed);
        self.stray_half = Some(StrayHalf {
            frame: half,
            expires_at_ms: self.clock.monotonic_ms() + STRAY_HALF_LIFETIME_MS,
        });
        FrameError::BusBusy
    }

    // IEC 62386-101 Table 17, Table 20
    fn judge_send_twice_interval(&mut self, half: WireFrame) -> Result<(), FrameError> {
        let Some(ticks) = self.last_tx_settle_ticks() else {
            return Ok(());
        };
        let interval_us = u32::from(ticks) * PHY_TICK_US;
        if interval_us >= RetryPolicy::SEND_TWICE_MAX_INTERVAL_US {
            return Err(self.split_send_twice_pair(half));
        }
        if interval_us >= RetryPolicy::SEND_TWICE_TX_MAX_US {
            self.wire_counters
                .send_twice_over_transmitter_max
                .fetch_add(1, Relaxed);
        }
        Ok(())
    }

    pub(super) fn stray_half_wait(&self, frame: WireFrame) -> Duration {
        let Some(stray) = self.stray_half.filter(|stray| stray.frame == frame) else {
            return Duration::ZERO;
        };
        Duration::from_millis(stray.expires_at_ms.saturating_sub(self.clock.monotonic_ms()))
    }

    pub(super) fn wait_out_stray_half(&mut self, frame: WireFrame) -> bool {
        let wait = self.stray_half_wait(frame);
        if wait.is_zero() {
            return false;
        }
        // sleep-ok: IEC 62386-101 §9.3 — a split pair's stray half must not pair with this frame
        std::thread::sleep(wait);
        if self.tx.depth > 0 && self.tx.first_frame_sent {
            self.close_transaction_for_yield();
            self.tx.shielded_across_release = true;
        }
        true
    }
}
