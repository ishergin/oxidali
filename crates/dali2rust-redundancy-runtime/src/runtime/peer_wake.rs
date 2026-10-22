use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher};
use dali2rust_contracts::msg::ConfigSlicePersistedEvent;
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

pub const PEER_WAKE_PATH: &str = "/api/v1/redundancy/config-changed";

pub const PEER_WAKE_QUIET_MS: u32 = 200;

#[derive(Debug, Default)]
pub struct PersistedConfigMark {
    raised: AtomicBool,
    last_ms: AtomicU32,
}

impl PersistedConfigMark {
    #[must_use]
    pub fn note(&self, now_ms: u32) -> bool {
        self.last_ms.store(now_ms, Ordering::Release);
        !self.raised.swap(true, Ordering::AcqRel)
    }

    pub fn lower(&self) {
        self.raised.store(false, Ordering::Release);
    }

    #[must_use]
    pub fn is_raised(&self) -> bool {
        self.raised.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn quiet_for(&self, now_ms: u32) -> u32 {
        now_ms.wrapping_sub(self.last_ms.load(Ordering::Acquire))
    }
}

#[must_use]
pub fn should_wake(enabled: bool, application_active: bool, peer_url: &str) -> bool {
    enabled && application_active && !peer_url.is_empty()
}

pub struct WakeOnPersistSliceStore {
    inner: Arc<dyn SliceStore>,
    transferable: Vec<SliceKey>,
    mark: Arc<PersistedConfigMark>,
    publisher: BusPublisher,
    bus_id: BusId,
}

impl WakeOnPersistSliceStore {
    #[must_use]
    pub fn new(
        inner: Arc<dyn SliceStore>,
        transferable: Vec<SliceKey>,
        mark: Arc<PersistedConfigMark>,
        publisher: BusPublisher,
        bus_id: BusId,
    ) -> Self {
        Self {
            inner,
            transferable,
            mark,
            publisher,
            bus_id,
        }
    }

    fn note_persisted(&self) {
        if !self.mark.note(monotonic_ms()) {
            return;
        }
        let ev = dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            CORRELATION_NONE,
            self.bus_id.0,
            None,
            ConfigSlicePersistedEvent {},
        );
        let _ = self.publisher.try_publish(BusChannel::Events, BusFrame::event(ev));
    }
}

impl SliceStore for WakeOnPersistSliceStore {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.inner.load(key)
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        let session = self.inner.begin_write(key)?;
        if !self.transferable.contains(&key) {
            return Ok(session);
        }
        Ok(Box::new(WatchedWrite {
            session,
            store: self,
        }))
    }
}

struct WatchedWrite<'a> {
    session: Box<dyn SliceWriteSession + 'a>,
    store: &'a WakeOnPersistSliceStore,
}

impl SliceWriteSession for WatchedWrite<'_> {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.session.append(chunk)
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        let Self { session, store } = *self;
        session.commit()?;
        store.note_persisted();
        Ok(())
    }

    fn abort(self: Box<Self>) {
        self.session.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_active_controller_in_an_enabled_pair_with_a_peer_wakes_it() {
        assert!(should_wake(true, true, "http://peer"));
        assert!(!should_wake(false, true, "http://peer"));
        assert!(!should_wake(true, false, "http://peer"));
        assert!(!should_wake(true, true, ""));
    }

    #[test]
    fn only_the_first_save_of_a_burst_raises_the_mark() {
        let mark = PersistedConfigMark::default();
        assert!(mark.note(10));
        assert!(!mark.note(20));
        assert!(mark.is_raised());
        mark.lower();
        assert!(!mark.is_raised());
        assert!(mark.note(30));
    }

    #[test]
    fn every_save_moves_the_quiet_clock_even_when_the_mark_is_already_up() {
        let mark = PersistedConfigMark::default();
        let _ = mark.note(100);
        let _ = mark.note(250);
        assert_eq!(mark.quiet_for(300), 50);
    }

    #[test]
    fn the_quiet_clock_survives_the_millisecond_counter_wrapping() {
        let mark = PersistedConfigMark::default();
        let _ = mark.note(u32::MAX - 10);
        assert_eq!(mark.quiet_for(5), 16);
    }
}
