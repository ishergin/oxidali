pub const OBSERVATION_ORDER_WINDOW_MS: u32 = 60_000;

pub fn observation_supersedes(incoming: Option<u32>, stored: Option<u32>) -> bool {
    let (Some(incoming), Some(stored)) = (incoming, stored) else {
        return true;
    };
    let behind = stored.wrapping_sub(incoming);
    behind == 0 || behind >= OBSERVATION_ORDER_WINDOW_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unstamped_fact_falls_back_to_arrival_order() {
        assert!(observation_supersedes(None, Some(1_000)));
        assert!(observation_supersedes(Some(1_000), None));
        assert!(observation_supersedes(None, None));
    }

    #[test]
    fn a_newer_or_simultaneous_observation_wins() {
        assert!(observation_supersedes(Some(2_000), Some(1_000)));
        assert!(observation_supersedes(Some(1_000), Some(1_000)));
    }

    #[test]
    fn an_observation_older_than_the_stored_one_loses() {
        assert!(!observation_supersedes(Some(1_000), Some(1_500)));
        assert!(!observation_supersedes(
            Some(1_000),
            Some(1_000 + OBSERVATION_ORDER_WINDOW_MS - 1)
        ));
    }

    #[test]
    fn a_long_quiet_record_still_accepts_a_fresh_fact() {
        assert!(observation_supersedes(
            Some(1_000),
            Some(1_000 + OBSERVATION_ORDER_WINDOW_MS)
        ));
        let month_ms: u32 = 30 * 24 * 60 * 60 * 1_000;
        assert!(observation_supersedes(Some(0), Some(month_ms)));
    }

    #[test]
    fn a_pair_straddling_the_u32_wrap_still_orders_correctly() {
        let before_wrap = u32::MAX - 500;
        let after_wrap = 500u32;
        assert!(observation_supersedes(Some(after_wrap), Some(before_wrap)));
        assert!(!observation_supersedes(Some(before_wrap), Some(after_wrap)));
    }
}
