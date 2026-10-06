use super::*;

#[derive(Debug, Default)]
pub(super) struct UnitAttempt {
    broken_by: Option<FrameError>,
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
            let broken_by = self.unit.take().and_then(|unit| unit.broken_by);
            let Some(error) = broken_by else {
                return result;
            };
            if self.retry_or_fail(attempt, attempts, error).is_err() {
                return result;
            }
            self.note_unit_reopened();
            attempt += 1;
        }
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

    pub(super) fn break_unit(&mut self, error: FrameError) {
        if let Some(unit) = self.unit.as_mut() {
            unit.broken_by.get_or_insert(error);
        }
    }
}
