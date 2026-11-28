use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::BusStackRuntime;
use dali2rust_api::http::router::Router;
use dali2rust_cluster_runtime::frame::{decode, Frame};
use dali2rust_platform::dali::ObservedRawFrameKind;
use dali2rust_test_support::{try_wait_until, ClusterHub, HubDatagram};
use serde_json::{json, Value};

mod cluster_node;

use cluster_node::{stack_on, Board};

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_secs(1);
const OK: u16 = 200;
const PRESS: [u8; 3] = [0x06, 0x80, 0x01];
const RELEASE: [u8; 3] = [0x06, 0x80, 0x00];
const UNADDRESSED_PRESS: [u8; 3] = [0x82, 0x80, 0x01];
const PRESSED: u16 = 1;
const RELEASED: u16 = 0;
const PANEL: u8 = 3;

struct Node {
    router: Router,
    wire: Arc<Mutex<MockDaliTransport>>,
    _runtime: Box<BusStackRuntime>,
}


const FIRST: Board = Board { mac: [0x02, 0, 0, 0x0e, 0x0f, 0x01], address: [10, 0, 2, 1], epoch: 41 };
const SECOND: Board = Board { mac: [0x02, 0, 0, 0x0e, 0x0f, 0x02], address: [10, 0, 2, 2], epoch: 42 };

fn node_on(hub: &Arc<ClusterHub>, board: &Board) -> Node {
    let wire = Arc::new(Mutex::new(MockDaliTransport::new()));
    let (router, runtime) = stack_on(hub, board, PULSE_PERIOD_MS, Arc::clone(&wire));
    Node { router, wire, _runtime: runtime }
}

impl Node {
    fn read(&self, path: &str) -> Value {
        let response = self.router.dispatch("GET", path, &[]);
        serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null)
    }

    fn patch(&self, path: &str, body: &Value) {
        let response = self.router.dispatch("PATCH", path, body.to_string().as_bytes());
        assert_eq!(response.status, OK, "PATCH {path}");
    }

    fn join(&self, node_number: u8) {
        self.patch("/api/v1/cluster", &json!({ "cluster_name": "hall", "node_number": node_number, "enabled": true }));
    }

    fn running(&self) -> bool {
        self.read("/api/v1/cluster/status")["own"]["state"] == "running"
    }

    fn observe(&self, bytes: [u8; 3]) {
        let injected = self.wire.lock().expect("the mock transport").inject_observed_frame(bytes, ObservedRawFrameKind::Forward24);
        assert!(injected, "the sniffer seam is attached");
    }

    fn stat(&self, counter: &str) -> u64 {
        self.read("/api/v1/stats")["cluster"][counter].as_u64().unwrap_or(u64::MAX)
    }
}

fn running_pair(hub: &Arc<ClusterHub>) -> (Node, Node) {
    let (first, second) = (node_on(hub, &FIRST), node_on(hub, &SECOND));
    first.join(0);
    second.join(1);
    assert!(try_wait_until(|| first.running() && second.running(), SETTLE), "both nodes run");
    (first, second)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SentInput {
    seq: u32,
    device: u8,
    event_info: u16,
}

fn input_of(sent: &HubDatagram, board: &Board) -> Option<SentInput> {
    let decoded = decode(&sent.bytes).ok().filter(|_| sent.source == board.address)?;
    match decoded.frame {
        Frame::InputEvent(event) => {
            Some(SentInput { seq: decoded.header.seq, device: event.address.device, event_info: event.event_info })
        }
        _ => None,
    }
}

fn inputs_from(hub: &ClusterHub, board: &Board, window: Duration) -> Vec<SentInput> {
    let deadline = Instant::now() + window;
    let mut inputs = Vec::new();
    while let Some(sent) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
        inputs.extend(input_of(&sent, board));
    }
    inputs
}

fn drain(hub: &ClusterHub) {
    while hub.next_sent(Duration::ZERO).is_some() {}
}

#[test]
fn a_press_and_its_release_leave_twice_each_in_order_and_arrive_once_each() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = running_pair(&hub);
    drain(&hub);
    first.observe(PRESS);
    first.observe(RELEASE);
    let arrived = || second.stat("input_events_received_total") == 2;
    assert!(try_wait_until(arrived, SETTLE), "the neighbour's counters: {}", second.read("/api/v1/stats")["cluster"]);
    let sent = inputs_from(&hub, &FIRST, QUIET);
    let infos: Vec<u16> = sent.iter().map(|input| input.event_info).collect();
    assert_eq!(infos, [PRESSED, PRESSED, RELEASED, RELEASED], "never folded, never reordered: {sent:?}");
    assert!(sent.iter().all(|input| input.device == PANEL));
    assert_eq!((sent[0].seq, sent[2].seq), (sent[1].seq, sent[3].seq), "a copy keeps its first's sequence");
    assert!(sent[0].seq < sent[2].seq);
    assert_eq!(first.stat("input_events_sent_total"), 2);
    assert_eq!(second.stat("input_events_dropped_total"), 0, "a copy is not a dropped event");
}

#[test]
fn a_press_whose_first_copy_is_lost_arrives_from_its_second() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = running_pair(&hub);
    let lost_one = Arc::new(AtomicBool::new(false));
    let losing = Arc::clone(&lost_one);
    hub.lose_sends(move |sent| {
        let first_copy = input_of(sent, &FIRST).is_some() && !losing.load(Ordering::Relaxed);
        losing.fetch_or(first_copy, Ordering::Relaxed);
        first_copy
    });
    first.observe(PRESS);
    let arrived = || second.stat("input_events_received_total") == 1;
    assert!(try_wait_until(arrived, SETTLE), "the neighbour's counters: {}", second.read("/api/v1/stats")["cluster"]);
    assert!(lost_one.load(Ordering::Relaxed), "the first copy was lost");
}

#[test]
fn an_event_without_a_short_address_stays_home_and_is_counted() {
    let hub = Arc::new(ClusterHub::new());
    let (first, _second) = running_pair(&hub);
    drain(&hub);
    first.observe(UNADDRESSED_PRESS);
    assert!(try_wait_until(|| first.stat("input_events_unaddressed_total") == 1, SETTLE), "the event is counted");
    assert_eq!(inputs_from(&hub, &FIRST, QUIET), Vec::new());
    assert_eq!(first.stat("input_events_sent_total"), 0);
}

#[test]
fn a_node_that_shares_no_input_events_sends_neither_events_nor_its_inputs() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = running_pair(&hub);
    first.patch("/api/v1/cluster", &json!({ "share_input_events": false }));
    drain(&hub);
    first.observe(PRESS);
    let window = Instant::now() + QUIET;
    let mut shared = 0;
    while let Some(sent) = hub.next_sent(window.saturating_duration_since(Instant::now())) {
        let frame = decode(&sent.bytes).ok().filter(|_| sent.source == FIRST.address).map(|decoded| decoded.frame);
        shared += usize::from(matches!(frame, Some(Frame::InputEvent(_) | Frame::InputInfo(_))));
    }
    assert_eq!(shared, 0, "neither an input event nor an input info page left");
    assert_eq!(second.stat("input_events_received_total"), 0);
}

#[test]
fn a_node_s_inputs_reach_its_neighbour_as_a_whole_set_with_the_owner() {
    let hub = Arc::new(ClusterHub::new());
    let (_first, second) = running_pair(&hub);
    let cached = || second.read("/api/v1/cluster/nodes/0/inputs")["source"] == "cached";
    assert!(try_wait_until(cached, SETTLE), "{}", second.read("/api/v1/cluster/nodes/0/inputs"));
    let inputs = second.read("/api/v1/cluster/nodes/0/inputs");
    assert_eq!(inputs["device_count"], 0);
    assert_eq!(inputs["devices"], json!([]));
    assert!(inputs["owner"]["discovery_prefix"].is_string(), "{inputs}");
    assert_eq!(second.read("/api/v1/cluster/nodes/1/inputs")["source"], "own");
}
