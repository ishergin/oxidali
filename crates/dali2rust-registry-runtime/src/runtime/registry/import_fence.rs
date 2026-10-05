use std::sync::Mutex;
use std::time::{Duration, Instant};

use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

pub(crate) const IMPORT_FENCE_WINDOW: Duration = Duration::from_secs(30);

struct Raised {
    key: SliceKey,
    generation: u64,
    at: Instant,
}

#[derive(Default)]
struct FenceState {
    raised: Vec<Raised>,
    generation: u64,
}

#[derive(Default)]
pub(crate) struct ImportFence {
    state: Mutex<FenceState>,
}

impl ImportFence {
    fn state(&self) -> std::sync::MutexGuard<'_, FenceState> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn raise(&self, key: SliceKey) {
        let mut state = self.state();
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        state.raised.retain(|raised| raised.key != key);
        state.raised.push(Raised { key, generation, at: Instant::now() });
    }

    pub(crate) fn generation(&self) -> u64 {
        self.state().generation
    }

    pub(crate) fn lower_up_to(&self, generation: u64) {
        self.state().raised.retain(|raised| raised.generation > generation);
    }

    fn holds(&self, key: SliceKey) -> bool {
        let mut state = self.state();
        state.raised.retain(|raised| raised.at.elapsed() < IMPORT_FENCE_WINDOW);
        state.raised.iter().any(|raised| raised.key == key)
    }

    pub(crate) fn guard<'a>(&'a self, slices: &'a dyn SliceStore) -> FencedSlices<'a> {
        FencedSlices { inner: slices, fence: self }
    }
}

pub(crate) struct FencedSlices<'a> {
    inner: &'a dyn SliceStore,
    fence: &'a ImportFence,
}

impl SliceStore for FencedSlices<'_> {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.inner.load(key)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        if self.fence.holds(key) {
            return Err(StoreError::Deferred);
        }
        self.inner.begin_write(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> dali2rust_bsp::slice_store_files::InMemorySliceStore {
        dali2rust_bsp::slice_store_files::InMemorySliceStore::new()
    }

    #[test]
    fn a_fence_raised_after_the_flush_started_still_stops_its_write() {
        let slices = store();
        let fence = ImportFence::default();
        let flush = fence.guard(&slices);
        fence.raise(SliceKey::PollerSettings);
        assert!(matches!(
            flush.begin_write(SliceKey::PollerSettings),
            Err(StoreError::Deferred)
        ));
        assert!(flush.begin_write(SliceKey::ControllerSettings).is_ok());
    }

    #[test]
    fn a_reload_lowers_only_the_fences_raised_before_its_snapshot() {
        let slices = store();
        let fence = ImportFence::default();
        fence.raise(SliceKey::PollerSettings);
        let before_snapshot = fence.generation();
        fence.raise(SliceKey::ControllerSettings);
        fence.lower_up_to(before_snapshot);
        let flush = fence.guard(&slices);
        assert!(flush.begin_write(SliceKey::PollerSettings).is_ok());
        assert!(matches!(
            flush.begin_write(SliceKey::ControllerSettings),
            Err(StoreError::Deferred)
        ));
    }
}
