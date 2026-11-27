use std::sync::Arc;

use dali2rust_api::http::cluster_info_state::{ClusterInfoHttpState, ClusterInfoNodeDto, ClusterInfoSource};
use dali2rust_api::http::cluster_parts_state::{cluster_part_view_to_dto, ClusterPartDto};
use dali2rust_api::http::cluster_records_state::{cluster_record_view_to_dto, ClusterRecordDto};
use dali2rust_cluster_runtime::info::{ClusterInfoReadPort, InfoHeldView};
use dali2rust_cluster_runtime::ClusterPresenceReadPort;
use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{
    ClusterPartKeys, ClusterPartReadPort, ClusterRecordNumbers, ClusterRecordReadPort, ClusterSettingsReadPort,
};

pub(crate) struct ClusterRegistryPorts {
    pub settings: Arc<dyn ClusterSettingsReadPort>,
    pub records: Arc<dyn ClusterRecordReadPort>,
    pub parts: Arc<dyn ClusterPartReadPort>,
}

pub(crate) struct ClusterInfoBridge {
    registry: ClusterRegistryPorts,
    info: Arc<dyn ClusterInfoReadPort>,
    presence: Arc<dyn ClusterPresenceReadPort>,
}

enum Home {
    Own,
    Other(ClusterInfoSource),
}

impl ClusterInfoBridge {
    pub(crate) fn new(
        registry: ClusterRegistryPorts,
        info: Arc<dyn ClusterInfoReadPort>,
        presence: Arc<dyn ClusterPresenceReadPort>,
    ) -> Self {
        Self { registry, info, presence }
    }

    fn own_node(&self) -> u8 {
        self.registry.settings.cluster_settings_view().node_number
    }

    fn home(&self, node: u8) -> Home {
        if node == self.own_node() {
            return Home::Own;
        }
        let heard = self.presence.heard_node(node).is_some_and(|view| view.live);
        Home::Other(if heard { ClusterInfoSource::Cached } else { ClusterInfoSource::Stale })
    }

    fn own_info(&self, node: u8) -> ClusterInfoNodeDto {
        let (channels, channel_parts) = self.own_counts(ClusterRecordKind::Channel);
        let (scenes, scene_parts) = self.own_counts(ClusterRecordKind::Scene);
        let source = ClusterInfoSource::Own;
        ClusterInfoNodeDto { node, source, set: None, age_ms: None, channels, scenes, channel_parts, scene_parts, skipped: 0 }
    }

    fn own_counts(&self, kind: ClusterRecordKind) -> (u8, u8) {
        let records = self.registry.records.cluster_record_numbers(kind).iter().count();
        let parts = self.registry.parts.cluster_part_keys(kind).iter().count();
        (saturating_u8(records), saturating_u8(parts))
    }
}

fn saturating_u8(count: usize) -> u8 {
    u8::try_from(count).unwrap_or(u8::MAX)
}

fn cached_info(held: InfoHeldView, source: ClusterInfoSource) -> ClusterInfoNodeDto {
    ClusterInfoNodeDto {
        node: held.node,
        source,
        set: Some(held.set),
        age_ms: Some(held.age_ms),
        channels: held.channels,
        scenes: held.scenes,
        channel_parts: held.channel_parts,
        scene_parts: held.scene_parts,
        skipped: held.skipped,
    }
}

impl ClusterInfoHttpState for ClusterInfoBridge {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto> {
        match self.home(node) {
            Home::Own => Some(self.own_info(node)),
            Home::Other(source) => self.info.info_held(node).map(|held| cached_info(held, source)),
        }
    }

    fn record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        match self.home(node) {
            Home::Own => self.registry.records.cluster_record(kind, number).map(cluster_record_view_to_dto),
            Home::Other(source) => {
                let view = self.info.info_record(node, kind, number)?;
                Some(ClusterRecordDto { source, ..cluster_record_view_to_dto(view) })
            }
        }
    }

    fn record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        match self.home(node) {
            Home::Own => self.registry.records.cluster_record_numbers(kind),
            Home::Other(_) => self.info.info_record_numbers(node, kind),
        }
    }

    fn part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto> {
        let view = match self.home(node) {
            Home::Own => self.registry.parts.cluster_part(kind, adapter, local),
            Home::Other(_) => self.info.info_part(node, kind, adapter, local),
        };
        view.map(cluster_part_view_to_dto)
    }

    fn part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys {
        match self.home(node) {
            Home::Own => self.registry.parts.cluster_part_keys(kind),
            Home::Other(_) => self.info.info_part_keys(node, kind),
        }
    }
}
