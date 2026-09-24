pub const ROLE_HEADER: &str = "X-Dali2rust-Role";

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

    #[test]
    fn a_response_that_carries_its_own_headers_is_left_alone() {
        const GZIP: &[(&str, &str)] = &[("Content-Encoding", "gzip")];
        assert_eq!(role_headers(GZIP, true), GZIP);
    }
}
