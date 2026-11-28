use dali2rust_contracts::msg::{
    ColorMode, DeviceType, DeviceTypeSet, FixedText32, FixedText64, PowerState, RuntimeSource,
};

use super::physical_device::{CapabilityFlagsView, ColorTemperatureRangeView};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterLampStateView {
    pub power: PowerState,
    pub level: Option<u8>,
    pub color_mode: ColorMode,
    pub kelvin: Option<u16>,
    pub xy: Option<(f64, f64)>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
    pub observed_at_ms: Option<u32>,
    pub value_source: Option<RuntimeSource>,
    pub absent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterLampInfoView {
    pub name: FixedText64,
    pub device_type: DeviceType,
    pub device_type_overridden: bool,
    pub device_types: Option<DeviceTypeSet>,
    pub color_mode: ColorMode,
    pub color_mode_overridden: bool,
    pub capabilities: CapabilityFlagsView,
    pub kelvin_range: Option<ColorTemperatureRangeView>,
    pub bound: bool,
    pub ha_entity: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterGroupLitView {
    pub commanded: bool,
    pub lit: u8,
    pub known: u8,
    pub members: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterHaIdentityView {
    pub controller_id: FixedText32,
    pub discovery_prefix: FixedText32,
}

pub trait ClusterLampReadPort: Send + Sync {
    fn cluster_ha_identity(&self) -> ClusterHaIdentityView;

    fn cluster_lamp_ids(&self, adapter: u8) -> u64;

    fn cluster_lamp_state(&self, adapter: u8, lamp: u8) -> Option<ClusterLampStateView>;

    fn cluster_lamp_info(&self, adapter: u8, lamp: u8) -> Option<ClusterLampInfoView>;

    fn cluster_group_lit(&self, adapter: u8, group: u8) -> ClusterGroupLitView;
}

pub const fn lamp_device_type_name(device_type: DeviceType) -> &'static str {
    match device_type {
        DeviceType::Dt8Color => "dt8_color",
        DeviceType::Dt6Led => "dt6_led",
        _ => "unknown",
    }
}

pub const fn override_source_name(manual_override: bool) -> &'static str {
    if manual_override {
        "manual_override"
    } else {
        "discovered"
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPartLit {
    pub lit: Option<ClusterGroupLitView>,
    pub fresh: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelStateView {
    pub on: Option<bool>,
    pub lit: u8,
    pub known: u8,
    pub members: u8,
    pub parts_unknown: u8,
}

impl ChannelPartLit {
    pub const UNHEARD: Self = Self { lit: None, fresh: false };

    pub const fn own(lit: ClusterGroupLitView) -> Self {
        Self { lit: Some(lit), fresh: true }
    }

    fn verdict(self) -> Option<bool> {
        let lit = self.lit.filter(|_| self.fresh)?;
        if lit.commanded && lit.lit > 0 {
            Some(true)
        } else if lit.members > 0 && lit.lit == 0 && lit.known == lit.members {
            Some(false)
        } else {
            None
        }
    }
}

impl ChannelStateView {
    fn count(&mut self, part: ChannelPartLit) {
        let Some(lit) = part.lit else {
            return;
        };
        self.members = self.members.saturating_add(lit.members);
        if part.fresh {
            self.lit = self.lit.saturating_add(lit.lit);
            self.known = self.known.saturating_add(lit.known);
        }
    }
}

pub fn channel_state(parts: impl IntoIterator<Item = ChannelPartLit>) -> ChannelStateView {
    let mut view = ChannelStateView::default();
    let (mut any_on, mut all_off, mut any) = (false, true, false);
    for part in parts {
        any = true;
        let verdict = part.verdict();
        any_on |= verdict == Some(true);
        all_off &= verdict == Some(false);
        view.parts_unknown = view.parts_unknown.saturating_add(u8::from(verdict.is_none()));
        view.count(part);
    }
    view.on = if any_on { Some(true) } else { (any && all_off).then_some(false) };
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMBERS: u8 = 4;

    const fn part(commanded: bool, lit: u8, known: u8, fresh: bool) -> ChannelPartLit {
        ChannelPartLit { lit: Some(ClusterGroupLitView { commanded, lit, known, members: MEMBERS }), fresh }
    }

    #[test]
    fn a_channel_is_on_when_a_node_was_commanded_on_and_has_a_lit_member() {
        let state = channel_state([part(true, 1, 1, true), part(false, 0, MEMBERS, true)]);
        assert_eq!(state.on, Some(true));
        assert_eq!((state.lit, state.known, state.members), (1, 1 + MEMBERS, 2 * MEMBERS));
    }

    #[test]
    fn commanded_without_a_lit_member_and_lit_without_a_command_are_both_not_on() {
        let commanded_dark = part(true, 0, MEMBERS, true);
        let lit_uncommanded = part(false, MEMBERS, MEMBERS, true);
        assert_eq!(channel_state([commanded_dark]).on, Some(false), "a command alone is not on");
        assert_eq!(channel_state([lit_uncommanded]).on, None, "a lit member alone is not on, nor off");
        assert_eq!(channel_state([commanded_dark, lit_uncommanded]).on, None, "nor are they across nodes");
    }

    #[test]
    fn a_part_is_off_only_when_every_member_is_known_dark() {
        assert_eq!(channel_state([part(false, 0, MEMBERS, true)]).on, Some(false), "every member known dark");
        let one_unknown = channel_state([part(false, 0, MEMBERS - 1, true)]);
        assert_eq!(one_unknown.on, None, "a missing command does not prove a member of unknown state dark");
        assert_eq!(one_unknown.parts_unknown, 1);
    }

    #[test]
    fn a_part_with_no_members_known_is_unknown_never_off() {
        let empty = ChannelPartLit { lit: Some(ClusterGroupLitView { commanded: false, lit: 0, known: 0, members: 0 }), fresh: true };
        assert_eq!(channel_state([empty]).on, None, "an unread membership looks empty and proves nothing dark");
    }

    #[test]
    fn a_stale_node_makes_the_channel_unknown_and_never_off() {
        let state = channel_state([part(false, 0, MEMBERS, true), part(false, 0, MEMBERS, false)]);
        assert_eq!(state.on, None);
        assert_eq!(state.parts_unknown, 1);
        assert_eq!((state.known, state.members), (MEMBERS, 2 * MEMBERS), "the stale members are counted, not known");
    }

    #[test]
    fn a_stale_node_cannot_light_the_channel() {
        assert_eq!(channel_state([part(true, MEMBERS, MEMBERS, false)]).on, None);
    }

    #[test]
    fn a_commanded_part_with_members_of_unknown_state_is_unknown() {
        let state = channel_state([part(true, 0, MEMBERS - 1, true)]);
        assert_eq!(state.on, None);
        assert_eq!(state.parts_unknown, 1);
    }

    #[test]
    fn an_unheard_part_and_an_empty_channel_are_unknown() {
        assert_eq!(channel_state([ChannelPartLit::UNHEARD]).on, None);
        assert_eq!(channel_state([]).on, None);
        assert_eq!(channel_state([ChannelPartLit::UNHEARD]).members, 0);
    }
}
