const PRODUCTION_PULSE_PERIOD_MS: u32 = 2_000;

const LISTEN_PERIODS: u32 = 2;

const LIVENESS_HALF_PERIODS: u32 = 7;

const HALVES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceTiming {
    pulse_period_ms: u32,
}

impl PresenceTiming {
    pub const PRODUCTION: Self = Self::every_ms(PRODUCTION_PULSE_PERIOD_MS);

    pub const fn every_ms(pulse_period_ms: u32) -> Self {
        Self { pulse_period_ms }
    }

    pub const fn pulse_period_ms(self) -> u32 {
        self.pulse_period_ms
    }

    pub const fn listen_ms(self) -> u32 {
        self.pulse_period_ms.saturating_mul(LISTEN_PERIODS)
    }

    pub const fn liveness_ms(self) -> u32 {
        self.pulse_period_ms.saturating_mul(LIVENESS_HALF_PERIODS) / HALVES
    }
}

pub(crate) const fn elapsed_ms(now_ms: u32, since_ms: u32) -> u32 {
    now_ms.wrapping_sub(since_ms)
}

pub(crate) const fn later(now_ms: u32, at_ms: u32, than_ms: u32) -> bool {
    elapsed_ms(now_ms, at_ms) < elapsed_ms(now_ms, than_ms)
}
