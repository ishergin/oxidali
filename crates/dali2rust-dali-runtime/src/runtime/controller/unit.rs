use super::*;

#[derive(Debug, Default)]
pub(super) struct UnitAttempt {
    broken_by: Option<FrameError>,
    started: bool,
}

impl<T: DaliTransport + Send> DaliController<T> {
    // IEC 62386-101 §9.1.4, §9.2
    pub(super) fn run_unit<R>(&mut self, exempt: bool, mut run: impl FnMut(&mut Self) -> R) -> R {
        if self.unit.is_some() {
            return self.run_transaction(exempt, |c| run(c));
        }
        let attempts = self.retry_policy.effective_max_attempts();
        let mut attempt: u8 = 0;
        loop {
            self.unit = Some(UnitAttempt::default());
            let result = self.run_transaction(exempt, |c| run(c));
            let finished = self.unit.take().unwrap_or_default();
            if !self.reruns_after(finished, attempt, attempts) {
                return result;
            }
            attempt += 1;
        }
    }

    fn reruns_after(&mut self, finished: UnitAttempt, attempt: u8, attempts: u8) -> bool {
        let Some(error) = finished.broken_by else {
            return false;
        };
        if self.retry_or_fail(attempt, attempts, error).is_err() {
            return false;
        }
        if finished.started {
            self.wire_counters.transaction_reopened.fetch_add(1, Relaxed);
        }
        true
    }

    pub(super) fn attempts_here(&self, attempts: u8) -> u8 {
        if self.unit.is_some() {
            1
        } else {
            attempts
        }
    }

    pub(super) fn unit_broken_by(&self) -> Option<FrameError> {
        self.unit.as_ref().and_then(|unit| unit.broken_by)
    }

    pub(super) fn note_unit_started(&mut self) {
        if let Some(unit) = self.unit.as_mut() {
            unit.started = true;
        }
    }

    pub(super) fn break_unit(&mut self, error: FrameError) {
        if let Some(unit) = self.unit.as_mut() {
            unit.broken_by.get_or_insert(error);
        }
    }
}
