mod cache;
mod copy_set;
mod set;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{ClusterPartKeys, ClusterPartView, ClusterRecordNumbers, ClusterRecordView};
use dali2rust_platform::liveness::monotonic_ms;

pub use cache::{Arrival, HeldNode, InfoCache, PageTally, SetId};
pub(crate) use cache::{placed_cache, placed_set};
pub(crate) use copy_set::{placed_copy_set, CopySet};
pub use set::{HeldPart, HeldRecord, InfoSet, Kept, BLOCK_SLOTS, PART_SLOTS};

use crate::frame::InfoPage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoHeldView {
    pub node: u8,
    pub mac: [u8; 6],
    pub set: u32,
    pub age_ms: u32,
    pub skipped: u16,
    pub channels: u8,
    pub scenes: u8,
    pub channel_parts: u8,
    pub scene_parts: u8,
}

pub trait ClusterInfoReadPort: Send + Sync {
    fn info_held(&self, node: u8) -> Option<InfoHeldView>;

    fn info_record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView>;

    fn info_record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers;

    fn info_part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView>;

    fn info_part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys;
}

pub struct ClusterInfo {
    cache: Mutex<PsramBox<InfoCache>>,
    era: AtomicU32,
}

impl Default for ClusterInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterInfo {
    pub fn new() -> Self {
        Self { cache: Mutex::new(placed_cache()), era: AtomicU32::new(0) }
    }

    fn lock(&self) -> MutexGuard<'_, PsramBox<InfoCache>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn era(&self) -> u32 {
        self.era.load(Relaxed)
    }

    pub(crate) fn take_page(&self, at: Arrival, page: &InfoPage<'_>, era: u32) -> PageTally {
        let mut cache = self.lock();
        if self.era.load(Relaxed) != era {
            return PageTally::default();
        }
        cache.take_page(at, page)
    }

    pub(crate) fn with_shown<R>(&self, node: u8, now_ms: u32, read: impl FnOnce(&InfoSet, HeldNode) -> R) -> Option<R> {
        let cache = self.lock();
        let held = cache.held(node, now_ms)?;
        cache.shown(node).map(|set| read(set, held))
    }

    pub(crate) fn forget(&self, node: u8) {
        self.lock().forget(node);
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

    fn held_view(cache: &InfoCache, node: u8, now_ms: u32) -> Option<InfoHeldView> {
        let held = cache.held(node, now_ms)?;
        let (channels, channel_parts) = cache.counts(node, ClusterRecordKind::Channel)?;
        let (scenes, scene_parts) = cache.counts(node, ClusterRecordKind::Scene)?;
        let HeldNode { mac, set, age_ms, skipped } = held;
        Some(InfoHeldView { node, mac, set, age_ms, skipped, channels, scenes, channel_parts, scene_parts })
    }
}

impl ClusterInfoReadPort for ClusterInfo {
    fn info_held(&self, node: u8) -> Option<InfoHeldView> {
        Self::held_view(&self.lock(), node, monotonic_ms())
    }

    fn info_record(&self, node: u8, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        self.lock().shown(node)?.record_view(kind, number)
    }

    fn info_record_numbers(&self, node: u8, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        self.lock().shown(node).map(|set| set.record_numbers(kind)).unwrap_or_default()
    }

    fn info_part(&self, node: u8, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        self.lock().shown(node)?.part_view(kind, adapter, local)
    }

    fn info_part_keys(&self, node: u8, kind: ClusterRecordKind) -> ClusterPartKeys {
        self.lock().shown(node).map(|set| set.part_key_set(kind)).unwrap_or_default()
    }
}
