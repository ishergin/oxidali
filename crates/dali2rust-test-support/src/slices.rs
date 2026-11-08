use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

#[derive(Debug, Default)]
pub struct UnreadableSlices {
    stored: InMemorySliceStore,
    unreadable: Mutex<Vec<SliceKey>>,
}

impl UnreadableSlices {
    pub fn fail_reads_of(&self, key: SliceKey) {
        self.unreadable.lock().expect("unreadable lock").push(key);
    }

    pub fn read_everything(&self) {
        self.unreadable.lock().expect("unreadable lock").clear();
    }
}

impl SliceStore for UnreadableSlices {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Idle,
    Armed(SliceKey),
    Holding(SliceKey),
}

pub struct HeldSliceWrite {
    inner: Arc<dyn SliceStore>,
    hold: Mutex<Hold>,
    released: Condvar,
    deadline: Duration,
}

impl HeldSliceWrite {
    pub fn over(inner: Arc<dyn SliceStore>, deadline: Duration) -> Self {
        Self {
            inner,
            hold: Mutex::new(Hold::Idle),
            released: Condvar::new(),
            deadline,
        }
    }

    pub fn hold_next_write_of(&self, key: SliceKey) {
        *self.hold.lock().expect("hold lock") = Hold::Armed(key);
    }
}

impl std::fmt::Debug for HeldSliceWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldSliceWrite")
            .field("hold", &self.hold)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl SliceStore for HeldSliceWrite {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        let loaded = self.inner.load(key);
        let mut hold = self.hold.lock().expect("hold lock");
        if *hold == Hold::Holding(key) {
            *hold = Hold::Idle;
            self.released.notify_all();
        }
        loaded
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        let mut hold = self.hold.lock().expect("hold lock");
        if *hold == Hold::Armed(key) {
            *hold = Hold::Holding(key);
            hold = self
                .released
                .wait_timeout_while(hold, self.deadline, |held| *held == Hold::Holding(key))
                .expect("hold lock")
                .0;
            *hold = Hold::Idle;
        }
        drop(hold);
        self.inner.begin_write(key)
    }
}
