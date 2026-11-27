use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::SystemTime;

use dali2rust_api::http::cluster_status_state::{
    ClusterIgnoredDto, ClusterNodeDto, ClusterOwnDto, ClusterStatusHttpState, ClusterWireOwnerDto,
};
use dali2rust_bus::{BusId, BusPublisher, BusSubscriberRx};
use dali2rust_cluster_runtime::presence::{IgnoredReason, OwnMode, PresenceTiming, WireOwner};
use dali2rust_cluster_runtime::{
    spawn_cluster_worker, ClusterCounters, ClusterPresence, ClusterPresenceReadPort,
    ClusterWorkerPorts, ClusterWorkerThreads, IgnoredView, NodeView, OwnStatus,
};
use dali2rust_platform::cluster_link::{ClusterLinkBinder, NoClusterLink};
use dali2rust_platform::net::{ipv4_text, node_name, NetworkLink};

use super::RegistryStore;

#[derive(Clone)]
pub struct ClusterComposition {
    pub binder: Arc<dyn ClusterLinkBinder>,
    pub timing: PresenceTiming,
    pub boot_epoch: u32,
}

impl ClusterComposition {
    pub fn production(binder: Arc<dyn ClusterLinkBinder>, boot_epoch: u32) -> Self {
        Self { binder, timing: PresenceTiming::PRODUCTION, boot_epoch }
    }
}

impl Default for ClusterComposition {
    fn default() -> Self {
        Self::production(Arc::new(NoClusterLink), host_boot_epoch())
    }
}

pub fn host_boot_epoch() -> u32 {
    let hashed = RandomState::new().hash_one(SystemTime::now());
    let [low, high] = [hashed as u32, (hashed >> u32::BITS) as u32];
    low ^ high
}

#[derive(Clone)]
pub(crate) struct ClusterSeams {
    pub presence: Arc<ClusterPresence>,
    pub counters: Arc<ClusterCounters>,
}

impl ClusterSeams {
    pub(crate) fn new(composition: &ClusterComposition) -> Self {
        Self {
            presence: Arc::new(ClusterPresence::new(composition.timing, composition.boot_epoch)),
            counters: Arc::new(ClusterCounters::default()),
        }
    }
}

pub(crate) struct ClusterSources {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub store: Arc<RegistryStore>,
    pub network: Option<Arc<dyn NetworkLink>>,
    pub binder: Arc<dyn ClusterLinkBinder>,
}

pub(crate) fn spawn_cluster(
    inbox: BusSubscriberRx,
    seams: &ClusterSeams,
    sources: ClusterSources,
) -> ClusterWorkerThreads {
    let ClusterSources { publisher, bus_id, store, network, binder } = sources;
    spawn_cluster_worker(
        inbox,
        ClusterWorkerPorts {
            publisher,
            bus_id,
            settings: Arc::clone(&store) as _,
            home_assistant: Arc::clone(&store) as _,
            role: store as _,
            network,
            binder,
            presence: Arc::clone(&seams.presence),
            counters: Arc::clone(&seams.counters),
        },
    )
}

pub(crate) struct ClusterStatusBridge {
    port: Arc<dyn ClusterPresenceReadPort>,
}

impl ClusterStatusBridge {
    pub(crate) fn new(port: Arc<dyn ClusterPresenceReadPort>) -> Self {
        Self { port }
    }
}

impl ClusterStatusHttpState for ClusterStatusBridge {
    fn own(&self) -> ClusterOwnDto {
        own_dto(&self.port.own_status())
    }

    fn node(&self, node: u8) -> Option<ClusterNodeDto> {
        self.port.heard_node(node).map(|view| node_dto(&view))
    }

    fn ignored(&self, index: usize) -> Option<ClusterIgnoredDto> {
        self.port.ignored_board(index).map(|view| ignored_dto(&view))
    }
}

fn own_dto(status: &OwnStatus) -> ClusterOwnDto {
    ClusterOwnDto {
        state: mode_name(status.mode),
        node_number: status.node,
        wire: status.wire,
        board: status.mac.map(node_name),
        boot_epoch: status.boot_epoch,
        wire_owner: wire_owner_dto(status),
        split_pair: status.faults.split_pair,
        wire_chain: status.faults.wire_chain,
        number_held_by: status.held_by.map(node_name),
    }
}

fn wire_owner_dto(status: &OwnStatus) -> Option<ClusterWireOwnerDto> {
    match status.wire_owner {
        WireOwner::Nobody => None,
        WireOwner::ThisNode => Some(ClusterWireOwnerDto {
            node: status.node,
            board: status.mac.map(node_name).unwrap_or_default(),
            this_node: true,
        }),
        WireOwner::Board { node, mac } => {
            Some(ClusterWireOwnerDto { node, board: node_name(mac), this_node: false })
        }
    }
}

fn node_dto(view: &NodeView) -> ClusterNodeDto {
    ClusterNodeDto {
        node: view.node,
        board: node_name(view.mac),
        address: ipv4_text(view.source),
        installation: view.installation.as_str().to_owned(),
        wire: view.wire,
        wire_protection: view.wire_protection,
        home_assistant_face: view.ha_face,
        heard: view.live,
        age_ms: view.age_ms,
        sequence_gaps: view.sequence_gaps,
        split: view.split,
    }
}

fn ignored_dto(view: &IgnoredView) -> ClusterIgnoredDto {
    let other_cluster = view.reason == IgnoredReason::OtherCluster;
    ClusterIgnoredDto {
        board: node_name(view.mac),
        address: ipv4_text(view.source),
        node: view.node,
        reason: reason_name(view.reason),
        cluster_name: other_cluster.then(|| view.cluster_name.as_str().to_owned()),
        age_ms: view.age_ms,
    }
}

const fn mode_name(mode: OwnMode) -> &'static str {
    match mode {
        OwnMode::Off => "off",
        OwnMode::NoAddress => "no_address",
        OwnMode::BindFailed => "bind_failed",
        OwnMode::Standby => "standby",
        OwnMode::Listening => "listening",
        OwnMode::Waiting => "waiting",
        OwnMode::Running => "running",
    }
}

const fn reason_name(reason: IgnoredReason) -> &'static str {
    match reason {
        IgnoredReason::OtherCluster => "other_cluster",
        IgnoredReason::Waiting => "waiting",
        IgnoredReason::NumberHeld => "number_held",
    }
}
