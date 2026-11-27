const PRODUCTION_PULSE_PERIOD_MS: u32 = 2_000;

const LISTEN_PERIODS: u32 = 2;

const LIVENESS_HALF_PERIODS: u32 = 7;

const HALVES: u32 = 2;

const COALESCE_PER_PERIOD: u32 = 8;

const REFRESH_PERIODS: u32 = 30;

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

    pub const fn info_coalesce_ms(self) -> u32 {
        self.pulse_period_ms / COALESCE_PER_PERIOD
    }

    pub const fn info_refresh_ms(self) -> u32 {
        self.pulse_period_ms.saturating_mul(REFRESH_PERIODS)
    }
}

pub(crate) const HALF_CLOCK_MS: u32 = 1 << 31;

const AGE_CAP_MS: u32 = 1 << 30;

pub(crate) const fn elapsed_ms(now_ms: u32, since_ms: u32) -> u32 {
    let elapsed = now_ms.wrapping_sub(since_ms);
    if elapsed < HALF_CLOCK_MS {
        elapsed
    } else {
        0
    }
}

pub(crate) const fn in_reach(now_ms: u32, since_ms: u32) -> u32 {
    if elapsed_ms(now_ms, since_ms) > AGE_CAP_MS {
        now_ms.wrapping_sub(AGE_CAP_MS)
    } else {
        since_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenWindow {
    since_ms: u32,
}

impl ListenWindow {
    pub const fn opened_at(now_ms: u32) -> Self {
        Self { since_ms: now_ms }
    }

    pub fn listened_ms(&mut self, now_ms: u32) -> u32 {
        self.since_ms = in_reach(now_ms, self.since_ms);
        elapsed_ms(now_ms, self.since_ms)
    }
}

pub(crate) const fn later(now_ms: u32, at_ms: u32, than_ms: u32) -> bool {
    elapsed_ms(now_ms, at_ms) < elapsed_ms(now_ms, than_ms)
}
