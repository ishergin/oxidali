mod cache;
mod own;
mod set;

#[cfg(test)]
mod tests;

use core::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_platform::liveness::monotonic_ms;

pub use cache::{InputArrival, InputCache, InputHeard, InputTally, NodeInputs};
pub(crate) use cache::placed_input_cache;
pub(crate) use own::{fingerprint, placed_own_inputs};
pub use own::{OwnIdentity, OwnInputs};
pub use set::{HeldDevice, HeldInstance, HeldOwner, HeldValue, InputSet, DEVICE_SLOTS, INSTANCE_SLOTS};

use crate::frame::{InputEvent, InputInfoPage, NODE_COUNT};
use crate::presence::{elapsed_ms, PresenceTiming};

pub const STALE_REFRESH_PERIODS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputsHeldView {
    pub node: u8,
    pub mac: [u8; 6],
    pub set: u32,
    pub heard_ms_ago: u32,
    pub fresh: bool,
    pub devices: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedValue {
    pub event_info: u16,
    pub value: Option<u16>,
    pub age_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedInstance {
    pub instance: u8,
    pub instance_type: Option<u8>,
    pub groups: [Option<u8>; 3],
    pub last: Option<CachedValue>,
}

pub trait ClusterInputsReadPort: Send + Sync {
    fn inputs_held(&self, node: u8) -> Option<InputsHeldView>;

    fn cached_input_owner(&self, node: u8) -> Option<HeldOwner>;

    fn cached_input_device(&self, node: u8, index: usize) -> Option<HeldDevice>;

    fn cached_input_instance(&self, node: u8, device_index: usize, index: usize) -> Option<CachedInstance>;
}

const NODES: usize = NODE_COUNT as usize;

pub struct ClusterInputs {
    cache: Mutex<PsramBox<InputCache>>,
    eras: [AtomicU32; NODES],
    timing: PresenceTiming,
}

impl ClusterInputs {
    pub fn new(timing: PresenceTiming) -> Self {
        Self { cache: Mutex::new(placed_input_cache()), eras: [const { AtomicU32::new(0) }; NODES], timing }
    }

    fn lock(&self) -> MutexGuard<'_, PsramBox<InputCache>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn era(&self, node: u8) -> u32 {
        self.eras.get(usize::from(node)).map_or(0, |era| era.load(Relaxed))
    }

    pub(crate) fn take_page(&self, at: InputArrival, page: &InputInfoPage<'_>, era: u32) {
        let mut cache = self.lock();
        if self.era(at.node) == era {
            cache.take_page(at, page);
        }
    }

    pub(crate) fn hear_event(&self, at: InputArrival, seq: u32, event: &InputEvent, era: u32) -> Option<InputHeard> {
        let mut cache = self.lock();
        (self.era(at.node) == era).then(|| cache.hear_event(at, seq, event))
    }

    pub(crate) fn forget(&self, node: u8) {
        let mut cache = self.lock();
        if let Some(era) = self.eras.get(usize::from(node)) {
            era.fetch_add(1, Relaxed);
        }
        cache.forget(node);
    }

    pub(crate) fn note_run(&self, at: InputArrival) {
        self.lock().note_run(at);
    }

    pub(crate) fn clear(&self) {
        let mut cache = self.lock();
        self.eras.iter().for_each(|era| {
            era.fetch_add(1, Relaxed);
        });
        cache.clear();
    }

    pub(crate) fn hold_in_reach(&self, now_ms: u32) {
        self.lock().hold_in_reach(now_ms);
    }

    pub(crate) fn digest(&self, now_ms: u32) -> u64 {
        let cache = self.lock();
        let mut hasher = DefaultHasher::new();
        cache.changes().hash(&mut hasher);
        for node in 0..NODE_COUNT {
            let held = cache.node(node).and_then(|inputs| self.held_view(node, inputs, now_ms));
            held.map(|view| (view.mac, view.set, view.fresh)).hash(&mut hasher);
        }
        hasher.finish()
    }

    const fn stale_after_ms(&self) -> u32 {
        self.timing.info_refresh_ms().saturating_mul(STALE_REFRESH_PERIODS)
    }

    fn held_view(&self, node: u8, inputs: &NodeInputs, now_ms: u32) -> Option<InputsHeldView> {
        let shown = inputs.shown?;
        let heard_ms_ago = elapsed_ms(now_ms, shown.heard_ms);
        let fresh = heard_ms_ago <= self.stale_after_ms();
        let devices = inputs.shown_set().map_or(0, InputSet::device_count);
        Some(InputsHeldView { node, mac: shown.id.mac, set: shown.id.set, heard_ms_ago, fresh, devices })
    }
}

fn cached(held: HeldInstance, now_ms: u32) -> CachedInstance {
    let last = held.last.map(|last| CachedValue {
        event_info: last.event_info,
        value: last.value,
        age_ms: elapsed_ms(now_ms, last.heard_ms),
    });
    CachedInstance { instance: held.instance, instance_type: held.instance_type, groups: held.groups, last }
}

impl ClusterInputsReadPort for ClusterInputs {
    fn inputs_held(&self, node: u8) -> Option<InputsHeldView> {
        let cache = self.lock();
        self.held_view(node, cache.node(node)?, monotonic_ms())
    }

    fn cached_input_owner(&self, node: u8) -> Option<HeldOwner> {
        self.lock().node(node)?.shown_set()?.owner().cloned()
    }

    fn cached_input_device(&self, node: u8, index: usize) -> Option<HeldDevice> {
        self.lock().node(node)?.shown_set()?.device_at(index).cloned()
    }

    fn cached_input_instance(&self, node: u8, device_index: usize, index: usize) -> Option<CachedInstance> {
        let held = self.lock().node(node)?.shown_set()?.instance_at(device_index, index)?;
        Some(cached(held, monotonic_ms()))
    }
}
