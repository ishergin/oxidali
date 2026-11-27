use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::SystemTime;

use dali2rust_platform::cluster_link::{ClusterLinkBinder, NoClusterLink};

use super::counters::ClusterCounters;
use super::shared::ClusterPresence;
use crate::presence::PresenceTiming;

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
pub struct ClusterSeams {
    pub presence: Arc<ClusterPresence>,
    pub counters: Arc<ClusterCounters>,
}

impl ClusterSeams {
    pub fn new(composition: &ClusterComposition) -> Self {
        Self {
            presence: Arc::new(ClusterPresence::new(composition.timing, composition.boot_epoch)),
            counters: Arc::new(ClusterCounters::default()),
        }
    }
}
