use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, MockNetworkLink};
use serde_json::{json, Value};

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const OK: u16 = 200;
const UNAVAILABLE: u16 = 503;
const CREATED: u16 = 201;
const GROUP_5_DAPC_200: u16 = 0x8AC8;
const GROUP_5_DAPC_100: u16 = 0x8A64;
const BROADCAST_RECALL_SCENE_6: u16 = 0xFF16;

struct Node {
    router: Router,
    wire: Arc<Mutex<MockDaliTransport>>,
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
        boot_epoch: fixed_boot_epoch(board.epoch),
    };
    let link = MockNetworkLink::new(board.mac, Some(board.address), "cluster-node");
    let wire = Arc::new(Mutex::new(MockDaliTransport::new()));
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        Arc::clone(&wire) as _,
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        None,
        &[],
        HttpTestPorts { network_link: Some(Arc::new(link)), cluster: Some(cluster), ..HttpTestPorts::default() },
    );
    Node { router, wire, _runtime: runtime }
}

impl Node {
    fn send(&self, method: &str, path: &str, body: &str) -> Value {
        let response = self.router.dispatch(method, path, body.as_bytes());
        let status = response.status;
        let answer = serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null);
        assert!([OK, CREATED].contains(&status), "{method} {path} answered {status}: {answer}");
        answer
    }

    fn read(&self, path: &str) -> Value {
        let response = self.router.dispatch("GET", path, &[]);
        serde_json::from_slice(&response.into_body_bytes()).unwrap_or(Value::Null)
    }

    fn join(&self, node_number: u8) {
        self.send("PATCH", "/api/v1/cluster", &json!({ "cluster_name": "hall", "node_number": node_number, "enabled": true }).to_string());
    }

    fn running(&self) -> bool {
        self.read("/api/v1/cluster/status")["own"]["state"] == "running"
    }

    fn link(&self, part: &str, record: &str) {
        assert!(try_wait_until(|| self.read(record)["source"] == "cached", SETTLE), "{record}: {}", self.read(record));
        let held = self.read(record);
        let reference = json!({ "number": held["number"], "birth_mark": held["birth_mark"] });
        self.send("PUT", part, &json!({ "record": reference, "send": true, "accept": true }).to_string());
    }

    fn sent_frames(&self) -> Vec<u16> {
        self.wire.lock().expect("the mock transport").sent_frames()
    }
}

fn running_pair() -> (Arc<ClusterHub>, Node, Node) {
    let hub = Arc::new(ClusterHub::new());
    let (first, second) = (node_on(&hub, &FIRST), node_on(&hub, &SECOND));
    first.join(0);
    second.join(1);
    assert!(try_wait_until(|| first.running() && second.running(), SETTLE), "both nodes run");
    (hub, first, second)
}

#[test]
fn a_channel_command_put_on_one_node_reaches_the_linked_group_on_the_other() {
    let (_hub, first, second) = running_pair();
    first.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    second.link("/api/v1/cluster/channel-parts/0/5", "/api/v1/cluster/channels/2");
    let answer = first.send("PUT", "/api/v1/cluster/channels/2/target-state", r#"{"power":"on","level":200}"#);
    assert_eq!(answer["status"], "sent");
    let reached = || second.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(reached, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    assert_eq!(first.sent_frames(), Vec::<u16>::new(), "the first node holds no part of the channel");
    assert_eq!(first.read("/api/v1/stats")["cluster"]["commands_sent_total"], 1);
}

#[test]
fn a_scene_recalled_on_one_node_is_recalled_on_the_other_node_s_scene_part() {
    let (_hub, first, second) = running_pair();
    first.send("POST", "/api/v1/cluster/scenes", r#"{"number":3,"name":"Evening"}"#);
    second.link("/api/v1/cluster/scene-parts/0/6", "/api/v1/cluster/scenes/3");
    let answer = first.send("POST", "/api/v1/cluster/scenes/3/recall", "");
    assert_eq!(answer["status"], "sent");
    let reached = || second.sent_frames() == [BROADCAST_RECALL_SCENE_6];
    assert!(try_wait_until(reached, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    assert_eq!(first.sent_frames(), Vec::<u16>::new());
}

#[test]
fn a_node_whose_command_cannot_leave_does_not_execute_its_own_part() {
    let hub = Arc::new(ClusterHub::new());
    let node = node_on(&hub, &FIRST);
    node.join(0);
    assert!(try_wait_until(|| node.running(), SETTLE), "the node runs");
    let channel = node.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    let reference = json!({ "number": channel["number"], "birth_mark": channel["birth_mark"] });
    node.send("PUT", "/api/v1/cluster/channel-parts/0/5", &json!({ "record": reference, "send": false, "accept": true }).to_string());
    hub.refuse_sends(true);
    let refused = node.router.dispatch("PUT", "/api/v1/cluster/channels/2/target-state", br#"{"power":"on","level":200}"#);
    assert_eq!(refused.status, UNAVAILABLE);
    hub.refuse_sends(false);
    node.send("PUT", "/api/v1/cluster/channels/2/target-state", r#"{"power":"on","level":100}"#);
    assert!(try_wait_until(|| !node.sent_frames().is_empty(), SETTLE), "the own part executes once the frame leaves");
    assert_eq!(node.sent_frames(), [GROUP_5_DAPC_100], "the refused command never reached the wire");
}
