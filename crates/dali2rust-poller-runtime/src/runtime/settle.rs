use dali2rust_contracts::msg::{DaliLevelTransitionAppliedEvent, DaliTargetScope, LevelTransition};
use dali2rust_domain::registry::PollTargetView;

// IEC 62386-102 §9.8.2, §11.3.3, §11.3.4
pub(super) fn settle_shorts(
    body: &DaliLevelTransitionAppliedEvent,
    targets: impl FnOnce() -> Vec<PollTargetView>,
) -> Vec<u8> {
    if !matches!(body.transition, LevelTransition::Up | LevelTransition::Down) {
        return Vec::new();
    }
    match body.scope {
        DaliTargetScope::Short | DaliTargetScope::VirtualLamp => body.short_address.into_iter().collect(),
        DaliTargetScope::Group => match body.group_id.and_then(|group| 1u16.checked_shl(u32::from(group))) {
            Some(bit) => targets()
                .into_iter()
                .filter(|target| target.dali_groups & bit != 0)
                .map(|target| target.short_address)
                .collect(),
            None => Vec::new(),
        },
        DaliTargetScope::Broadcast => targets().into_iter().map(|target| target.short_address).collect(),
        DaliTargetScope::AddressRange => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::RuntimeSource;

    fn applied(scope: DaliTargetScope, transition: LevelTransition) -> DaliLevelTransitionAppliedEvent {
        DaliLevelTransitionAppliedEvent {
            registry_adapter_id: 0,
            scope,
            virtual_lamp_id: (scope == DaliTargetScope::VirtualLamp).then_some(1),
            short_address: matches!(scope, DaliTargetScope::Short | DaliTargetScope::VirtualLamp).then_some(5),
            group_id: (scope == DaliTargetScope::Group).then_some(3),
            transition,
            source: RuntimeSource::Rules,
            applied_at_mono_ms: 0,
            hold_hcl: true,
        }
    }

    fn gear(short_address: u8, dali_groups: u16) -> PollTargetView {
        PollTargetView {
            short_address,
            is_dt8: false,
            is_dt6: false,
            declares_energy: false,
            declares_diagnostics: false,
            dali_groups,
        }
    }

    fn installation() -> Vec<PollTargetView> {
        vec![gear(1, 1 << 3), gear(2, 0), gear(4, (1 << 3) | 1)]
    }

    #[test]
    fn a_lamp_or_a_device_settles_the_short_its_frame_went_to() {
        for scope in [DaliTargetScope::Short, DaliTargetScope::VirtualLamp] {
            assert_eq!(settle_shorts(&applied(scope, LevelTransition::Up), installation), vec![5]);
        }
    }

    #[test]
    fn a_group_settles_its_members_and_broadcast_settles_every_listed_gear() {
        assert_eq!(settle_shorts(&applied(DaliTargetScope::Group, LevelTransition::Down), installation), vec![1, 4]);
        assert_eq!(
            settle_shorts(&applied(DaliTargetScope::Broadcast, LevelTransition::Up), installation),
            vec![1, 2, 4]
        );
    }

    #[test]
    fn a_transition_whose_level_the_registry_computes_settles_nothing() {
        for transition in [LevelTransition::RecallMaxLevel, LevelTransition::RecallMinLevel] {
            assert!(settle_shorts(&applied(DaliTargetScope::Broadcast, transition), installation).is_empty());
        }
    }
}
