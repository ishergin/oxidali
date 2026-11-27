use std::sync::atomic::Ordering;
use std::sync::Arc;

use dali2rust_cluster_runtime::frame::NODE_COUNT;
use dali2rust_cluster_runtime::presence::OwnMode;
use dali2rust_cluster_runtime::ClusterPresenceReadPort;
use dali2rust_display_runtime::source::{ClusterGlance, ClusterMode, DisplaySample, DisplaySource};
use dali2rust_platform::dali::{DaliWireCounters, PhySnifferCounters, WIRE_LOAD_FULL_PERMILLE};
use dali2rust_platform::wall_clock::{TimeSource, WallClock};
use dali2rust_registry_runtime::RegistryStore;

pub(crate) struct ControllerDisplaySource {
    pub registry: Arc<RegistryStore>,
    pub wire: Arc<DaliWireCounters>,
    pub phy_sniffer: Arc<PhySnifferCounters>,
    pub poller: Arc<dali2rust_poller_runtime::PollerCounters>,
    pub hcl_overrides: dali2rust_hcl_runtime::SharedOverrideLedger,
    pub mqtt: Arc<dali2rust_mqtt_runtime::MqttCounters>,
    pub ws: Arc<dali2rust_ws_runtime::WsCounters>,
    pub clock: Arc<dyn WallClock>,
    pub cluster: Arc<dyn ClusterPresenceReadPort>,
    pub adapter_id: u8,
}

impl DisplaySource for ControllerDisplaySource {
    fn sample(&self) -> DisplaySample {
        let parts = self.registry.display_sample_parts(self.adapter_id);
        let counts = parts.counts;
        DisplaySample {
            adapter_id: self.adapter_id,
            gear_known: counts.gear_known,
            gear_unreachable: counts.gear_unreachable,
            gear_faulted: counts.gear_faulted,
            input_known: counts.input_known,
            input_present: counts.input_present,
            adapter_enabled: parts.adapter_enabled,
            redundancy_enabled: parts.redundancy_enabled,
            controller_active: parts.controller_active,
            controller_quiescent: parts.controller_quiescent,
            controller_identifying: parts.controller_identifying,
            time_synced: self.clock.source() != TimeSource::Unset,
            frames_sent_total: self.frames_sent(),
            foreign_frames_total: self.phy_sniffer.frames.load(Ordering::Relaxed),
            wire_load_permille: self.wire_load_permille(),
            poller_enabled: parts.poller_enabled,
            poller_interval_ms: parts.poller_interval_ms,
            poller_reads_failed: self.poller.reads_failed.load(Ordering::Relaxed),
            poller_reads_absent: self.poller.reads_absent.load(Ordering::Relaxed),
            hcl_overrides_active: self.overrides_active(),
            mqtt_connected: self.mqtt.connected.load(Ordering::Relaxed) != 0,
            ws_clients: self.ws.clients.load(Ordering::Relaxed),
            persistence_failed: self.persistence_failed(),
            display: parts.display,
            cluster: cluster_glance(&*self.cluster),
        }
    }
}

impl ControllerDisplaySource {
    fn wire_load_permille(&self) -> u16 {
        let permille = self.wire.load_permille.load(Ordering::Relaxed);
        u16::try_from(permille.min(WIRE_LOAD_FULL_PERMILLE)).unwrap_or(u16::MAX)
    }

    fn frames_sent(&self) -> u32 {
        self.wire
            .frames_sent_by_priority
            .iter()
            .fold(0u32, |acc, c| acc.saturating_add(c.load(Ordering::Relaxed)))
    }

    fn overrides_active(&self) -> u32 {
        u32::try_from(
            self.hcl_overrides
                .lock()
                .map(|l| l.len())
                .unwrap_or_default(),
        )
        .unwrap_or(u32::MAX)
    }

    fn persistence_failed(&self) -> u32 {
        let c = self.registry.persistence_counters();
        c.flush_error_total
            .load(Ordering::Relaxed)
            .saturating_add(c.no_space_total.load(Ordering::Relaxed))
            .saturating_add(c.hydrate_error_total.load(Ordering::Relaxed))
    }
}

pub(crate) fn cluster_glance(presence: &dyn ClusterPresenceReadPort) -> ClusterGlance {
    let own = presence.own_status();
    let mode = cluster_mode(own.mode);
    if mode == ClusterMode::Off {
        return ClusterGlance::default();
    }
    let (heard, known) = if mode == ClusterMode::Running { heard_of_known(presence, own.node) } else { (0, 0) };
    ClusterGlance { mode, node: own.node, heard, known, pair_split: own.faults.split_pair }
}

fn heard_of_known(presence: &dyn ClusterPresenceReadPort, own_node: u8) -> (u8, u8) {
    (0..NODE_COUNT)
        .filter(|node| *node != own_node)
        .filter_map(|node| presence.heard_node(node))
        .fold((0, 0), |(heard, known), view| (heard + u8::from(view.live), known + 1))
}

const fn cluster_mode(mode: OwnMode) -> ClusterMode {
    match mode {
        OwnMode::Off => ClusterMode::Off,
        OwnMode::NoAddress => ClusterMode::NoAddress,
        OwnMode::BindFailed => ClusterMode::BindFailed,
        OwnMode::Standby => ClusterMode::Standby,
        OwnMode::Listening => ClusterMode::Listening,
        OwnMode::Waiting => ClusterMode::Waiting,
        OwnMode::Running => ClusterMode::Running,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use dali2rust_cluster_runtime::presence::{OwnFaults, WireOwner};
    use dali2rust_cluster_runtime::{IgnoredView, NodeView, OwnStatus};

    use super::*;

    const OWN_NODE: u8 = 2;

    struct FakePresence {
        own: OwnStatus,
        nodes: Vec<NodeView>,
        node_reads: AtomicU32,
    }

    impl FakePresence {
        fn new(mode: OwnMode) -> Self {
            let own = OwnStatus {
                mode,
                node: OWN_NODE,
                wire: OWN_NODE,
                mac: None,
                boot_epoch: None,
                wire_owner: WireOwner::Nobody,
                faults: OwnFaults::default(),
                held_by: None,
            };
            Self { own, nodes: Vec::new(), node_reads: AtomicU32::new(0) }
        }

        fn hearing(mut self, node: u8, live: bool) -> Self {
            self.nodes.push(node_view(node, live));
            self
        }
    }

    fn node_view(node: u8, live: bool) -> NodeView {
        NodeView {
            node,
            mac: [2, 0, 0, 0, 0, node],
            source: [10, 0, 0, node],
            installation: dali2rust_contracts::msg::FixedText32::new(),
            wire: node,
            wire_protection: false,
            ha_face: None,
            live,
            age_ms: 0,
            sequence_gaps: 0,
            split: false,
        }
    }

    impl ClusterPresenceReadPort for FakePresence {
        fn own_status(&self) -> OwnStatus {
            self.own.clone()
        }

        fn heard_node(&self, node: u8) -> Option<NodeView> {
            self.node_reads.fetch_add(1, Ordering::Relaxed);
            self.nodes.iter().find(|view| view.node == node).cloned()
        }

        fn ignored_board(&self, _index: usize) -> Option<IgnoredView> {
            None
        }
    }

    #[test]
    fn a_cluster_that_is_off_says_so_and_reads_no_node() {
        let presence = FakePresence::new(OwnMode::Off).hearing(1, true);
        assert_eq!(cluster_glance(&presence), ClusterGlance::default());
        assert_eq!(presence.node_reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_running_node_counts_the_other_nodes_it_hears_out_of_those_it_knows() {
        let presence = FakePresence::new(OwnMode::Running).hearing(0, true).hearing(5, false).hearing(7, true);
        let glance = cluster_glance(&presence);
        assert_eq!(glance, ClusterGlance { mode: ClusterMode::Running, node: OWN_NODE, heard: 2, known: 3, pair_split: false });
    }

    #[test]
    fn a_rival_holding_this_node_number_is_not_one_of_the_others() {
        let presence = FakePresence::new(OwnMode::Running).hearing(OWN_NODE, true).hearing(1, true);
        let glance = cluster_glance(&presence);
        assert_eq!((glance.heard, glance.known), (1, 1));
        assert_eq!(presence.node_reads.load(Ordering::Relaxed), u32::from(NODE_COUNT) - 1);
    }

    #[test]
    fn a_node_that_is_not_running_keeps_its_mode_and_reads_no_node() {
        for (mode, shown) in [
            (OwnMode::NoAddress, ClusterMode::NoAddress),
            (OwnMode::BindFailed, ClusterMode::BindFailed),
            (OwnMode::Standby, ClusterMode::Standby),
            (OwnMode::Listening, ClusterMode::Listening),
            (OwnMode::Waiting, ClusterMode::Waiting),
        ] {
            let presence = FakePresence::new(mode).hearing(1, true);
            let glance = cluster_glance(&presence);
            assert_eq!(glance, ClusterGlance { mode: shown, node: OWN_NODE, ..ClusterGlance::default() }, "{mode:?}");
            assert_eq!(presence.node_reads.load(Ordering::Relaxed), 0, "{mode:?}");
        }
    }

    #[test]
    fn a_split_pair_travels_with_the_mode() {
        let mut presence = FakePresence::new(OwnMode::Running);
        presence.own.faults = OwnFaults { split_pair: true, wire_chain: false };
        assert!(cluster_glance(&presence).pair_split);
        presence.own.faults = OwnFaults { split_pair: false, wire_chain: true };
        assert!(!cluster_glance(&presence).pair_split, "a wire chain is not shown on the panel");
    }
}
