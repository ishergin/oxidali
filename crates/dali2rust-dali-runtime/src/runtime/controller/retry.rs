use super::*;

impl<T: DaliTransport + Send> DaliController<T> {
    pub(super) fn next_retry_jitter(&mut self) -> u64 {
        let mut state = self.retry_seed;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        if state == 0 {
            state = 1;
        }
        self.retry_seed = state;
        state
    }

    pub(super) fn exchange_with_attempts(
        &mut self,
        frame: WireFrame,
        expects_backward: bool,
        attempts: u8,
    ) -> Result<ExchangeObservation, FrameError> {
        let mut contended = false;
        for attempt in 0..attempts {
            if self.must_yield() {
                return Err(FrameError::Preempted);
            }
            self.at_step_boundary = false;
            self.credit_natural_release();
            let (priority, settle_us) = self.next_settle();
            self.session_wait(settle_us)?;
            let exchange = self.perform_exchange(frame, expects_backward, settle_us)?;
            self.tap_exchange(frame, exchange.outcome, attempt);
            contended |= exchange.contended;
            self.note_frame_transmitted(exchange.outcome, priority);
            if let Some(result) =
                self.resolve_exchange_outcome(exchange, expects_backward, attempt, attempts, frame)?
            {
                return Ok(ExchangeObservation {
                    outcome: result,
                    contended,
                });
            }
        }
        self.wire_counters.retry_exhausted.fetch_add(1, Relaxed);
        Err(FrameError::TransportError)
    }

    pub(super) fn should_retry_contended_query_no_answer(
        &self,
        exchange: ExchangeObservation,
        expects_backward: bool,
    ) -> bool {
        expects_backward
            && exchange.outcome == TransferOutcome::NoAnswer
            && exchange.contended
            && self.retry_policy.query_contention_retry
    }

    pub(super) fn retry_query_no_answer(
        &mut self,
        attempt: u8,
        attempts: u8,
    ) -> Result<Option<TransferOutcome>, FrameError> {
        log::info!("DALI session: retrying query after contended no-answer");
        self.retry_or_fail(attempt, attempts, FrameError::Collision)
    }

    pub(super) fn retry_or_fail(
        &mut self,
        attempt: u8,
        attempts: u8,
        error: FrameError,
    ) -> Result<Option<TransferOutcome>, FrameError> {
        if !self.has_retry_remaining(attempt, attempts) {
            return Err(error);
        }
        self.wire_counters.exchange_retries.fetch_add(1, Relaxed);
        let retry_index = attempt;
        let jitter = self.next_retry_jitter();
        let delay = self.retry_delay_for(error, retry_index, jitter);
        if !delay.is_zero() {
            // sleep-ok: bounded retry backoff/collision-recovery from the RetryPolicy schedule
            std::thread::sleep(delay);
        }
        Ok(None)
    }

    fn has_retry_remaining(&self, attempt: u8, attempts: u8) -> bool {
        attempt.saturating_add(1) < attempts
    }

    fn retry_delay_for(&self, error: FrameError, retry_index: u8, jitter: u64) -> Duration {
        if error == FrameError::Collision {
            return Duration::ZERO;
        }
        self.retry_policy.backoff_delay(retry_index, jitter)
    }
}

pub(super) fn unit_retryable(error: FrameError) -> bool {
    matches!(error, FrameError::Collision | FrameError::BusBusy)
}
