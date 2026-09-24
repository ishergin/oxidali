use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusConfig, BusHost, PublishResult};
use dali2rust_platform::dali::SnifferTap;
use dali2rust_test_support::wait_until;
use dali2rust_ws_runtime::{WsCounters, WsHub, WsHubConfig, WsSink, WsSinkError};

const TINY_INBOX: usize = 4;

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

fn wait_for(frames: &Arc<Mutex<Vec<String>>>, needle: &str) -> String {
    let found = |f: &String| f.contains(needle);
    wait_until(
        || frames.lock().expect("frames").iter().any(found),
        Duration::from_secs(2),
    );
    frames
        .lock()
        .expect("frames")
        .iter()
        .find(|f| found(f))
        .cloned()
        .expect("wait_until returned, so it is there")
}

fn overflow_the_inbox() -> u32 {
    let (host, publisher, (rx, index)) = BusHost::spawn(BusConfig::default(), |reg| {
        reg.subscribe_events_indexed(TINY_INBOX, dali2rust_api::ws::WS_PROJECTED_EVENTS)
    });
    let probe = publisher.event_inbox_probe(index);
    wait_until(
        || {
            assert_eq!(
                publisher.try_publish(
                    BusChannel::Events,
                    dali2rust_test_support::projected_event_frame(1)
                ),
                PublishResult::Queued,
                "ingress must accept; the drop under test is the per-subscriber one"
            );
            probe.dropped_total() > 0
        },
        Duration::from_secs(5),
    );
    let dropped = probe.dropped_total();
    drop(rx);
    drop(host);
    dropped
}

#[test]
fn the_bus_counts_frames_it_drops_for_a_subscriber_that_never_reads() {
    let dropped = overflow_the_inbox();
    assert!(
        dropped > 0,
        "a subscriber that never drains must overflow, or the probe has nothing to report"
    );
}

#[test]
fn real_inbox_loss_reaches_a_connected_client_as_a_wildcard_drop_notice() {
    let dropped = overflow_the_inbox();
    assert!(dropped > 0, "precondition: the bus actually dropped frames");

    let (tap, _tap_rx) = SnifferTap::new(8);
    let hub = WsHub::new(WsHubConfig::default(), Arc::new(WsCounters::default()), tap, dali2rust_bsp::log_ring::global());
    let frames = Arc::new(Mutex::new(Vec::new()));
    hub.register(Box::new(RecordingSink {
        frames: Arc::clone(&frames),
    }))
    .expect("register");
    let _greeting = wait_for(&frames, "\"op\":\"hello\"");

    hub.note_inbox_overflow(dropped);

    let notice = wait_for(&frames, "DropNotice");
    assert!(
        notice.contains("\"channel\":\"*\""),
        "inbox loss is not per channel — every screen may be stale: {notice}"
    );
    assert!(
        notice.contains(&format!("\"dropped_count\":{dropped}")),
        "the count must be the bus's, not a placeholder: {notice}"
    );
    assert_eq!(
        WsCounters::load(&hub.counters().inbox_overflow_total),
        dropped,
        "the diagnostics counter reported 0 for as long as nothing called this"
    );
}
