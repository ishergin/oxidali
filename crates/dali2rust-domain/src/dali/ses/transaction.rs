use core::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub base_backoff_ms: u64,
    pub jitter_ms: u64,
    pub query_contention_retry: bool,
}

impl RetryPolicy {
    // IEC 62386-101 Table 17
    pub const SEND_TWICE_TX_MAX_US: u32 = 75_000;
    // IEC 62386-101 Table 20
    pub const SEND_TWICE_MAX_INTERVAL_US: u32 = 94_000;
    // IEC 62386-101 Table 20
    pub const SEND_TWICE_GREY_MAX_INTERVAL_US: u32 = 105_000;

    pub const fn new(max_attempts: u8, base_backoff_ms: u64, jitter_ms: u64) -> Self {
        Self {
            max_attempts,
            base_backoff_ms,
            jitter_ms,
            query_contention_retry: true,
        }
    }

    pub const fn with_query_contention_retry(mut self, on: bool) -> Self {
        self.query_contention_retry = on;
        self
    }

    pub const fn effective_max_attempts(&self) -> u8 {
        if self.max_attempts == 0 {
            1
        } else {
            self.max_attempts
        }
    }

    pub fn backoff_delay(&self, retry_index: u8, jitter_sample: u64) -> Duration {
        let shift = u32::from(retry_index.min(20));
        let multiplier = 1u64 << shift;
        let base = self.base_backoff_ms.saturating_mul(multiplier);
        let jitter = self.bounded_jitter(jitter_sample);
        Duration::from_millis(base.saturating_add(jitter))
    }

    fn bounded_jitter(&self, jitter_sample: u64) -> u64 {
        if self.jitter_ms == 0 {
            return 0;
        }
        jitter_sample % self.jitter_ms.saturating_add(1)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new(3, 3, 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::ses::session::DaliPriority;

    #[test]
    fn retry_policy_zero_attempts_clamps_to_one() {
        let policy = RetryPolicy::new(0, 3, 3);
        assert_eq!(policy.effective_max_attempts(), 1);
    }

    #[test]
    fn retry_policy_backoff_grows_exponentially() {
        let policy = RetryPolicy::new(3, 3, 0);
        assert_eq!(policy.backoff_delay(0, 0), Duration::from_millis(3));
        assert_eq!(policy.backoff_delay(1, 0), Duration::from_millis(6));
        assert_eq!(policy.backoff_delay(2, 0), Duration::from_millis(12));
    }

    #[test]
    fn retry_policy_jitter_is_bounded_inclusively() {
        let policy = RetryPolicy::new(3, 3, 3);
        assert_eq!(policy.backoff_delay(0, 0), Duration::from_millis(3));
        assert_eq!(policy.backoff_delay(0, 1), Duration::from_millis(4));
        assert_eq!(policy.backoff_delay(0, 3), Duration::from_millis(6));
        assert_eq!(policy.backoff_delay(0, 7), Duration::from_millis(6));
    }

    #[test]
    fn retry_policy_defaults_query_contention_retry_to_true() {
        assert!(RetryPolicy::default().query_contention_retry);
    }

    #[test]
    fn retry_policy_can_disable_query_contention_retry() {
        let policy = RetryPolicy::new(3, 3, 3).with_query_contention_retry(false);
        assert!(!policy.query_contention_retry);
    }

    #[test]
    fn the_transmitter_limit_is_tighter_than_the_receivers() {
        assert!(
            RetryPolicy::SEND_TWICE_TX_MAX_US < RetryPolicy::SEND_TWICE_MAX_INTERVAL_US,
            "our own obligation must be the tighter of the two"
        );
        assert_eq!(RetryPolicy::SEND_TWICE_TX_MAX_US, 75_000, "Table 17 note c");
        assert_eq!(
            RetryPolicy::SEND_TWICE_MAX_INTERVAL_US,
            94_000,
            "Table 20, forward-to-forward of a send-twice pair"
        );
        assert!(
            RetryPolicy::SEND_TWICE_MAX_INTERVAL_US < RetryPolicy::SEND_TWICE_GREY_MAX_INTERVAL_US,
            "Table 20's grey area sits above the pair limit, not below it"
        );
    }

    #[test]
    fn high_priority_repeat_fits_send_twice_window() {
        assert!(
            DaliPriority::Transaction.min_settle_us() < RetryPolicy::SEND_TWICE_MAX_INTERVAL_US,
            "repeat High-settling must remain inside the send-twice receiver window"
        );
    }

}
