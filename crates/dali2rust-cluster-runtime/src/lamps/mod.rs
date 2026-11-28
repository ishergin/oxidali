mod cache;
mod own;
mod set;
mod staged;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_platform::liveness::monotonic_ms;

pub use cache::{LampArrival, LampCache, NodeLamps};
pub(crate) use cache::placed_lamp_cache;
pub(crate) use own::{fill_info, fill_state};
pub use own::{observation_age_ms, OLDEST_OBSERVATION_AGE_MS};
pub use set::{HeldIdentity, HeldLampInfo, LampInfoSet, LampSet, StateSet, LAMP_SLOTS, PART_SLOTS};
pub use staged::{LampTally, Staged};

use crate::frame::{LampInfoPage, LampState, LampStatePage, PartState};
use crate::presence::{elapsed_ms, PresenceTiming};

pub const LAMP_ADAPTERS: u8 = 1;

pub const STALE_REFRESH_PERIODS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampsHeldView {
    pub node: u8,
    pub mac: [u8; 6],
    pub state_set: Option<u32>,
    pub state_heard_ms_ago: Option<u32>,
    pub info_set: Option<u32>,
    pub info_heard_ms_ago: Option<u32>,
    pub fresh: bool,
    pub lamps: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedLampView {
    pub node: u8,
    pub adapter: u8,
    pub lamp: u8,
    pub mac: [u8; 6],
    pub state: Option<LampState>,
    pub info: Option<HeldLampInfo>,
    pub fresh: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedPartView {
    pub node: u8,
    pub mac: [u8; 6],
    pub state: PartState,
    pub fresh: bool,
}

pub trait ClusterLampsReadPort: Send + Sync {
    fn lamps_held(&self, node: u8) -> Option<LampsHeldView>;

    fn cached_lamp_ids(&self, node: u8, adapter: u8) -> u64;

    fn cached_lamp(&self, node: u8, adapter: u8, lamp: u8) -> Option<CachedLampView>;

    fn cached_part(&self, node: u8, adapter: u8, group: u8) -> Option<CachedPartView>;

    fn cached_identity(&self, node: u8) -> Option<HeldIdentity>;
}

pub struct ClusterLamps {
    cache: Mutex<PsramBox<LampCache>>,
    era: AtomicU32,
    timing: PresenceTiming,
}

impl ClusterLamps {
    pub fn new(timing: PresenceTiming) -> Self {
        Self { cache: Mutex::new(placed_lamp_cache()), era: AtomicU32::new(0), timing }
    }

    fn lock(&self) -> MutexGuard<'_, PsramBox<LampCache>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn era(&self) -> u32 {
        self.era.load(Relaxed)
    }

    pub(crate) fn take_state(&self, at: LampArrival, page: &LampStatePage<'_>, era: u32) -> LampTally {
        let mut cache = self.lock();
        if self.era.load(Relaxed) != era {
            return LampTally::default();
        }
        cache.take_state(at, page)
    }

    pub(crate) fn take_info(&self, at: LampArrival, page: &LampInfoPage<'_>, era: u32) -> LampTally {
        let mut cache = self.lock();
        if self.era.load(Relaxed) != era {
            return LampTally::default();
        }
        cache.take_info(at, page)
    }

    pub(crate) fn forget(&self, node: u8) {
        let mut cache = self.lock();
        self.era.fetch_add(1, Relaxed);
        cache.forget(node);
    }

    pub(crate) fn note_run(&self, at: LampArrival) {
        self.lock().note_run(at);
    }

    pub(crate) fn clear(&self) {
        let mut cache = self.lock();
        self.era.fetch_add(1, Relaxed);
        cache.clear();
    }

    pub(crate) fn changes(&self, now_ms: u32) -> u32 {
        let mut cache = self.lock();
        cache.hold_in_reach(now_ms);
        cache.changes()
    }

    const fn stale_after_ms(&self) -> u32 {
        self.timing.info_refresh_ms().saturating_mul(STALE_REFRESH_PERIODS)
    }

    fn fresh(&self, lamps: &NodeLamps, now_ms: u32) -> bool {
        let heard = lamps.state.shown_mark().map(|shown| elapsed_ms(now_ms, shown.heard_ms));
        heard.is_some_and(|ago_ms| ago_ms <= self.stale_after_ms())
    }

    fn held_view(&self, node: u8, lamps: &NodeLamps, now_ms: u32) -> LampsHeldView {
        let state = lamps.state.shown_mark();
        let info = lamps.info.shown_mark();
        let ids = described_ids(lamps);
        LampsHeldView {
            node,
            mac: lamps.run.map_or([0; 6], |run| run.mac),
            state_set: state.map(|shown| shown.set),
            state_heard_ms_ago: state.map(|shown| elapsed_ms(now_ms, shown.heard_ms)),
            info_set: info.map(|shown| shown.set),
            info_heard_ms_ago: info.map(|shown| elapsed_ms(now_ms, shown.heard_ms)),
            fresh: self.fresh(lamps, now_ms),
            lamps: u8::try_from(ids.count_ones()).unwrap_or(u8::MAX),
        }
    }

    fn lamp_view(&self, (node, adapter, lamp): (u8, u8, u8), lamps: &NodeLamps, now_ms: u32) -> Option<CachedLampView> {
        let heard_ms_ago = lamps.state.shown_mark().map_or(0, |shown| elapsed_ms(now_ms, shown.heard_ms));
        let state = lamps.state.shown().and_then(|set| set.lamp(lamp)).map(|state| LampState {
            age_ms: state.age_ms.map(|age_ms| age_ms.saturating_add(heard_ms_ago)),
            ..state
        });
        let info = lamps.info.shown().and_then(|set| set.lamp(lamp)).cloned();
        if state.is_none() && info.is_none() {
            return None;
        }
        let mac = lamps.run.map_or([0; 6], |run| run.mac);
        Some(CachedLampView { node, adapter, lamp, mac, state, info, fresh: self.fresh(lamps, now_ms) })
    }
}

impl ClusterLampsReadPort for ClusterLamps {
    fn lamps_held(&self, node: u8) -> Option<LampsHeldView> {
        let cache = self.lock();
        Some(self.held_view(node, cache.node(node)?, monotonic_ms()))
    }

    fn cached_lamp_ids(&self, node: u8, adapter: u8) -> u64 {
        let cache = self.lock();
        let Some(lamps) = cache.node(node).filter(|_| adapter < LAMP_ADAPTERS) else {
            return 0;
        };
        described_ids(lamps)
    }

    fn cached_lamp(&self, node: u8, adapter: u8, lamp: u8) -> Option<CachedLampView> {
        let cache = self.lock();
        let lamps = cache.node(node).filter(|_| adapter < LAMP_ADAPTERS)?;
        self.lamp_view((node, adapter, lamp), lamps, monotonic_ms()).filter(|view| view.info.is_some())
    }

    fn cached_part(&self, node: u8, adapter: u8, group: u8) -> Option<CachedPartView> {
        let cache = self.lock();
        let lamps = cache.node(node).filter(|_| adapter < LAMP_ADAPTERS)?;
        let state = lamps.state.shown()?.part(group)?;
        let mac = lamps.run.map_or([0; 6], |run| run.mac);
        Some(CachedPartView { node, mac, state, fresh: self.fresh(lamps, monotonic_ms()) })
    }

    fn cached_identity(&self, node: u8) -> Option<HeldIdentity> {
        self.lock().node(node)?.info.shown()?.owner().cloned()
    }
}

fn described_ids(lamps: &NodeLamps) -> u64 {
    lamps.info.shown().map_or(0, LampInfoSet::lamp_ids)
}
