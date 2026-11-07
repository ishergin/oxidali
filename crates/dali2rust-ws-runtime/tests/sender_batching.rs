use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dali2rust_api::ws::Channel;
use dali2rust_platform::dali::SnifferTap;
use dali2rust_test_support::wait_until;
use dali2rust_ws_runtime::{WsCounters, WsHub, WsHubConfig, WsSink, WsSinkError, SENDER_BATCH_FRAMES};

const BACKLOG_EVENTS: usize = 8;

const SETTLE: Duration = Duration::from_secs(2);

struct HeldSink {
    batches: Arc<Mutex<Vec<usize>>>,
    hold: Arc<(Mutex<bool>, Condvar)>,
    entered: Arc<AtomicUsize>,
}

impl WsSink for HeldSink {
    fn send_batch(&self, frames: &[Arc<str>]) -> Result<(), WsSinkError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (lock, released) = &*self.hold;
        let mut held = lock.lock().expect("hold");
        while *held {
            held = released.wait(held).expect("hold");
        }
        drop(held);
        self.batches.lock().expect("batches").push(frames.len());
        Ok(())
    }

    fn close(&self) {}
}

fn batches(seen: &Arc<Mutex<Vec<usize>>>) -> Vec<usize> {
    seen.lock().expect("batches").clone()
}

#[test]
fn a_clients_backlog_reaches_its_sink_as_one_batch() {
    let (tap, _tap_rx) = SnifferTap::new(8);
    let hub = WsHub::new(
        WsHubConfig::default(),
        Arc::new(WsCounters::default()),
        tap,
        dali2rust_bsp::log_ring::global(),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hold = Arc::new((Mutex::new(true), Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let id = hub
        .register(Box::new(HeldSink {
            batches: Arc::clone(&seen),
            hold: Arc::clone(&hold),
            entered: Arc::clone(&entered),
        }))
        .expect("register");
    wait_until(|| entered.load(Ordering::SeqCst) == 1, SETTLE);
    hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups"]}"#);
    for _ in 0..BACKLOG_EVENTS {
        hub.broadcast(Channel::Groups, "EVENT");
    }
    assert_eq!(
        WsCounters::load(&hub.counters().events_dropped_total),
        0,
        "precondition: the backlog must fit the client queue"
    );

    *hold.0.lock().expect("hold") = false;
    hold.1.notify_all();
    let backlog = BACKLOG_EVENTS + 1;
    let with_hello = backlog + 1;
    wait_until(
        || batches(&seen).iter().sum::<usize>() == with_hello,
        SETTLE,
    );
    let seen = batches(&seen);
    assert_eq!(
        seen.first().copied(),
        Some(1),
        "the hello is sent before the backlog exists: {seen:?}"
    );
    assert_eq!(
        seen.get(1).copied(),
        Some(backlog),
        "the whole backlog owes the httpd task one work item, not one per frame: {seen:?}"
    );
}

#[test]
fn one_work_item_never_carries_more_than_the_batch_bound() {
    const DEEP: usize = SENDER_BATCH_FRAMES * 3;
    let (tap, _tap_rx) = SnifferTap::new(8);
    let hub = WsHub::new(
        WsHubConfig {
            queue_depth: DEEP + 1,
            ..WsHubConfig::default()
        },
        Arc::new(WsCounters::default()),
        tap,
        dali2rust_bsp::log_ring::global(),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hold = Arc::new((Mutex::new(true), Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let id = hub
        .register(Box::new(HeldSink {
            batches: Arc::clone(&seen),
            hold: Arc::clone(&hold),
            entered: Arc::clone(&entered),
        }))
        .expect("register");
    wait_until(|| entered.load(Ordering::SeqCst) == 1, SETTLE);
    hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups"]}"#);
    for _ in 0..DEEP {
        hub.broadcast(Channel::Groups, "EVENT");
    }
    assert_eq!(
        WsCounters::load(&hub.counters().events_dropped_total),
        0,
        "precondition: a queue this deep must hold the hello and the whole burst"
    );

    *hold.0.lock().expect("hold") = false;
    hold.1.notify_all();
    let queued = DEEP + 1;
    let with_hello = queued + 1;
    wait_until(
        || batches(&seen).iter().sum::<usize>() == with_hello,
        SETTLE,
    );
    let seen = batches(&seen);
    let biggest = seen.iter().copied().max().expect("at least one batch");
    assert!(
        biggest <= SENDER_BATCH_FRAMES,
        "one work item carried {biggest} frames, past the bound of {SENDER_BATCH_FRAMES}: {seen:?}"
    );
    assert!(
        seen.len() >= 3,
        "a burst three batches deep must leave a residue for later turns: {seen:?}"
    );
}
