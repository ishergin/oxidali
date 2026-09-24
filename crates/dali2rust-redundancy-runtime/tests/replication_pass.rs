use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusId};
use dali2rust_domain::registry::{
    DaliSettingsReadPort, DaliSettingsView, RedundancySettingsReadPort, RedundancySettingsView,
};
use dali2rust_platform::http_fetch::{FetchError, HttpFetch};
use dali2rust_redundancy_runtime::{
    run_pass, PassOutcome, ReplicationCounters, ReplicationDeps, ReplicationSink, SliceDigest,
};

struct FakeSettings {
    enabled: AtomicBool,
    active: AtomicBool,
    peer_url: String,
}

impl RedundancySettingsReadPort for FakeSettings {
    fn redundancy_settings_view(&self) -> RedundancySettingsView {
        RedundancySettingsView {
            enabled: self.enabled.load(Ordering::Relaxed),
            standby_role: true,
            probe_interval_ms: 250,
            takeover_after_missed: 2,
            boot_listen_ms: 0,
            peer_device_short_address: None,
            peer_url: self.peer_url.clone(),
        }
    }
}

impl DaliSettingsReadPort for FakeSettings {
    fn dali_settings_view(&self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: self.active.load(Ordering::Relaxed),
            device_short_address: None,
        }
    }
}

struct FakePeer {
    manifest: String,
    bodies: Vec<(String, Vec<u8>)>,
    asked: Mutex<Vec<String>>,
    unreachable: AtomicBool,
}

impl HttpFetch for FakePeer {
    fn get(&self, _base: &str, path: &str) -> Result<Vec<u8>, FetchError> {
        self.asked.lock().unwrap().push(path.to_string());
        if self.unreachable.load(Ordering::Relaxed) {
            return Err(FetchError::Transport("peer down".to_string()));
        }
        if path == "/api/v1/config/slices" {
            return Ok(self.manifest.clone().into_bytes());
        }
        self.bodies
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.clone())
            .ok_or(FetchError::Status(404))
    }
}

struct FakeSink {
    local: Vec<SliceDigest>,
    refuse: Vec<&'static str>,
    written: Mutex<Vec<(String, usize)>>,
}

impl ReplicationSink for FakeSink {
    fn local_digests(&self) -> Vec<SliceDigest> {
        self.local.clone()
    }

    fn write_slice(&self, name: &str, bytes: &[u8]) -> bool {
        if self.refuse.contains(&name) {
            return false;
        }
        self.written
            .lock()
            .unwrap()
            .push((name.to_string(), bytes.len()));
        true
    }
}

struct Rig {
    deps: ReplicationDeps,
    peer: Arc<FakePeer>,
    sink: Arc<FakeSink>,
    settings: Arc<FakeSettings>,
    cmd_rx: dali2rust_bus::BusSubscriberRx,
    _host: BusHost,
}

const WATCHED: &[&str] = &["RegistrySliceReloadCommand"];

fn rig(local: Vec<SliceDigest>, refuse: Vec<&'static str>) -> Rig {
    let (host, publisher, cmd_rx) =
        BusHost::spawn(BusConfig::default(), |reg| reg.subscribe_commands(64, WATCHED));
    let peer = Arc::new(FakePeer {
        manifest: r#"[
            {"name":"physical_devices_a0","bytes":9,"crc32":1},
            {"name":"virtual_lamps_a0","bytes":5,"crc32":2},
            {"name":"poller_settings","bytes":4,"crc32":3}
        ]"#
        .to_string(),
        bodies: vec![
            (
                "/api/v1/config/slices/physical_devices_a0".to_string(),
                b"physdevs!".to_vec(),
            ),
            (
                "/api/v1/config/slices/virtual_lamps_a0".to_string(),
                b"vlamp".to_vec(),
            ),
            (
                "/api/v1/config/slices/poller_settings".to_string(),
                b"poll".to_vec(),
            ),
        ],
        asked: Mutex::new(Vec::new()),
        unreachable: AtomicBool::new(false),
    });
    let sink = Arc::new(FakeSink {
        local,
        refuse,
        written: Mutex::new(Vec::new()),
    });
    let settings = Arc::new(FakeSettings {
        enabled: AtomicBool::new(true),
        active: AtomicBool::new(false),
        peer_url: "http://peer".to_string(),
    });
    Rig {
        deps: ReplicationDeps {
            publisher,
            bus_id: BusId(1),
            fetch: Arc::clone(&peer) as Arc<dyn HttpFetch>,
            sink: Arc::clone(&sink) as Arc<dyn ReplicationSink>,
            redundancy: Arc::clone(&settings) as Arc<dyn RedundancySettingsReadPort>,
            dali: Arc::clone(&settings) as Arc<dyn DaliSettingsReadPort>,
            counters: Arc::new(ReplicationCounters::default()),
        },
        peer,
        sink,
        settings,
        cmd_rx,
        _host: host,
    }
}

fn digest(name: &str, bytes: usize, crc: u32) -> SliceDigest {
    SliceDigest {
        name: name.to_string(),
        bytes: Some(bytes),
        crc32: crc,
    }
}

fn drain_reloads(rig: &Rig, want: usize) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let floor = std::time::Instant::now() + std::time::Duration::from_millis(200);
    let mut n = 0;
    while (n < want || std::time::Instant::now() < floor)
        && std::time::Instant::now() < deadline
    {
        while let Ok(BusFrame::Command(ce)) = rig.cmd_rx.try_recv() {
            if matches!(
                ce.payload,
                dali2rust_contracts::msg::BusCommandPayload::RegistrySliceReloadCommand(_)
            ) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn an_empty_standby_pulls_every_slice_in_the_peers_order() {
    let rig = rig(Vec::new(), Vec::new());
    let outcome = run_pass(&rig.deps);
    assert!(matches!(outcome, PassOutcome::Pulled(ref v) if v.len() == 3));
    let written: Vec<String> = rig
        .sink
        .written
        .lock()
        .unwrap()
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(
        written,
        vec!["physical_devices_a0", "virtual_lamps_a0", "poller_settings"]
    );
    assert_eq!(rig.deps.counters.slices_pulled.load(Ordering::Relaxed), 3);
}

#[test]
fn three_pulled_slices_ask_for_exactly_one_reload() {
    let rig = rig(Vec::new(), Vec::new());
    run_pass(&rig.deps);
    assert_eq!(drain_reloads(&rig, 1), 1);
}

#[test]
fn a_matching_standby_fetches_no_bodies_and_asks_for_no_reload() {
    let rig = rig(
        vec![
            digest("physical_devices_a0", 9, 1),
            digest("virtual_lamps_a0", 5, 2),
            digest("poller_settings", 4, 3),
        ],
        Vec::new(),
    );
    assert!(matches!(run_pass(&rig.deps), PassOutcome::UpToDate));
    assert_eq!(rig.peer.asked.lock().unwrap().len(), 1);
    assert_eq!(drain_reloads(&rig, 0), 0);
}

#[test]
fn a_refused_slice_does_not_hold_back_the_others() {
    let rig = rig(Vec::new(), vec!["virtual_lamps_a0"]);
    let outcome = run_pass(&rig.deps);
    assert!(matches!(outcome, PassOutcome::Pulled(ref v) if v.len() == 2));
    assert_eq!(rig.deps.counters.slices_rejected.load(Ordering::Relaxed), 1);
    assert_eq!(rig.deps.counters.slices_pulled.load(Ordering::Relaxed), 2);
}

#[test]
fn an_unreachable_peer_leaves_the_local_copy_alone() {
    let rig = rig(vec![digest("poller_settings", 4, 99)], Vec::new());
    rig.peer.unreachable.store(true, Ordering::Relaxed);
    assert!(matches!(run_pass(&rig.deps), PassOutcome::PeerUnreachable(_)));
    assert!(rig.sink.written.lock().unwrap().is_empty());
    assert_eq!(
        rig.deps.counters.peer_unreachable.load(Ordering::Relaxed),
        1
    );
}

#[test]
fn an_active_controller_never_pulls() {
    let rig = rig(Vec::new(), Vec::new());
    rig.settings.active.store(true, Ordering::Relaxed);
    assert!(matches!(run_pass(&rig.deps), PassOutcome::Idle));
    assert!(rig.peer.asked.lock().unwrap().is_empty());
    assert_eq!(rig.deps.counters.passes.load(Ordering::Relaxed), 0);
}
