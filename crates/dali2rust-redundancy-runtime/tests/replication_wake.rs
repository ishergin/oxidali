use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    fixed_text_64, BusEventPayload, PeerConfigChangedEvent, RedundancySettingsChangedEvent,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{
    DaliSettingsReadPort, DaliSettingsView, RedundancySettingsReadPort, RedundancySettingsView,
};
use dali2rust_platform::http_fetch::{FetchError, HttpFetch};
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_redundancy_runtime::{
    spawn_replication_worker, PersistedConfigMark, PulledSlice, ReplicationCounters,
    ReplicationDeps, ReplicationSink, SliceDigest, WakeOnPersistSliceStore, PEER_WAKE_PATH,
    REPLICATION_HANDLED_EVENTS,
};
use dali2rust_test_support::{remains_false_for, try_wait_until, write_slice};

const MANIFEST: &str = "/api/v1/config/slices";
const TRANSFERS: SliceKey = SliceKey::PollerSettings;
const STAYS: SliceKey = SliceKey::RedundancySettings;
const BUDGET: Duration = Duration::from_millis(1_500);
const SETTLE: Duration = Duration::from_millis(600);
const BURST: usize = 6;
const BURST_GAP: Duration = Duration::from_millis(25);
const WAKE_FLOOD: usize = 8;

struct Role {
    active: AtomicBool,
}

impl RedundancySettingsReadPort for Role {
    fn redundancy_settings_view(&self) -> RedundancySettingsView {
        RedundancySettingsView {
            enabled: true,
            standby_role: false,
            probe_interval_ms: 250,
            takeover_after_missed: 2,
            boot_listen_ms: 0,
            peer_device_short_address: None,
            peer_url: "http://peer".to_string(),
        }
    }
}

impl DaliSettingsReadPort for Role {
    fn dali_settings_view(&self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: self.active.load(Ordering::Relaxed),
            device_short_address: None,
        }
    }
}

#[derive(Default)]
struct Peer {
    asked: Mutex<Vec<(String, Instant)>>,
    during_post: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl Peer {
    fn count(&self, line: &str) -> usize {
        self.asked.lock().unwrap().iter().filter(|(l, _)| l == line).count()
    }

    fn first(&self, line: &str) -> Option<Instant> {
        self.asked.lock().unwrap().iter().find(|(l, _)| l == line).map(|(_, at)| *at)
    }
}

impl HttpFetch for Peer {
    fn get(&self, _base: &str, path: &str) -> Result<Vec<u8>, FetchError> {
        self.asked.lock().unwrap().push((format!("GET {path}"), Instant::now()));
        Ok(b"[]".to_vec())
    }

    fn post(&self, _base: &str, path: &str) -> Result<(), FetchError> {
        self.asked.lock().unwrap().push((format!("POST {path}"), Instant::now()));
        if let Some(hook) = self.during_post.get() {
            hook();
        }
        Ok(())
    }
}

struct NoSlices;

impl ReplicationSink for NoSlices {
    fn local_digests(&self) -> Vec<SliceDigest> {
        Vec::new()
    }

    fn accepts(&self, _name: &str) -> bool {
        false
    }

    fn family(&self, name: &str) -> String {
        name.to_string()
    }

    fn stage(&self, _pulled: Vec<PulledSlice>) -> bool {
        false
    }

    fn discard_staged(&self) {}
}

struct Pair {
    store: Arc<WakeOnPersistSliceStore>,
    peer: Arc<Peer>,
    persisted: Arc<PersistedConfigMark>,
    counters: Arc<ReplicationCounters>,
    publisher: BusPublisher,
    _host: BusHost,
}

fn wake_line() -> String {
    format!("POST {PEER_WAKE_PATH}")
}

fn manifest_line() -> String {
    format!("GET {MANIFEST}")
}

fn pair(active: bool) -> Pair {
    pair_with(active, Arc::new(PersistedConfigMark::default()))
}

fn pair_with(active: bool, persisted: Arc<PersistedConfigMark>) -> Pair {
    let (host, publisher, ev_rx) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events(8, REPLICATION_HANDLED_EVENTS)
    });
    let peer = Arc::new(Peer::default());
    let role = Arc::new(Role {
        active: AtomicBool::new(active),
    });
    let counters = Arc::new(ReplicationCounters::default());
    let store = Arc::new(WakeOnPersistSliceStore::new(
        Arc::new(InMemorySliceStore::new()),
        vec![TRANSFERS],
        Arc::clone(&persisted),
        publisher.clone(),
        BusId(1),
    ));
    let _worker = spawn_replication_worker(
        ev_rx,
        ReplicationDeps {
            publisher: publisher.clone(),
            bus_id: BusId(1),
            fetch: Arc::clone(&peer) as Arc<dyn HttpFetch>,
            sink: Arc::new(NoSlices),
            redundancy: Arc::clone(&role) as Arc<dyn RedundancySettingsReadPort>,
            dali: Arc::clone(&role) as Arc<dyn DaliSettingsReadPort>,
            counters: Arc::clone(&counters),
            persisted: Arc::clone(&persisted),
        },
    );
    Pair {
        store,
        peer,
        persisted,
        counters,
        publisher,
        _host: host,
    }
}

fn publish(pair: &Pair, payload: impl Into<BusEventPayload>) {
    let ev = event_envelope(SOURCE_ID_UNSPECIFIED, CORRELATION_NONE, 1, None, payload);
    assert_eq!(
        pair.publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
}

fn settings_changed() -> RedundancySettingsChangedEvent {
    RedundancySettingsChangedEvent {
        enabled: true,
        standby_role: false,
        probe_interval_ms: 250,
        takeover_after_missed: 2,
        boot_listen_ms: 0,
        peer_device_short_address: 0xFF,
        peer_url: fixed_text_64("http://peer"),
    }
}

fn first_pass_done(pair: &Pair) {
    assert!(
        try_wait_until(|| pair.counters.passes.load(Ordering::Relaxed) >= 1, BUDGET),
        "the worker never ran its first pass"
    );
}

#[test]
fn a_wake_from_the_peer_starts_a_pass_long_before_the_period() {
    let pair = pair(false);
    first_pass_done(&pair);
    assert!(try_wait_until(|| pair.peer.count(&manifest_line()) == 1, BUDGET));
    publish(&pair, PeerConfigChangedEvent {});
    assert!(
        try_wait_until(|| pair.peer.count(&manifest_line()) == 2, BUDGET),
        "the wake did not start a pass within {BUDGET:?}"
    );
    assert_eq!(pair.counters.wakes_received.load(Ordering::Relaxed), 1);
}

#[test]
fn a_flood_of_wakes_runs_at_most_two_passes() {
    let pair = pair(false);
    first_pass_done(&pair);
    let before = pair.counters.passes.load(Ordering::Relaxed);
    for _ in 0..WAKE_FLOOD {
        publish(&pair, PeerConfigChangedEvent {});
    }
    assert!(try_wait_until(
        || pair.counters.passes.load(Ordering::Relaxed) > before,
        BUDGET
    ));
    assert!(remains_false_for(
        || pair.counters.passes.load(Ordering::Relaxed) > before + 2,
        SETTLE
    ));
}

#[test]
fn a_burst_of_saves_wakes_the_peer_once_after_the_last_of_them() {
    let pair = pair(true);
    let mut last_save = Instant::now();
    for i in 0..BURST {
        write_slice(pair.store.as_ref(), TRANSFERS, &[u8::try_from(i).unwrap()]);
        last_save = Instant::now();
        assert!(
            remains_false_for(|| pair.peer.count(&wake_line()) > 0, BURST_GAP),
            "the peer was woken in the middle of the burst, after save {i}"
        );
    }
    assert!(try_wait_until(|| pair.peer.count(&wake_line()) == 1, BUDGET));
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 1, SETTLE));
    assert!(pair.peer.first(&wake_line()).expect("the wake") > last_save);
    assert_eq!(pair.counters.wakes_sent.load(Ordering::Relaxed), 1);
    assert!(!pair.persisted.is_raised());
}

#[test]
fn a_slice_that_never_transfers_wakes_nobody_and_one_that_does_still_wakes() {
    let pair = pair(true);
    write_slice(pair.store.as_ref(), STAYS, b"role");
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 0, SETTLE));
    assert!(!pair.persisted.is_raised());
    write_slice(pair.store.as_ref(), TRANSFERS, b"poll");
    assert!(try_wait_until(|| pair.peer.count(&wake_line()) == 1, BUDGET));
}

#[test]
fn a_save_made_while_the_wake_is_on_the_wire_wakes_the_peer_once_more() {
    let pair = pair(true);
    let store = Arc::clone(&pair.store);
    let saved_during = Arc::new(AtomicBool::new(false));
    let once = Arc::clone(&saved_during);
    let hook: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            write_slice(store.as_ref(), TRANSFERS, b"late");
        }
    });
    assert!(pair.peer.during_post.set(hook).is_ok());
    write_slice(pair.store.as_ref(), TRANSFERS, b"first");
    assert!(try_wait_until(|| pair.peer.count(&wake_line()) == 2, BUDGET));
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 2, SETTLE));
    assert!(saved_during.load(Ordering::SeqCst));
}

#[test]
fn a_passive_controller_drops_the_mark_without_waking_its_peer() {
    let pair = pair(false);
    first_pass_done(&pair);
    write_slice(pair.store.as_ref(), TRANSFERS, b"pulled");
    assert!(try_wait_until(|| !pair.persisted.is_raised(), BUDGET));
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 0, SETTLE));
    assert_eq!(pair.counters.wakes_sent.load(Ordering::Relaxed), 0);
}

#[test]
fn a_mark_whose_event_was_lost_is_sent_on_the_next_turn() {
    let raised = Arc::new(PersistedConfigMark::default());
    assert!(raised.note(monotonic_ms()));
    let pair = pair_with(true, raised);
    assert!(
        try_wait_until(|| pair.peer.count(&wake_line()) == 1, BUDGET),
        "a mark raised before the worker started never rode its first turn"
    );
    assert!(pair.persisted.note(monotonic_ms()));
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 1, SETTLE));
    publish(&pair, settings_changed());
    assert!(
        try_wait_until(|| pair.peer.count(&wake_line()) == 2, BUDGET),
        "a raised mark waited for its own event instead of riding the next turn"
    );
}

#[test]
fn a_store_write_that_fails_raises_no_mark() {
    let pair = pair(true);
    let mut session = pair.store.begin_write(TRANSFERS).expect("begin");
    session.append(b"half").expect("append");
    session.abort();
    assert!(!pair.persisted.is_raised());
    assert!(remains_false_for(|| pair.peer.count(&wake_line()) > 0, SETTLE));
}
