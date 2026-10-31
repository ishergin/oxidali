pub const ROLE_HEADER: &str = "X-Dali2rust-Role";
const RETRY_AFTER: &str = "Retry-After";
const MILLIS_PER_SECOND: u32 = 1_000;

pub const QUIESCENT_REFUSAL: &[(&str, &str)] = &[(RETRY_AFTER, "900")];

macro_rules! quiescent_retry_after {
    ($($seconds:literal),+) => {
        const QUIESCENT_RETRY_SECONDS: &[u32] = &[$($seconds),+];
        const QUIESCENT_ACTIVE: &[&[(&str, &str)]] =
            &[$(&[(RETRY_AFTER, stringify!($seconds)), (ROLE_HEADER, "active")]),+];
        const QUIESCENT_STANDBY: &[&[(&str, &str)]] =
            &[$(&[(RETRY_AFTER, stringify!($seconds)), (ROLE_HEADER, "standby")]),+];
    };
}

quiescent_retry_after!(1, 2, 5, 10, 20, 30, 60, 120, 180, 240, 300, 360, 420, 480, 540, 600, 660, 720, 780, 840, 900);

pub const ROLE_HEADER_ACTIVE: &[(&str, &str)] = &[(ROLE_HEADER, "active")];
pub const ROLE_HEADER_STANDBY: &[(&str, &str)] = &[(ROLE_HEADER, "standby")];

#[must_use]
pub fn role_name(active: bool) -> &'static str {
    if active {
        "active"
    } else {
        "standby"
    }
}

pub trait ControllerRolePort: Send + Sync {
    fn is_active(&self) -> bool;

    fn quiescent_remaining_ms(&self) -> Option<u32> {
        None
    }
}

#[must_use]
pub fn stamped_headers(
    existing: &'static [(&'static str, &'static str)],
    role: &dyn ControllerRolePort,
) -> &'static [(&'static str, &'static str)] {
    if existing == QUIESCENT_REFUSAL {
        return quiescent_headers(role.quiescent_remaining_ms(), role.is_active());
    }
    role_headers(existing, role.is_active())
}

// IEC 62386-103 §9.9.3
fn quiescent_headers(remaining_ms: Option<u32>, active: bool) -> &'static [(&'static str, &'static str)] {
    let seconds = remaining_ms.map_or(1, |remaining| remaining.div_ceil(MILLIS_PER_SECOND));
    let last = QUIESCENT_RETRY_SECONDS.len().saturating_sub(1);
    let bucket = QUIESCENT_RETRY_SECONDS.iter().position(|bucket| *bucket >= seconds).unwrap_or(last);
    let table = if active { QUIESCENT_ACTIVE } else { QUIESCENT_STANDBY };
    table.get(bucket).copied().unwrap_or(QUIESCENT_REFUSAL)
}

#[must_use]
pub fn role_headers(
    existing: &'static [(&'static str, &'static str)],
    active: bool,
) -> &'static [(&'static str, &'static str)] {
    if !existing.is_empty() {
        return existing;
    }
    if active {
        ROLE_HEADER_ACTIVE
    } else {
        ROLE_HEADER_STANDBY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_api_response_is_stamped_with_the_role_that_answered() {
        assert_eq!(role_headers(&[], true), ROLE_HEADER_ACTIVE);
        assert_eq!(role_headers(&[], false), ROLE_HEADER_STANDBY);
    }

    #[test]
    fn the_standby_refusal_states_its_role_despite_the_exemption() {
        let refusal = crate::http::handlers::common::standby_refusal_headers();
        assert!(refusal
            .iter()
            .any(|(name, value)| *name == ROLE_HEADER && *value == "standby"));
        assert_eq!(role_headers(refusal, false), refusal);
    }

    struct Quiescent {
        active: bool,
        remaining_ms: Option<u32>,
    }

    impl ControllerRolePort for Quiescent {
        fn is_active(&self) -> bool {
            self.active
        }

        fn quiescent_remaining_ms(&self) -> Option<u32> {
            self.remaining_ms
        }
    }

    fn retry_after(active: bool, remaining_ms: Option<u32>) -> (&'static str, &'static str) {
        let headers = stamped_headers(QUIESCENT_REFUSAL, &Quiescent { active, remaining_ms });
        let value = |name: &str| headers.iter().find(|(header, _)| *header == name).map(|(_, value)| *value);
        (value(RETRY_AFTER).expect("a quiescent refusal says when to retry"), value(ROLE_HEADER).expect("and who refused"))
    }

    #[test]
    fn a_quiescent_refusal_is_retried_no_earlier_than_the_deadline_rounded_up_to_a_step() {
        assert_eq!(retry_after(true, Some(15 * 60 * MILLIS_PER_SECOND)), ("900", "active"));
        assert_eq!(retry_after(true, Some(15 * 60 * MILLIS_PER_SECOND - 1)), ("900", "active"));
        assert_eq!(retry_after(true, Some(14 * 60 * MILLIS_PER_SECOND + 1)), ("900", "active"));
        assert_eq!(retry_after(true, Some(14 * 60 * MILLIS_PER_SECOND)), ("840", "active"));
        assert_eq!(retry_after(false, Some(61 * MILLIS_PER_SECOND)), ("120", "standby"));
        assert_eq!(retry_after(true, Some(3 * MILLIS_PER_SECOND)), ("5", "active"));
        assert_eq!(retry_after(true, Some(1)), ("1", "active"));
    }

    #[test]
    fn a_deadline_that_lapsed_since_the_refusal_is_retried_after_a_second() {
        assert_eq!(retry_after(true, None), ("1", "active"));
        assert_eq!(retry_after(false, None), ("1", "standby"));
    }

    #[test]
    fn a_retry_is_never_asked_later_than_quiescent_mode_can_last() {
        assert_eq!(retry_after(true, Some(u32::MAX)), ("900", "active"));
    }

    #[test]
    fn every_retry_step_states_its_own_seconds() {
        for (index, seconds) in QUIESCENT_RETRY_SECONDS.iter().enumerate() {
            for table in [QUIESCENT_ACTIVE, QUIESCENT_STANDBY] {
                assert_eq!(table[index][0], (RETRY_AFTER, seconds.to_string().as_str()));
            }
        }
        assert!(QUIESCENT_RETRY_SECONDS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_response_that_carries_its_own_headers_is_left_alone() {
        const GZIP: &[(&str, &str)] = &[("Content-Encoding", "gzip")];
        assert_eq!(role_headers(GZIP, true), GZIP);
    }
}
