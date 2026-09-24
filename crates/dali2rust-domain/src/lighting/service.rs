// IEC 62386-102 Table 4
pub const FADE_TIME_MS: [u32; 16] = [
    0, 700, 1000, 1400, 2000, 2800, 4000, 5700, 8000, 11300, 16000, 22600, 32000, 45300, 64000,
    90500,
];

const FADE_RATE_STEPS_MS: [u32; 8] = [8, 16, 32, 64, 128, 256, 512, 1024];
const FADE_TIME_ZERO: u8 = 0;
const FADE_DIVISOR: u8 = 2;
const FADE_MUL_1: u32 = 1;
const FADE_MUL_2: u32 = 2;
const FADE_OFFSET_1: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FadeParams {
    pub fade_time: u8,
    pub fade_rate: u8,
}

impl FadeParams {
    pub fn new(fade_time: u8, fade_rate: u8) -> Self {
        Self {
            fade_time,
            fade_rate,
        }
    }

    pub fn fade_duration_ms(&self) -> u32 {
        FADE_TIME_MS
            .get(self.fade_time as usize)
            .copied()
            .unwrap_or(FADE_TIME_MS[15])
    }

    pub fn fade_rate_ms_per_step(&self) -> u32 {
        if self.fade_rate == FADE_TIME_ZERO {
            return 0;
        }
        let idx = ((self.fade_rate.wrapping_sub(FADE_OFFSET_1)) / FADE_DIVISOR) as usize;
        let mul = if (self.fade_rate.wrapping_sub(FADE_OFFSET_1)) % FADE_DIVISOR == 0 {
            FADE_MUL_1
        } else {
            FADE_MUL_2
        };
        FADE_RATE_STEPS_MS.get(idx).copied().unwrap_or(1024) * mul
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_duration_zero() {
        let p = FadeParams::new(0, 0);
        assert_eq!(p.fade_duration_ms(), 0);
    }

    #[test]
    fn fade_duration_matches_iec_table() {
        assert_eq!(FadeParams::new(1, 0).fade_duration_ms(), 700);
        assert_eq!(FadeParams::new(2, 0).fade_duration_ms(), 1000);
        assert_eq!(FadeParams::new(4, 0).fade_duration_ms(), 2000);
        assert_eq!(FadeParams::new(8, 0).fade_duration_ms(), 8000);
        assert_eq!(FadeParams::new(15, 0).fade_duration_ms(), 90500);
    }

    #[test]
    fn fade_rate_zero() {
        let p = FadeParams::new(1, 0);
        assert_eq!(p.fade_rate_ms_per_step(), 0);
    }
}
