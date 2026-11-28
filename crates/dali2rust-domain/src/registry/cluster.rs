use core::ops::RangeInclusive;

use dali2rust_contracts::msg::ClusterForwarding;

pub const CLUSTER_NODE_COUNT: u8 = 8;

pub const CLUSTER_NUMBERS_PER_NODE: u8 = 16;

pub const CLUSTER_RECORD_NUMBER_COUNT: u8 = CLUSTER_NODE_COUNT * CLUSTER_NUMBERS_PER_NODE;

pub const MAX_CLUSTER_NAME_BYTES: usize = 64;

pub const DEFAULT_CLUSTER_PORT: u16 = 47810;

#[must_use]
pub const fn is_cluster_node(node: u8) -> bool {
    node < CLUSTER_NODE_COUNT
}

#[must_use]
pub const fn cluster_block(node: u8) -> RangeInclusive<u8> {
    let first = node.saturating_mul(CLUSTER_NUMBERS_PER_NODE);
    first..=first.saturating_add(CLUSTER_NUMBERS_PER_NODE - 1)
}

#[must_use]
pub const fn cluster_node_of(number: u8) -> u8 {
    number / CLUSTER_NUMBERS_PER_NODE
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClusterWire {
    #[default]
    Own,
    SameAs(u8),
}

impl ClusterWire {
    #[must_use]
    pub const fn from_same_as(node: Option<u8>) -> Self {
        match node {
            Some(node) => Self::SameAs(node),
            None => Self::Own,
        }
    }

    #[must_use]
    pub const fn same_as(self) -> Option<u8> {
        match self {
            Self::Own => None,
            Self::SameAs(node) => Some(node),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterSettingsConflict {
    NameRequired,
    WireNamesThisNode,
    WireProtectionRequired,
}

impl ClusterSettingsConflict {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NameRequired => "cluster_name_required",
            Self::WireNamesThisNode => "wire_names_this_node",
            Self::WireProtectionRequired => "wire_protection_required",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterSettingsView {
    pub enabled: bool,
    pub cluster_name: String,
    pub node_number: u8,
    pub port: u16,
    pub wire: ClusterWire,
    pub forwarding: ClusterForwarding,
    pub accept_all_lamps: bool,
    pub send_all_lamps: bool,
    pub accept_single_lamps: bool,
    pub wire_protection: bool,
    pub share_input_events: bool,
}

impl Default for ClusterSettingsView {
    fn default() -> Self {
        Self {
            enabled: false,
            cluster_name: String::new(),
            node_number: 0,
            port: DEFAULT_CLUSTER_PORT,
            wire: ClusterWire::Own,
            forwarding: ClusterForwarding::FACTORY,
            accept_all_lamps: false,
            send_all_lamps: false,
            accept_single_lamps: false,
            wire_protection: true,
            share_input_events: true,
        }
    }
}

impl ClusterSettingsView {
    #[must_use]
    pub const fn block(&self) -> RangeInclusive<u8> {
        cluster_block(self.node_number)
    }

    #[must_use]
    pub const fn shares_wire(&self) -> bool {
        matches!(self.wire, ClusterWire::SameAs(_))
    }

    pub fn conflict(&self) -> Option<ClusterSettingsConflict> {
        if self.enabled && self.cluster_name.is_empty() {
            return Some(ClusterSettingsConflict::NameRequired);
        }
        if self.wire == ClusterWire::SameAs(self.node_number) {
            return Some(ClusterSettingsConflict::WireNamesThisNode);
        }
        let storm = self.shares_wire() && self.forwarding.foreign_master && !self.wire_protection;
        storm.then_some(ClusterSettingsConflict::WireProtectionRequired)
    }

    #[must_use]
    pub fn in_range(&self) -> bool {
        is_cluster_node(self.node_number)
            && self.port != 0
            && self.wire.same_as().is_none_or(is_cluster_node)
            && self.cluster_name.len() <= MAX_CLUSTER_NAME_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_foreign_unprotected() -> ClusterSettingsView {
        ClusterSettingsView {
            wire: ClusterWire::SameAs(1),
            forwarding: ClusterForwarding { foreign_master: true, ..ClusterForwarding::FACTORY },
            wire_protection: false,
            ..ClusterSettingsView::default()
        }
    }

    #[test]
    fn the_factory_settings_are_consistent_and_in_range() {
        let factory = ClusterSettingsView::default();
        assert_eq!(factory.conflict(), None);
        assert!(factory.in_range());
        assert_eq!(factory.block(), 0..=15);
    }

    #[test]
    fn a_node_owns_its_block_of_sixteen_and_the_last_block_ends_the_numbers() {
        assert_eq!(cluster_block(1), 16..=31);
        assert_eq!(*cluster_block(CLUSTER_NODE_COUNT - 1).end(), CLUSTER_RECORD_NUMBER_COUNT - 1);
    }

    #[test]
    fn an_enabled_cluster_needs_a_name_and_a_disabled_one_does_not() {
        let mut view = ClusterSettingsView { enabled: true, ..ClusterSettingsView::default() };
        assert_eq!(view.conflict(), Some(ClusterSettingsConflict::NameRequired));
        view.cluster_name = "dali-cluster".to_string();
        assert_eq!(view.conflict(), None);
    }

    #[test]
    fn a_shared_wire_naming_this_node_is_a_conflict() {
        let view = ClusterSettingsView { node_number: 3, wire: ClusterWire::SameAs(3), ..ClusterSettingsView::default() };
        assert_eq!(view.conflict(), Some(ClusterSettingsConflict::WireNamesThisNode));
    }

    #[test]
    fn the_storm_needs_all_three_of_its_conditions() {
        assert_eq!(shared_foreign_unprotected().conflict(), Some(ClusterSettingsConflict::WireProtectionRequired));
        let own_wire = ClusterSettingsView { wire: ClusterWire::Own, ..shared_foreign_unprotected() };
        assert_eq!(own_wire.conflict(), None);
        let no_foreign = ClusterSettingsView { forwarding: ClusterForwarding::FACTORY, ..shared_foreign_unprotected() };
        assert_eq!(no_foreign.conflict(), None);
        let protected = ClusterSettingsView { wire_protection: true, ..shared_foreign_unprotected() };
        assert_eq!(protected.conflict(), None);
    }

    #[test]
    fn each_field_out_of_range_is_out_of_range() {
        let base = ClusterSettingsView::default();
        let cases = [
            ClusterSettingsView { node_number: CLUSTER_NODE_COUNT, ..base.clone() },
            ClusterSettingsView { port: 0, ..base.clone() },
            ClusterSettingsView { wire: ClusterWire::SameAs(CLUSTER_NODE_COUNT), ..base.clone() },
            ClusterSettingsView { cluster_name: "x".repeat(MAX_CLUSTER_NAME_BYTES + 1), ..base.clone() },
        ];
        for case in cases {
            assert!(!case.in_range(), "{case:?}");
        }
        let longest = ClusterSettingsView { cluster_name: "x".repeat(MAX_CLUSTER_NAME_BYTES), ..base };
        assert!(longest.in_range());
    }
}
