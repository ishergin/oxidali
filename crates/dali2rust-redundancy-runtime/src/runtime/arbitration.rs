use dali2rust_domain::registry::RedundancySettingsView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionReason {
    PeerSilent = 0,
    PeerAnswered = 1,
    Manual = 2,
    Handover = 3,
    Boot = 4,
}

impl TransitionReason {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitrationState {
    Disabled,
    Listening { until_ms: u32 },
    Passive { missed: u8 },
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitrationAction {
    Idle,
    Probe,
    Claim(TransitionReason),
    StandDown(TransitionReason),
}

#[must_use]
pub fn arbitration_step(
    state: ArbitrationState,
    now_ms: u32,
    settings: RedundancySettingsView,
    application_active: bool,
    verdict: Option<bool>,
) -> (ArbitrationState, ArbitrationAction) {
    if !settings.enabled {
        return (ArbitrationState::Disabled, ArbitrationAction::Idle);
    }
    match state {
        ArbitrationState::Disabled => listening_turn(
            now_ms.wrapping_add(settings.boot_listen_ms),
            now_ms,
            settings,
            application_active,
        ),
        ArbitrationState::Listening { until_ms } => {
            listening_turn(until_ms, now_ms, settings, application_active)
        }
        ArbitrationState::Passive { missed } => {
            passive_turn(missed, settings, application_active, verdict)
        }
        ArbitrationState::Active => active_turn(settings, application_active, verdict),
    }
}

fn listening_turn(
    until_ms: u32,
    now_ms: u32,
    settings: RedundancySettingsView,
    application_active: bool,
) -> (ArbitrationState, ArbitrationAction) {
    if (until_ms.wrapping_sub(now_ms) as i32) > 0 {
        return (ArbitrationState::Listening { until_ms }, ArbitrationAction::Idle);
    }
    // IEC 62386-103 §9.9.1
    if application_active {
        return (ArbitrationState::Active, ArbitrationAction::Idle);
    }
    if settings.standby_role {
        return (
            ArbitrationState::Passive { missed: 0 },
            ArbitrationAction::Probe,
        );
    }
    (
        ArbitrationState::Passive { missed: 0 },
        ArbitrationAction::Idle,
    )
}

fn passive_turn(
    missed: u8,
    settings: RedundancySettingsView,
    application_active: bool,
    verdict: Option<bool>,
) -> (ArbitrationState, ArbitrationAction) {
    if application_active {
        return (ArbitrationState::Active, ArbitrationAction::Idle);
    }
    if !settings.standby_role {
        // DiiA 351 §5
        return (ArbitrationState::Passive { missed }, ArbitrationAction::Idle);
    }
    match verdict {
        Some(true) => (
            ArbitrationState::Passive { missed: 0 },
            ArbitrationAction::Probe,
        ),
        Some(false) => {
            let missed = missed.saturating_add(1);
            if missed >= settings.takeover_after_missed.max(1) {
                (
                    ArbitrationState::Active,
                    ArbitrationAction::Claim(TransitionReason::PeerSilent),
                )
            } else {
                (
                    ArbitrationState::Passive { missed },
                    ArbitrationAction::Probe,
                )
            }
        }
        None => (
            ArbitrationState::Passive { missed },
            ArbitrationAction::Probe,
        ),
    }
}

fn active_turn(
    settings: RedundancySettingsView,
    application_active: bool,
    verdict: Option<bool>,
) -> (ArbitrationState, ArbitrationAction) {
    if !application_active {
        return (
            ArbitrationState::Passive { missed: 0 },
            ArbitrationAction::Idle,
        );
    }
    if !settings.standby_role {
        return (ArbitrationState::Active, ArbitrationAction::Idle);
    }
    // DiiA 351 §7
    match verdict {
        Some(true) => (
            ArbitrationState::Passive { missed: 0 },
            ArbitrationAction::StandDown(TransitionReason::PeerAnswered),
        ),
        _ => (ArbitrationState::Active, ArbitrationAction::Probe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(enabled: bool, standby: bool) -> RedundancySettingsView {
        RedundancySettingsView {
            enabled,
            standby_role: standby,
            probe_interval_ms: 400,
            takeover_after_missed: 2,
            boot_listen_ms: 2_000,
            peer_device_short_address: None,
            peer_url: String::new(),
        }
    }

    fn step(
        state: ArbitrationState,
        now: u32,
        standby: bool,
        active: bool,
        verdict: Option<bool>,
    ) -> (ArbitrationState, ArbitrationAction) {
        arbitration_step(state, now, settings(true, standby), active, verdict)
    }

    #[test]
    fn switched_off_it_neither_probes_nor_claims() {
        let (s, a) = arbitration_step(
            ArbitrationState::Active,
            1_000,
            settings(false, true),
            true,
            Some(false),
        );
        assert_eq!(s, ArbitrationState::Disabled);
        assert_eq!(a, ArbitrationAction::Idle);
    }

    #[test]
    fn a_booting_standby_listens_before_it_probes() {
        let (s, a) = step(ArbitrationState::Disabled, 1_000, true, false, None);
        assert_eq!(s, ArbitrationState::Listening { until_ms: 3_000 });
        assert_eq!(a, ArbitrationAction::Idle);
        let (s, a) = step(s, 2_999, true, false, None);
        assert_eq!(a, ArbitrationAction::Idle);
        let (s, a) = step(s, 3_000, true, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
        assert_eq!(a, ArbitrationAction::Probe);
    }

    #[test]
    fn a_zero_listening_window_settles_on_the_turn_it_is_read() {
        let mut view = settings(true, true);
        view.boot_listen_ms = 0;
        let (s, a) = arbitration_step(ArbitrationState::Disabled, 1_000, view, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
        assert_eq!(a, ArbitrationAction::Probe);
    }

    #[test]
    fn two_unanswered_probes_take_the_bus_and_one_does_not() {
        let (s, a) = step(ArbitrationState::Passive { missed: 0 }, 1_000, true, false, Some(false));
        assert_eq!(s, ArbitrationState::Passive { missed: 1 });
        assert_eq!(a, ArbitrationAction::Probe);
        let (s, a) = step(s, 1_400, true, false, Some(false));
        assert_eq!(s, ArbitrationState::Active);
        assert_eq!(a, ArbitrationAction::Claim(TransitionReason::PeerSilent));
    }

    #[test]
    fn an_answered_probe_resets_the_count_rather_than_decrementing_it() {
        let (s, _) = step(ArbitrationState::Passive { missed: 1 }, 1_000, true, false, Some(true));
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
    }

    #[test]
    fn a_probe_with_no_verdict_is_not_a_miss() {
        let (s, a) = step(ArbitrationState::Passive { missed: 1 }, 1_000, true, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 1 });
        assert_eq!(a, ArbitrationAction::Probe);
    }

    #[test]
    fn an_active_standby_keeps_probing_and_stands_down_when_the_primary_answers() {
        let (s, a) = step(ArbitrationState::Active, 1_000, true, true, Some(false));
        assert_eq!(s, ArbitrationState::Active);
        assert_eq!(a, ArbitrationAction::Probe);
        let (s, a) = step(s, 1_400, true, true, Some(true));
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
        assert_eq!(a, ArbitrationAction::StandDown(TransitionReason::PeerAnswered));
    }

    #[test]
    fn a_primary_never_probes() {
        let (s, a) = step(ArbitrationState::Active, 1_000, false, true, None);
        assert_eq!(s, ArbitrationState::Active);
        assert_eq!(a, ArbitrationAction::Idle);
        let (s, a) = step(ArbitrationState::Passive { missed: 0 }, 1_000, false, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
        assert_eq!(a, ArbitrationAction::Idle);
    }

    #[test]
    fn a_standby_that_took_the_bus_comes_back_still_holding_it() {
        let (s, a) = step(ArbitrationState::Disabled, 1_000, true, true, None);
        assert_eq!(s, ArbitrationState::Listening { until_ms: 3_000 });
        assert_eq!(a, ArbitrationAction::Idle);
        let (s, a) = step(s, 3_000, true, true, None);
        assert_eq!(s, ArbitrationState::Active);
        assert_eq!(a, ArbitrationAction::Idle);
    }

    #[test]
    fn a_stood_down_primary_stays_down_across_its_own_reboot() {
        let (s, a) = step(ArbitrationState::Listening { until_ms: 0 }, 1_000, false, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
        assert_eq!(a, ArbitrationAction::Idle);
    }

    #[test]
    fn an_external_flip_is_followed_rather_than_argued_with() {
        let (s, _) = step(ArbitrationState::Passive { missed: 3 }, 1_000, true, true, None);
        assert_eq!(s, ArbitrationState::Active);
        let (s, _) = step(ArbitrationState::Active, 1_000, true, false, None);
        assert_eq!(s, ArbitrationState::Passive { missed: 0 });
    }

    #[test]
    fn a_takeover_threshold_of_zero_still_needs_one_silence() {
        let mut view = settings(true, true);
        view.takeover_after_missed = 0;
        let (s, a) = arbitration_step(
            ArbitrationState::Passive { missed: 0 },
            1_000,
            view,
            false,
            Some(false),
        );
        assert_eq!(s, ArbitrationState::Active);
        assert_eq!(a, ArbitrationAction::Claim(TransitionReason::PeerSilent));
    }
}
