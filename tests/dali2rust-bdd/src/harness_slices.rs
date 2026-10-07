use std::sync::Mutex;

use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

#[derive(Debug, Default)]
pub struct HarnessSlices {
    stored: InMemorySliceStore,
    unreadable: Mutex<Vec<SliceKey>>,
}

impl HarnessSlices {
    pub fn fail_reads_of(&self, key: SliceKey) {
        self.unreadable.lock().expect("unreadable lock").push(key);
    }

    pub fn read_everything(&self) {
        self.unreadable.lock().expect("unreadable lock").clear();
    }
}

impl SliceStore for HarnessSlices {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        if self.unreadable.lock().expect("unreadable lock").contains(&key) {
            return Err(StoreError::Backend(format!("{}: flash read failed", key.label())));
        }
        self.stored.load(key)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        self.stored.begin_write(key)
    }
}
