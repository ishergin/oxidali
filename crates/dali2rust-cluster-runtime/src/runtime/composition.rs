use std::collections::hash_map::{DefaultHasher, RandomState};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dali2rust_platform::cluster_link::{ClusterLinkBinder, NoClusterLink};
use dali2rust_platform::liveness::monotonic_ms;

use super::counters::ClusterCounters;
use super::shared::ClusterPresence;
use crate::info::ClusterInfo;
use crate::presence::PresenceTiming;

pub type EpochSource = Arc<dyn Fn() -> u32 + Send + Sync>;

const MICROS_PER_MILLI: u64 = 1_000;

#[derive(Clone)]
pub struct ClusterComposition {
    pub binder: Arc<dyn ClusterLinkBinder>,
    pub timing: PresenceTiming,
    pub boot_epoch: EpochSource,
}

impl ClusterComposition {
    pub fn production(binder: Arc<dyn ClusterLinkBinder>, boot_epoch: EpochSource) -> Self {
        Self { binder, timing: PresenceTiming::PRODUCTION, boot_epoch }
    }
}

impl Default for ClusterComposition {
    fn default() -> Self {
        Self::production(Arc::new(NoClusterLink), Arc::new(host_boot_epoch))
    }
}

pub fn fixed_boot_epoch(epoch: u32) -> EpochSource {
    Arc::new(move || epoch)
}

pub fn mixed_boot_epoch(random: u32, since_boot_us: u64, wall_clock: Duration) -> u32 {
    let mut hasher = DefaultHasher::new();
    (random, since_boot_us, wall_clock).hash(&mut hasher);
    folded(hasher.finish())
}

pub fn wall_clock_now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

fn host_boot_epoch() -> u32 {
    let random = folded(RandomState::new().hash_one(()));
    let since_boot_us = u64::from(monotonic_ms()).saturating_mul(MICROS_PER_MILLI);
    mixed_boot_epoch(random, since_boot_us, wall_clock_now())
}

const fn folded(hashed: u64) -> u32 {
    let [low, high] = [hashed as u32, (hashed >> u32::BITS) as u32];
    low ^ high
}

#[derive(Clone)]
pub struct ClusterSeams {
    pub presence: Arc<ClusterPresence>,
    pub counters: Arc<ClusterCounters>,
    pub info: Arc<ClusterInfo>,
}

impl ClusterSeams {
    pub fn new(composition: &ClusterComposition) -> Self {
        Self {
            presence: Arc::new(ClusterPresence::new(composition.timing, Arc::clone(&composition.boot_epoch))),
            counters: Arc::new(ClusterCounters::default()),
            info: Arc::new(ClusterInfo::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STUCK_RANDOM: u32 = 0x5eed_5eed;
    const SINCE_BOOT_US: u64 = 4_200_000;
    const WALL_CLOCK: Duration = Duration::from_secs(1_790_000_000);
    const ONE_TICK: Duration = Duration::from_micros(1);

    #[test]
    fn an_epoch_from_a_stuck_random_source_still_moves_with_the_timer_and_the_wall_clock() {
        let epoch = mixed_boot_epoch(STUCK_RANDOM, SINCE_BOOT_US, WALL_CLOCK);
        assert_eq!(epoch, mixed_boot_epoch(STUCK_RANDOM, SINCE_BOOT_US, WALL_CLOCK));
        assert_ne!(epoch, mixed_boot_epoch(STUCK_RANDOM, SINCE_BOOT_US + 1, WALL_CLOCK));
        assert_ne!(epoch, mixed_boot_epoch(STUCK_RANDOM, SINCE_BOOT_US, WALL_CLOCK + ONE_TICK));
        assert_ne!(epoch, mixed_boot_epoch(STUCK_RANDOM + 1, SINCE_BOOT_US, WALL_CLOCK));
    }
}
