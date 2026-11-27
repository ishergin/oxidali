use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::frame::{cluster_tag, decode, Frame};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, MockNetworkLink};
use serde_json::Value;

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const CLUSTER_NAME: &str = "hall";
const SETTINGS: &str = "/api/v1/cluster";
const STATUS: &str = "/api/v1/cluster/status";
const OK: u16 = 200;
const NO_CONTENT: u16 = 204;
const CONFLICT: u16 = 409;

struct Node {
    router: Router,
    _runtime: Box<BusStackRuntime>,
}

struct Board {
    mac: [u8; 6],
    address: [u8; 4],
    epoch: u32,
}

const FIRST: Board = Board { mac: [0x02, 0, 0, 0x0a, 0x0b, 0x01], address: [10, 0, 0, 1], epoch: 11 };
const SECOND: Board = Board { mac: [0x02, 0, 0, 0x0a, 0x0b, 0x02], address: [10, 0, 0, 2], epoch: 22 };

fn node_on(hub: &Arc<ClusterHub>, board: &Board) -> Node {
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(hub), board.address)),
        timing: PresenceTiming::every_ms(PULSE_PERIOD_MS),
        boot_epoch: board.epoch,
    };
    composed(MockNetworkLink::new(board.mac, Some(board.address), "cluster-node"), Some(cluster))
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

    fn state(&self) -> String {
        self.status()["own"]["state"].as_str().unwrap_or_default().to_owned()
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
    let datagram = (0..)
        .map_while(|_| hub.next_sent(SETTLE))
        .find(|sent| matches!(decode(&sent.bytes), Ok(decoded) if matches!(decoded.frame, Frame::Pulse(_))))
        .expect("the node pulses");
    let decoded = decode(&datagram.bytes).expect("a production frame");
    assert_eq!(datagram.source, FIRST.address);
    assert_eq!(decoded.header.sender_mac, FIRST.mac);
    assert_eq!(decoded.header.node_no, 3);
    assert_eq!(decoded.header.boot_epoch, FIRST.epoch);
    assert_eq!(decoded.header.cluster_tag, cluster_tag(CLUSTER_NAME));
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
fn a_switched_on_node_without_an_address_sends_nothing_and_says_why() {
    let hub = Arc::new(ClusterHub::new());
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(&hub), FIRST.address)),
        timing: PresenceTiming::every_ms(PULSE_PERIOD_MS),
        boot_epoch: FIRST.epoch,
    };
    let node = composed(MockNetworkLink::new(FIRST.mac, None, "cluster-node"), Some(cluster));
    node.join(0);
    assert!(try_wait_until(|| node.state() == "no_address", SETTLE), "state {}", node.state());
    assert!(hub.next_sent(Duration::from_millis(u64::from(PULSE_PERIOD_MS) * 2)).is_none());
}

#[test]
fn a_composition_that_wires_no_cluster_port_shows_bind_failed() {
    let node = composed(MockNetworkLink::new(FIRST.mac, Some(FIRST.address), "cluster-node"), None);
    node.join(0);
    assert!(try_wait_until(|| node.state() == "bind_failed", SETTLE), "state {}", node.state());
}
