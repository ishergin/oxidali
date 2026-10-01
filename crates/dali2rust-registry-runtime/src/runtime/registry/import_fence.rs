use std::sync::Mutex;
use std::time::{Duration, Instant};

use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

pub(crate) const IMPORT_FENCE_WINDOW: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct ImportFence {
    pending: Mutex<Vec<(SliceKey, Instant)>>,
}

impl ImportFence {
    pub(crate) fn raise(&self, key: SliceKey) {
        let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.retain(|(held, _)| *held != key);
        pending.push((key, Instant::now()));
    }

    pub(crate) fn lower_all(&self) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn held(&self) -> Vec<SliceKey> {
        let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.retain(|(_, raised)| raised.elapsed() < IMPORT_FENCE_WINDOW);
        pending.iter().map(|(key, _)| *key).collect()
    }

    pub(crate) fn guard<'a>(&self, slices: &'a dyn SliceStore) -> FencedSlices<'a> {
        FencedSlices { inner: slices, held: self.held() }
    }
}

pub(crate) struct FencedSlices<'a> {
    inner: &'a dyn SliceStore,
    held: Vec<SliceKey>,
}

impl SliceStore for FencedSlices<'_> {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.inner.load(key)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        if self.held.contains(&key) {
            return Err(StoreError::Backend("an imported slice waits for its reload".to_string()));
        }
        self.inner.begin_write(key)
    }
}
