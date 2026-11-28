use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::frame::{
    cluster_tag, decode, encode, DatagramBuffer, Frame, Header, Hello, NodeClaim, Pulse,
};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_gear_model::GearSpec;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, HubDatagram, MockNetworkLink};
use serde_json::{json, Value};

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const OK: u16 = 200;
const CREATED: u16 = 201;
const ACCEPTED: u16 = 202;
const GROUP: u8 = 5;
const LAMP: u8 = 1;
const SHORT: u8 = 0;
const LEVEL: u64 = 180;
const PAGE_GAP: Duration = Duration::from_millis(10);
const CLOCK_GRAIN: Duration = Duration::from_millis(1);
const COALESCE: Duration = Duration::from_millis(PULSE_PERIOD_MS as u64 / 8);
const BURST: Duration = Duration::from_millis(600);
const WATCH: Duration = Duration::from_secs(2);
const HELLO_EVERY: Duration = Duration::from_millis(100);
const NEIGHBOUR: Board = Board { mac: [0x02, 0, 0, 0x0e, 0x0f, 0x09], address: [10, 0, 2, 9], epoch: 49 };
const NEIGHBOUR_NODE: u8 = 1;

struct Node {
    router: Router,
    _runtime: Box<BusStackRuntime>,
}

struct Board {
    mac: [u8; 6],
    address: [u8; 4],
    epoch: u32,
}

const FIRST: Board = Board { mac: [0x02, 0, 0, 0x0e, 0x0f, 0x01], address: [10, 0, 2, 1], epoch: 41 };
const SECOND: Board = Board { mac: [0x02, 0, 0, 0x0e, 0x0f, 0x02], address: [10, 0, 2, 2], epoch: 42 };

fn wire_with_a_lamp_in_group_5(random_address: u32) -> SimDaliTransport {
    let mut sim = SimDaliTransport::new(vec![GearSpec::dt6(Some(SHORT), random_address)]);
    for gear in sim.fleet_mut().gears_mut() {
        gear.groups = 1 << GROUP;
    }
    sim
}

fn node_on(hub: &Arc<ClusterHub>, board: &Board) -> Node {
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(hub), board.address)),
        timing: PresenceTiming::every_ms(PULSE_PERIOD_MS),
        boot_epoch: fixed_boot_epoch(board.epoch),
    };
    let link = MockNetworkLink::new(board.mac, Some(board.address), "cluster-node");
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        Arc::new(Mutex::new(wire_with_a_lamp_in_group_5(board.epoch))),
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        None,
        &[],
        HttpTestPorts { network_link: Some(Arc::new(link)), cluster: Some(cluster), ..HttpTestPorts::default() },
    );
    Node { router, _runtime: runtime }
}

impl Node {
    fn send(&self, method: &str, path: &str, body: &str) -> Value {
        let response = self.router.dispatch(method, path, body.as_bytes());
        let status = response.status;
        let answer = serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null);
        assert!([OK, CREATED, ACCEPTED].contains(&status), "{method} {path} answered {status}: {answer}");
        answer
    }

    fn read(&self, path: &str) -> Value {
        let response = self.router.dispatch("GET", path, &[]);
        serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null)
    }

    fn wait(&self, path: &str, pointer: &str, wanted: &Value) {
        let holds = || self.read(path).pointer(pointer) == Some(wanted);
        assert!(try_wait_until(holds, SETTLE), "{path}{pointer} never read {wanted}: {}", self.read(path));
    }

    fn commission_lamp_1_in_group_5(&self) {
        self.send("POST", "/api/v1/adapters/0/discovery-runs", r#"{"mode":"scan_known_short_addresses"}"#);
        self.wait("/api/v1/adapters/0/physical-devices/0", "/short_address", &json!(SHORT));
        self.send("PUT", "/api/v1/adapters/0/virtual-lamps/1/binding", &json!({ "physical_short_address": SHORT }).to_string());
        let reads = r#"{"attribute_groups":["groups"],"memory_banks":"none"}"#;
        self.send("POST", "/api/v1/adapters/0/physical-devices/0/attribute-reads", reads);
        self.wait("/api/v1/adapters/0/physical-devices/0/attributes", "/attributes/groups/membership/value", &json!(1 << GROUP));
    }

    fn join(&self, node_number: u8) {
        self.send("PATCH", "/api/v1/cluster", &json!({ "cluster_name": "hall", "node_number": node_number, "enabled": true }).to_string());
    }

    fn running(&self) -> bool {
        self.read("/api/v1/cluster/status")["own"]["state"] == "running"
    }

    fn part(&self, reference: &Value, accept: bool) {
        let part = json!({ "record": reference, "send": true, "accept": accept }).to_string();
        self.send("PUT", "/api/v1/cluster/channel-parts/0/5", &part);
    }

    fn link(&self, accept: bool) {
        let record = "/api/v1/cluster/channels/2";
        assert!(try_wait_until(|| self.read(record)["source"] == "cached", SETTLE), "{record}: {}", self.read(record));
        let held = self.read(record);
        self.part(&json!({ "number": held["number"], "birth_mark": held["birth_mark"] }), accept);
    }
}

fn running_pair() -> (Arc<ClusterHub>, Node, Node) {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.commission_lamp_1_in_group_5();
    second.commission_lamp_1_in_group_5();
    first.join(0);
    second.join(1);
    assert!(try_wait_until(|| first.running() && second.running(), SETTLE), "both nodes run");
    (hub, first, second)
}

fn neighbour_datagram(seq: u32, frame: &Frame<'_>) -> Vec<u8> {
    let header = Header {
        cluster_tag: cluster_tag("hall"),
        sender_mac: NEIGHBOUR.mac,
        boot_epoch: NEIGHBOUR.epoch,
        node_no: NEIGHBOUR_NODE,
        seq,
    };
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(&header, frame, &mut buffer).expect("encode");
    buffer[..len].to_vec()
}

fn neighbour_greets_until_stopped(hub: &ClusterHub, stop: &Receiver<()>) {
    let pulse = Pulse { cluster_name: "hall", installation: "site-n", claim: NodeClaim::Holding, wire: NEIGHBOUR_NODE, wire_protection: false };
    for seq in (1_u32..).step_by(2) {
        hub.inject(NEIGHBOUR.address, &neighbour_datagram(seq, &Frame::Pulse(pulse)));
        hub.inject(NEIGHBOUR.address, &neighbour_datagram(seq + 1, &Frame::Hello(Hello { ha_face: false })));
        if stop.recv_timeout(HELLO_EVERY) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

fn is_announced_page(datagram: &HubDatagram) -> bool {
    let frame = decode(&datagram.bytes).map(|decoded| decoded.frame);
    matches!(frame, Ok(Frame::ChannelSceneInfo(_) | Frame::LampState(_) | Frame::LampInfo(_) | Frame::InputInfo(_)))
}

fn is_input_page(datagram: &HubDatagram) -> bool {
    decode(&datagram.bytes).is_ok_and(|decoded| matches!(decoded.frame, Frame::InputInfo(_)))
}

fn announced_by(hub: &ClusterHub, board: &Board, window: Duration) -> (Vec<Instant>, usize) {
    let deadline = Instant::now() + window;
    let (mut sent, mut inputs) = (Vec::new(), 0);
    while let Some(datagram) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
        if datagram.source == board.address && is_announced_page(&datagram) {
            sent.push(datagram.at);
            inputs += usize::from(is_input_page(&datagram));
        }
    }
    (sent, inputs)
}

fn sent_by(hub: &ClusterHub, board: &Board, window: Duration, wanted: fn(&HubDatagram) -> bool) -> Vec<Instant> {
    let deadline = Instant::now() + window;
    let mut sent = Vec::new();
    while let Some(datagram) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
        if datagram.source == board.address && wanted(&datagram) {
            sent.push(datagram.at);
        }
    }
    sent
}

fn is_state_page(datagram: &HubDatagram) -> bool {
    decode(&datagram.bytes).is_ok_and(|decoded| matches!(decoded.frame, Frame::LampState(_)))
}

fn gaps(sent: &[Instant]) -> Vec<Duration> {
    sent.windows(2).map(|pair| pair[1].saturating_duration_since(pair[0])).collect()
}

#[test]
fn a_lamp_lit_on_one_node_shows_on_the_other_with_its_level_and_source() {
    let (_hub, first, second) = running_pair();
    first.send("PUT", "/api/v1/adapters/0/virtual-lamps/1/target-state", &json!({ "power": "on", "level": LEVEL }).to_string());
    let path = format!("/api/v1/cluster/nodes/0/lamps/{LAMP}");
    second.wait(&path, "/state/level", &json!(LEVEL));
    let lamp = second.read(&path);
    assert_eq!(lamp["state"]["power"], "on", "{lamp}");
    assert_eq!(lamp["source"], "cached", "{lamp}");
    assert_eq!(lamp["cluster_address"], json!({ "node": 0, "adapter": 0, "lamp": LAMP }), "{lamp}");
    let own = first.read(&path);
    assert_eq!((own["source"].clone(), own["state"]["level"].clone()), (json!("own"), json!(LEVEL)), "{own}");
}

#[test]
fn a_node_s_announced_pages_go_out_a_page_gap_apart_across_info_lamps_and_inputs() {
    let hub = Arc::new(ClusterHub::new());
    let first = node_on(&hub, &FIRST);
    first.commission_lamp_1_in_group_5();
    first.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    first.join(0);
    assert!(try_wait_until(|| first.running(), SETTLE), "the node runs");
    let watch_until = Instant::now() + WATCH;
    let (stop, stopped) = mpsc::channel();
    let neighbour = std::thread::spawn({
        let hub = Arc::clone(&hub);
        move || neighbour_greets_until_stopped(&hub, &stopped)
    });
    let levels = [LEVEL, LEVEL / 2].into_iter().cycle();
    for level in levels.take_while(|_| Instant::now() < watch_until) {
        first.send("PUT", "/api/v1/adapters/0/virtual-lamps/1/target-state", &json!({ "power": "on", "level": level }).to_string());
    }
    drop(stop);
    neighbour.join().expect("the neighbour greets");
    let (sent, inputs) = announced_by(&hub, &FIRST, CLOCK_GRAIN);
    assert!(sent.len() >= 2, "both the info set and the lamp sets went out: {}", sent.len());
    assert!(inputs >= 1, "the input set went out among them");
    let gaps = gaps(&sent);
    assert!(gaps.iter().all(|gap| *gap + CLOCK_GRAIN >= PAGE_GAP), "pages closer than the page gap: {gaps:?}");
}

#[test]
fn a_burst_of_commands_goes_out_as_state_sets_a_coalescing_window_apart() {
    let (hub, first, _second) = running_pair();
    while hub.next_sent(CLOCK_GRAIN).is_some() {}
    let burst_until = Instant::now() + BURST;
    let mut commands = 0_u32;
    for level in (1..=u64::from(u8::MAX - 1)).cycle().take_while(|_| Instant::now() < burst_until) {
        first.send("PUT", "/api/v1/adapters/0/virtual-lamps/1/target-state", &json!({ "power": "on", "level": level }).to_string());
        commands += 1;
    }
    let sent = sent_by(&hub, &FIRST, COALESCE * 2, is_state_page);
    let gaps = gaps(&sent);
    assert!(u32::try_from(sent.len()).unwrap_or(u32::MAX) < commands, "{commands} commands, {} sets", sent.len());
    let held_back = PAGE_GAP + CLOCK_GRAIN;
    assert!(gaps.iter().all(|gap| *gap + held_back >= COALESCE), "sets closer than the coalescing window: {gaps:?}");
}

#[test]
fn a_channel_with_parts_on_both_nodes_is_on_when_one_node_s_member_is_lit() {
    let (_hub, first, second) = running_pair();
    let channel = first.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    first.part(&json!({ "number": channel["number"], "birth_mark": channel["birth_mark"] }), false);
    second.link(true);
    second.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    first.wait("/api/v1/cluster/channels/2", "/state/on", &json!(true));
    let state = first.read("/api/v1/cluster/channels/2")["state"].clone();
    assert_eq!((state["lit"].clone(), state["members"].clone()), (json!(1), json!(2)), "{state}");
    let parts = first.read("/api/v1/cluster/channels/2");
    let lit_parts = parts["parts"].as_array().cloned().unwrap_or_default();
    let on_parts: Vec<&Value> = lit_parts.iter().filter(|part| part["state"]["on"] == json!(true)).collect();
    assert_eq!(on_parts.len(), 1, "only the second node's part is on: {parts}");
    assert_eq!(on_parts.first().map(|part| part["node"].clone()), Some(json!(1)), "{parts}");
}
