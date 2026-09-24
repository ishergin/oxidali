use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

#[derive(Default)]
pub(crate) struct CountingStore {
    pub(crate) inner: InMemorySliceStore,
    writes: AtomicU32,
    bytes: AtomicU32,
}

impl CountingStore {
    pub(crate) fn write_count(&self) -> u32 {
        self.writes.load(Ordering::Acquire)
    }

    pub(crate) fn bytes_written(&self) -> u32 {
        self.bytes.load(Ordering::Acquire)
    }
}

impl SliceStore for CountingStore {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.inner.load(key)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(CountingSession {
            inner: self.inner.begin_write(key)?,
            bytes: &self.bytes,
        }))
    }
}

struct CountingSession<'a> {
    inner: Box<dyn SliceWriteSession + 'a>,
    bytes: &'a AtomicU32,
}

impl SliceWriteSession for CountingSession<'_> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.bytes
            .fetch_add(u32::try_from(chunk.len()).unwrap_or(u32::MAX), Ordering::AcqRel);
        self.inner.append(chunk)
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        self.inner.commit()
    }

    fn abort(self: Box<Self>) {
        self.inner.abort();
    }
}
