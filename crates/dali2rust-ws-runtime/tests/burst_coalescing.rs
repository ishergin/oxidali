use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_api::http::diagnostics_state::{DiagnosticsDto, DiagnosticsHttpState};
use dali2rust_api::http::stats_state::{StatsHttpState, StatsReportDto};
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, PublishResult};
use dali2rust_platform::dali::SnifferTap;
use dali2rust_test_support::wait_until;
use dali2rust_ws_runtime::{
    spawn_ws_worker, ClientId, WsCounters, WsHub, WsHubConfig, WsSink, WsSinkError, WsWorkerPorts,
};

struct RecordingSink {
    frames: Arc<Mutex<Vec<String>>>,
}

impl WsSink for RecordingSink {
    fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
        self.frames.lock().expect("frames").push(text.to_string());
        Ok(())
    }
    fn close(&self) {}
}

struct NullStats;
impl StatsHttpState for NullStats {
    fn stats_dto(&self) -> StatsReportDto {
        StatsReportDto::default()
    }
}
struct NullDiagnostics;
impl DiagnosticsHttpState for NullDiagnostics {
    fn diagnostics_dto(&self) -> DiagnosticsDto {
        DiagnosticsDto::default()
    }
}

fn addressed_runtime_frame(
    virtual_lamp_id: Option<u8>,
    short_address: Option<u8>,
    level: u8,
) -> BusFrame {
    BusFrame::event(dali2rust_contracts::bus::event_envelope(
        0,
        u64::from(level),
        0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::RuntimeStateChangedEvent {
            adapter_id: 0,
            virtual_lamp_id,
            short_address,
            state_setpoint: dali2rust_contracts::msg::LightSetpoint::from_level(level, None),
            state_observation: dali2rust_contracts::msg::RuntimeObservation::default(),
            commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
            commit_dimensions: dali2rust_contracts::msg::LightSetpoint::from_level(level, None)
                .dimensions(),
        },
    ))
}

fn runtime_frame(level: u8) -> BusFrame {
    addressed_runtime_frame(Some(1), Some(1), level)
}

struct Harness {
    _host: BusHost,
    _worker: std::thread::JoinHandle<()>,
    publisher: dali2rust_bus::BusPublisher,
    hub: Arc<WsHub>,
    counters: Arc<WsCounters>,
    frames: Arc<Mutex<Vec<String>>>,
    client_id: ClientId,
}

fn spawn_harness() -> Harness {
    let (_host, publisher, (ev_rx, index)) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events_indexed(128, dali2rust_api::ws::WS_PROJECTED_EVENTS)
    });
    let (tap, tap_rx) = SnifferTap::new(8);
    let counters = Arc::new(WsCounters::default());
    let hub = WsHub::new(WsHubConfig::default(), Arc::clone(&counters), tap.clone(), dali2rust_bsp::log_ring::global());
    let _worker = spawn_ws_worker(
        ev_rx,
        publisher.event_inbox_probe(index),
        tap_rx,
        tap,
        Arc::clone(&hub),
        WsWorkerPorts {
            stats: Arc::new(NullStats),
            diagnostics: Arc::new(NullDiagnostics),
        },
        Arc::new(|| 0),
    );

    let frames = Arc::new(Mutex::new(Vec::new()));
    let client_id = hub
        .register(Box::new(RecordingSink {
            frames: Arc::clone(&frames),
        }))
        .expect("register");
    Harness {
        _host,
        _worker,
        publisher,
        hub,
        counters,
        frames,
        client_id,
    }
}

#[test]
fn a_same_lamp_burst_reaches_the_client_as_its_final_state() {
    let h = spawn_harness();
    let (publisher, counters, frames) = (&h.publisher, &h.counters, &h.frames);
    h.hub
        .handle_client_text(h.client_id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
    wait_until(
        || {
            frames
                .lock()
                .expect("frames")
                .iter()
                .any(|f| f.contains("\"op\":\"subscribed\""))
        },
        Duration::from_secs(2),
    );

    const BURST: u8 = 5;
    for level in 1..=BURST {
        assert_eq!(
            publisher.try_publish(BusChannel::Events, runtime_frame(level)),
            PublishResult::Queued
        );
    }

    let vl_frames = |frames: &Arc<Mutex<Vec<String>>>| -> Vec<String> {
        frames
            .lock()
            .expect("frames")
            .iter()
            .filter(|f| f.contains("RuntimeStateChangedEvent"))
            .cloned()
            .collect()
    };
    wait_until(
        || {
            let delivered = vl_frames(frames).len() as u32;
            delivered + WsCounters::load(&counters.events_coalesced_total) >= u32::from(BURST)
        },
        Duration::from_secs(3),
    );

    let delivered = vl_frames(frames);
    let coalesced = WsCounters::load(&counters.events_coalesced_total);
    assert_eq!(
        delivered.len() as u32 + coalesced,
        u32::from(BURST),
        "every envelope is either delivered or superseded — never lost"
    );
    assert!(
        delivered.last().expect("at least the survivor").contains("\"level\":5"),
        "the last frame must carry the burst's final state: {delivered:?}"
    );
    assert_eq!(WsCounters::load(&counters.events_dropped_total), 0);
    assert_eq!(WsCounters::load(&counters.inbox_overflow_total), 0);
}

#[test]
fn two_keys_landing_on_one_device_row_end_on_its_final_state() {
    let h = spawn_harness();
    let (publisher, counters, frames) = (&h.publisher, &h.counters, &h.frames);
    h.hub.handle_client_text(
        h.client_id,
        r#"{"op":"subscribe","channels":["physical_devices"]}"#,
    );
    wait_until(
        || {
            frames
                .lock()
                .expect("frames")
                .iter()
                .any(|f| f.contains("\"op\":\"subscribed\""))
        },
        Duration::from_secs(2),
    );

    for frame in [
        addressed_runtime_frame(Some(1), Some(5), 10),
        addressed_runtime_frame(None, Some(5), 20),
        addressed_runtime_frame(Some(1), Some(5), 30),
    ] {
        assert_eq!(
            publisher.try_publish(BusChannel::Events, frame),
            PublishResult::Queued
        );
    }

    let pd_frames = |frames: &Arc<Mutex<Vec<String>>>| -> Vec<String> {
        frames
            .lock()
            .expect("frames")
            .iter()
            .filter(|f| f.contains("RuntimeStateChangedEvent"))
            .cloned()
            .collect()
    };
    wait_until(
        || {
            let delivered = pd_frames(frames).len() as u32;
            delivered + WsCounters::load(&counters.events_coalesced_total) >= 3
        },
        Duration::from_secs(3),
    );

    let delivered = pd_frames(frames);
    let last = delivered.last().expect("at least the survivor");
    assert!(
        last.contains("\"level\":30"),
        "the row's last write on the wire must be the last frame delivered: {delivered:?}"
    );
}

#[test]
fn a_dual_channel_subscriber_still_gets_both_views_of_the_survivor() {
    let h = spawn_harness();
    let frames = &h.frames;
    h.hub.handle_client_text(
        h.client_id,
        r#"{"op":"subscribe","channels":["virtual_lamps","physical_devices"]}"#,
    );

    assert_eq!(
        h.publisher.try_publish(BusChannel::Events, runtime_frame(9)),
        PublishResult::Queued
    );
    let has = |needle: &str| {
        let frames = Arc::clone(frames);
        let needle = needle.to_string();
        move || {
            frames
                .lock()
                .expect("frames")
                .iter()
                .any(|f| f.contains(&needle) && f.contains("RuntimeStateChangedEvent"))
        }
    };
    wait_until(has("\"channel\":\"virtual_lamps\""), Duration::from_secs(3));
    wait_until(has("\"channel\":\"physical_devices\""), Duration::from_secs(3));
}
