use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig,
    EpochSource, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::frame::{cluster_tag, decode, Frame, InfoElement};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, HubDatagram, MockNetworkLink};
use serde_json::Value;

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const CLUSTER_NAME: &str = "hall";
const SETTINGS: &str = "/api/v1/cluster";
const STATUS: &str = "/api/v1/cluster/status";
const OK: u16 = 200;
const CREATED: u16 = 201;
const NO_CONTENT: u16 = 204;
const CONFLICT: u16 = 409;
const MOVED_PORT: u16 = 47_811;
const QUICK_PULSE_MS: u32 = 125;
const ROUND: Duration = Duration::from_secs(30);

struct Node {
    router: Router,
    _runtime: Box<BusStackRuntime>,
}

struct Board {
    mac: [u8; 6],
    address: [u8; 4],
    epoch: u32,
}

const REPLACEMENT: Board = Board { mac: [0x02, 0, 0, 0x0a, 0x0b, 0x03], address: [10, 0, 0, 3], epoch: 33 };
const FIRST: Board = Board { mac: [0x02, 0, 0, 0x0a, 0x0b, 0x01], address: [10, 0, 0, 1], epoch: 11 };
const SECOND: Board = Board { mac: [0x02, 0, 0, 0x0a, 0x0b, 0x02], address: [10, 0, 0, 2], epoch: 22 };

fn node_on(hub: &Arc<ClusterHub>, board: &Board) -> Node {
    node_drawing(hub, board, fixed_boot_epoch(board.epoch))
}

fn node_drawing(hub: &Arc<ClusterHub>, board: &Board, boot_epoch: EpochSource) -> Node {
    node_timed(hub, board, boot_epoch, PresenceTiming::every_ms(PULSE_PERIOD_MS))
}

fn node_timed(hub: &Arc<ClusterHub>, board: &Board, boot_epoch: EpochSource, timing: PresenceTiming) -> Node {
    let binder = Arc::new(ClusterHubBinder::new(Arc::clone(hub), board.address));
    let cluster = ClusterComposition { binder, timing, boot_epoch };
    composed(MockNetworkLink::new(board.mac, Some(board.address), "cluster-node"), Some(cluster))
}

fn next_pulse(hub: &ClusterHub) -> HubDatagram {
    (0..)
        .map_while(|_| hub.next_sent(SETTLE))
        .find(|sent| matches!(decode(&sent.bytes), Ok(decoded) if matches!(decoded.frame, Frame::Pulse(_))))
        .expect("the node pulses")
}

fn composed(link: MockNetworkLink, cluster: Option<ClusterComposition>) -> Node {
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        Arc::new(Mutex::new(MockDaliTransport::new())),
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        None,
        &[],
        HttpTestPorts { network_link: Some(Arc::new(link)), cluster, ..HttpTestPorts::default() },
    );
    Node { router, _runtime: runtime }
}

impl Node {
    fn patch(&self, body: &str) {
        let status = self.router.dispatch("PATCH", SETTINGS, body.as_bytes()).status;
        assert_eq!(status, OK, "PATCH {body}");
    }

    fn join(&self, node_number: u8) {
        self.patch(&format!(r#"{{"cluster_name":"{CLUSTER_NAME}","node_number":{node_number},"enabled":true}}"#));
    }

    fn status(&self) -> Value {
        let response = self.router.dispatch("GET", STATUS, &[]);
        assert_eq!(response.status, OK);
        serde_json::from_slice(&response.into_body_bytes()).expect("the status is JSON")
    }

    fn heard(&self, node: u64) -> Option<bool> {
        let status = self.status();
        let nodes = status["nodes"].as_array()?;
        nodes.iter().find(|row| row["node"] == node).and_then(|row| row["heard"].as_bool())
    }

    fn block(&self) -> String {
        self.status()["own"]["block"].as_str().unwrap_or_default().to_owned()
    }

    fn settled(&self) {
        assert!(try_wait_until(|| self.block() == "settled", SETTLE), "block {}", self.block());
    }

    fn state(&self) -> String {
        self.status()["own"]["state"].as_str().unwrap_or_default().to_owned()
    }

    fn send(&self, method: &str, path: &str, body: &str) -> Value {
        let response = self.router.dispatch(method, path, body.as_bytes());
        assert!([OK, CREATED].contains(&response.status), "{method} {path} answered {}", response.status);
        serde_json::from_slice(&response.into_body_bytes()).expect("the answer is JSON")
    }

    fn read(&self, path: &str) -> Value {
        let response = self.router.dispatch("GET", path, &[]);
        serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null)
    }

    fn forget(&self, node: u8) -> u16 {
        self.router.dispatch("DELETE", &format!("/api/v1/cluster/nodes/{node}"), &[]).status
    }
}

#[test]
fn two_nodes_on_one_hub_run_and_each_hears_the_other() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.join(0);
    second.join(1);
    let both_run = || first.state() == "running" && second.state() == "running";
    assert!(try_wait_until(both_run, SETTLE), "states {} and {}", first.state(), second.state());
    let each_hears = || first.heard(1) == Some(true) && second.heard(0) == Some(true);
    assert!(try_wait_until(each_hears, SETTLE), "first {:?}, second {:?}", first.status(), second.status());
    let row = &first.status()["nodes"][0];
    assert_eq!(row["board"], "dali-0a0b02");
    assert_eq!(row["address"], "10.0.0.2");
}

#[test]
fn a_pulse_on_the_hub_is_a_production_frame_naming_its_node_and_cluster() {
    let hub = Arc::new(ClusterHub::new());
    let first = node_on(&hub, &FIRST);
    first.join(3);
    let datagram = next_pulse(&hub);
    let decoded = decode(&datagram.bytes).expect("a production frame");
    assert_eq!(datagram.source, FIRST.address);
    assert_eq!(decoded.header.sender_mac, FIRST.mac);
    assert_eq!(decoded.header.node_no, 3);
    assert_eq!(decoded.header.boot_epoch, FIRST.epoch);
    assert_eq!(decoded.header.cluster_tag, cluster_tag(CLUSTER_NAME));
    assert_eq!(first.status()["own"]["boot_epoch"], FIRST.epoch);
}

#[test]
fn the_boot_epoch_is_drawn_once_with_the_first_frame_and_kept_across_a_rebind() {
    let hub = Arc::new(ClusterHub::new());
    let draws = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&draws);
    let drawing: EpochSource = Arc::new(move || FIRST.epoch + counted.fetch_add(1, Ordering::Relaxed));
    let node = node_drawing(&hub, &FIRST, drawing);
    assert!(node.status()["own"]["boot_epoch"].is_null(), "{:?}", node.status());
    node.join(0);
    let pulse = next_pulse(&hub);
    let decoded = decode(&pulse.bytes).expect("a production frame");
    assert_eq!(decoded.header.boot_epoch, FIRST.epoch);
    node.patch(r#"{"enabled":false}"#);
    assert!(try_wait_until(|| node.state() == "off", SETTLE), "state {}", node.state());
    node.patch(r#"{"enabled":true}"#);
    assert!(try_wait_until(|| node.state() == "running", SETTLE), "state {}", node.state());
    assert_eq!(node.status()["own"]["boot_epoch"], FIRST.epoch);
    assert_eq!(draws.load(Ordering::Relaxed), 1);
}

#[test]
fn forget_waits_until_the_node_is_silent_and_then_drops_it() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.join(0);
    second.join(1);
    assert!(try_wait_until(|| first.heard(1) == Some(true), SETTLE), "{:?}", first.status());
    assert_eq!(first.forget(1), CONFLICT, "a heard node is not forgotten");
    second.patch(r#"{"enabled":false}"#);
    assert!(try_wait_until(|| first.heard(1) == Some(false), SETTLE), "{:?}", first.status());
    assert_eq!(first.forget(1), NO_CONTENT);
    assert_eq!(first.heard(1), None, "{:?}", first.status());
}

#[test]
fn a_node_moved_to_another_port_hears_its_neighbour_on_the_new_link() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.join(0);
    second.join(1);
    assert!(try_wait_until(|| first.heard(1) == Some(true), SETTLE), "{:?}", first.status());
    second.patch(r#"{"enabled":false}"#);
    assert!(try_wait_until(|| first.heard(1) == Some(false), SETTLE), "{:?}", first.status());
    first.patch(&format!(r#"{{"port":{MOVED_PORT}}}"#));
    assert!(try_wait_until(|| first.heard(1).is_none(), SETTLE), "the move clears the table: {:?}", first.status());
    second.patch(r#"{"enabled":true}"#);
    assert!(try_wait_until(|| first.heard(1) == Some(true), SETTLE), "{:?}", first.status());
}

#[test]
fn a_switched_on_node_without_an_address_sends_nothing_and_says_why() {
    let hub = Arc::new(ClusterHub::new());
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(&hub), FIRST.address)),
        timing: PresenceTiming::every_ms(PULSE_PERIOD_MS),
        boot_epoch: fixed_boot_epoch(FIRST.epoch),
    };
    let node = composed(MockNetworkLink::new(FIRST.mac, None, "cluster-node"), Some(cluster));
    node.join(0);
    assert!(try_wait_until(|| node.state() == "no_address", SETTLE), "state {}", node.state());
    assert!(node.status()["own"]["boot_epoch"].is_null(), "{:?}", node.status());
    assert!(hub.next_sent(Duration::from_millis(u64::from(PULSE_PERIOD_MS) * 2)).is_none());
}

#[test]
fn a_composition_that_wires_no_cluster_port_shows_bind_failed() {
    let node = composed(MockNetworkLink::new(FIRST.mac, Some(FIRST.address), "cluster-node"), None);
    node.join(0);
    assert!(try_wait_until(|| node.state() == "bind_failed", SETTLE), "state {}", node.state());
    assert!(node.status()["own"]["boot_epoch"].is_null(), "{:?}", node.status());
}

#[test]
fn two_nodes_learn_each_other_s_records_and_parts_and_a_rename_crosses_over() {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.join(0);
    second.join(1);
    first.settled();
    second.settled();
    let corridor = first.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    second.send("POST", "/api/v1/cluster/scenes", r#"{"number":17,"name":"Evening"}"#);
    let named = serde_json::json!({ "number": 2, "birth_mark": corridor["birth_mark"] });
    let part = serde_json::json!({ "record": named, "send": true, "accept": true }).to_string();
    second.send("PUT", "/api/v1/cluster/channel-parts/0/3", &part);
    let learnt = || {
        let (channel, scene) = (second.read("/api/v1/cluster/channels/2"), first.read("/api/v1/cluster/scenes/17"));
        let part = first.read("/api/v1/cluster/channels/2")["parts"][0].clone();
        channel["name"] == "Corridor" && channel["source"] == "cached" && scene["name"] == "Evening"
            && part["node"] == 1 && part["group"] == 3 && part["link"] == "live"
    };
    let (first_info, second_info) = (|| first.read("/api/v1/cluster/info"), || second.read("/api/v1/cluster/info"));
    assert!(try_wait_until(learnt, SETTLE), "first {:?}, second {:?}", first_info(), second_info());
    first.send("PATCH", "/api/v1/cluster/channels/2", r#"{"name":"Hall","revision":1}"#);
    let renamed = || second.read("/api/v1/cluster/channels/2")["name"] == "Hall";
    assert!(try_wait_until(renamed, SETTLE), "{:?}", second.read("/api/v1/cluster/channels/2"));
}

fn is_info_page(bytes: &[u8]) -> bool {
    decode(bytes).is_ok_and(|decoded| matches!(decoded.frame, Frame::ChannelSceneInfo(_)))
}

fn copy_numbers(sent: &HubDatagram, source: [u8; 4]) -> Option<Vec<u8>> {
    let decoded = decode(&sent.bytes).ok().filter(|_| sent.source == source)?;
    let Frame::ChannelSceneInfo(page) = decoded.frame else {
        return None;
    };
    let whole = page.meta().page == 0 && page.meta().last;
    let numbers = page.elements().filter_map(|element| match element {
        InfoElement::ChannelRecord(record) => Some(record.id.number),
        _ => None,
    });
    whole.then(|| numbers.collect())
}

fn lets_go(hub: &ClusterHub, source: [u8; 4], (kept, dropped): (u8, u8)) -> bool {
    let deadline = Instant::now() + ROUND;
    std::iter::from_fn(|| hub.next_sent(deadline.saturating_duration_since(Instant::now())))
        .filter_map(|sent| copy_numbers(&sent, source))
        .any(|numbers| numbers.contains(&kept) && !numbers.contains(&dropped))
}

fn quick(hub: &Arc<ClusterHub>, board: &Board) -> Node {
    node_timed(hub, board, fixed_boot_epoch(board.epoch), PresenceTiming::every_ms(QUICK_PULSE_MS))
}

fn part_body(record: &Value) -> String {
    let named = serde_json::json!({ "number": record["number"], "birth_mark": record["birth_mark"] });
    serde_json::json!({ "record": named, "send": true, "accept": true }).to_string()
}

impl Node {
    fn part_link(&self, node: u8, group: u64) -> Value {
        let parts = self.read(&format!("/api/v1/cluster/nodes/{node}/channel-parts"));
        let rows = parts["channel_parts"].as_array().cloned().unwrap_or_default();
        rows.into_iter().find(|row| row["group"] == group).map_or(Value::Null, |row| row["link"].clone())
    }

    fn stat(&self, name: &str) -> u64 {
        self.read("/api/v1/stats")["cluster"][name].as_u64().unwrap_or_default()
    }
}

#[test]
fn a_replaced_home_node_takes_its_block_back_from_a_neighbour_s_copies() {
    let hub = Arc::new(ClusterHub::new());
    let (home, keeper) = (quick(&hub, &FIRST), quick(&hub, &SECOND));
    home.join(0);
    keeper.join(1);
    home.settled();
    keeper.settled();
    let corridor = home.send("POST", "/api/v1/cluster/channels", r#"{"name":"Corridor"}"#);
    let porch = home.send("POST", "/api/v1/cluster/channels", r#"{"name":"Porch"}"#);
    assert_eq!((corridor["number"].as_u64(), porch["number"].as_u64()), (Some(0), Some(1)));
    keeper.send("PUT", "/api/v1/cluster/channel-parts/0/3", &part_body(&corridor));
    keeper.send("PUT", "/api/v1/cluster/channel-parts/0/4", &part_body(&porch));
    assert!(try_wait_until(|| keeper.stat("copies_stored_total") == 2, ROUND), "{:?}", keeper.read("/api/v1/stats"));
    let deleted = home.router.dispatch("DELETE", "/api/v1/cluster/channels/1", br#"{"revision":1}"#).status;
    assert_eq!(deleted, NO_CONTENT);
    let judged = || keeper.part_link(1, 4) == "orphan";
    assert!(try_wait_until(judged, ROUND), "{:?}", keeper.read("/api/v1/cluster/nodes/1/channel-parts"));
    assert!(lets_go(&hub, SECOND.address, (0, 1)), "the keeper still announces the copy of the deleted record");
    drop(home);
    let replaced = quick(&hub, &REPLACEMENT);
    replaced.join(0);
    assert!(try_wait_until(|| replaced.block() == "settled", ROUND), "{:?}", replaced.status());
    took_its_block_back(&replaced, &keeper, &corridor);
}

#[test]
fn a_home_node_replaced_while_its_keeper_went_unheard_takes_its_block_back_late() {
    let hub = Arc::new(ClusterHub::new());
    let (home, keeper) = (quick(&hub, &FIRST), quick(&hub, &SECOND));
    home.join(0);
    keeper.join(1);
    home.settled();
    keeper.settled();
    let corridor = home.send("POST", "/api/v1/cluster/channels", r#"{"name":"Corridor"}"#);
    let porch = home.send("POST", "/api/v1/cluster/channels", r#"{"name":"Porch"}"#);
    keeper.send("PUT", "/api/v1/cluster/channel-parts/0/3", &part_body(&corridor));
    keeper.send("PUT", "/api/v1/cluster/channel-parts/0/4", &part_body(&porch));
    assert!(try_wait_until(|| keeper.stat("copies_stored_total") == 2, ROUND), "{:?}", keeper.read("/api/v1/stats"));
    assert_eq!(home.router.dispatch("DELETE", "/api/v1/cluster/channels/1", br#"{"revision":1}"#).status, NO_CONTENT);
    assert!(lets_go(&hub, SECOND.address, (0, 1)), "the keeper still announces the copy of the deleted record");
    drop(home);
    hub.lose_sends(|_| true);
    let replaced = quick(&hub, &REPLACEMENT);
    replaced.join(0);
    assert!(try_wait_until(|| replaced.block() == "settled", ROUND), "{:?}", replaced.status());
    assert_eq!(replaced.router.dispatch("GET", "/api/v1/cluster/channels/0", &[]).status, 404, "nothing was heard");
    let mut first = true;
    hub.lose_sends(move |sent| {
        let lost = first && sent.source == SECOND.address && is_info_page(&sent.bytes);
        first &= !lost;
        lost
    });
    let back = || replaced.router.dispatch("GET", "/api/v1/cluster/channels/0", &[]).status == OK;
    assert!(try_wait_until(back, ROUND), "{:?}", replaced.read("/api/v1/cluster/channels"));
    took_its_block_back(&replaced, &keeper, &corridor);
}

fn took_its_block_back(replaced: &Node, keeper: &Node, corridor: &Value) {
    let taken = replaced.read("/api/v1/cluster/channels/0");
    assert_eq!((taken["name"].as_str(), taken["source"].as_str()), (Some("Corridor"), Some("own")), "{taken:?}");
    assert_eq!(taken["birth_mark"], corridor["birth_mark"]);
    assert_eq!(replaced.router.dispatch("GET", "/api/v1/cluster/channels/1", &[]).status, 404, "deleted on purpose");
    let next = replaced.send("POST", "/api/v1/cluster/channels", r#"{"name":"Stairs"}"#);
    assert_eq!(next["number"].as_u64(), Some(2), "a number in use or deleted is not issued again: {next:?}");
    assert_eq!(replaced.stat("records_taken_back_total"), 1);
    let live = || keeper.part_link(1, 3) == "live";
    assert!(try_wait_until(live, ROUND), "{:?}", keeper.read("/api/v1/cluster/nodes/1/channel-parts"));
}
