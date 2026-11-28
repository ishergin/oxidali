use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::frame::{decode, Command, Frame, SourceClass};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, MockNetworkLink, SharedWire, Speaker};
use serde_json::{json, Value};

const PULSE_PERIOD_MS: u32 = 250;
const SLOW_PULSE_PERIOD_MS: u32 = 1_000;
const SETTLE: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_secs(1);
const OK: u16 = 200;
const CREATED: u16 = 201;
const ACCEPTED: u16 = 202;
const GROUP_5_DAPC_200: u16 = 0x8AC8;
const GROUP_5_STEP_UP: u16 = 0x8B03;
const GROUP_5_UP: u16 = 0x8B01;
const STEPS: usize = 3;
const HELD_KEY_UPS: usize = 20;

type Wire = Arc<SharedWire<SimDaliTransport>>;

struct Node {
    router: Router,
    wire: Wire,
    tap: usize,
    _runtime: Box<BusStackRuntime>,
}

struct Board {
    mac: [u8; 6],
    address: [u8; 4],
    epoch: u32,
}

const OWNER: Board = Board { mac: [0x02, 0, 0, 0x0c, 0x0d, 0x01], address: [10, 0, 1, 1], epoch: 31 };
const BESIDE: Board = Board { mac: [0x02, 0, 0, 0x0c, 0x0d, 0x02], address: [10, 0, 1, 2], epoch: 32 };
const APART: Board = Board { mac: [0x02, 0, 0, 0x0c, 0x0d, 0x03], address: [10, 0, 1, 3], epoch: 33 };

fn shared_wire() -> Wire {
    SharedWire::new(SimDaliTransport::inject_foreign_frame)
}

fn node_on(hub: &Arc<ClusterHub>, board: &Board, wire: &Wire, period_ms: u32) -> Node {
    let cluster = ClusterComposition {
        binder: Arc::new(ClusterHubBinder::new(Arc::clone(hub), board.address)),
        timing: PresenceTiming::every_ms(period_ms),
        boot_epoch: fixed_boot_epoch(board.epoch),
    };
    let link = MockNetworkLink::new(board.mac, Some(board.address), "cluster-node");
    let tap = wire.tap(SimDaliTransport::new(Vec::new()));
    let index = tap.index();
    let (router, _hub, runtime) = build_http_test_stack(
        "0.1.0-test",
        Arc::new(Mutex::new(tap)),
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        None,
        &[],
        HttpTestPorts { network_link: Some(Arc::new(link)), cluster: Some(cluster), ..HttpTestPorts::default() },
    );
    Node { router, wire: Arc::clone(wire), tap: index, _runtime: runtime }
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

    fn join(&self, settings: &Value) {
        self.send("PATCH", "/api/v1/cluster", &settings.to_string());
    }

    fn running(&self) -> bool {
        self.read("/api/v1/cluster/status")["own"]["state"] == "running"
    }

    fn settled(&self) -> bool {
        self.read("/api/v1/cluster/status")["own"]["block"] == "settled"
    }

    fn part(&self, reference: &Value, send: bool) {
        let part = json!({ "record": reference, "send": send, "accept": true }).to_string();
        self.send("PUT", "/api/v1/cluster/channel-parts/0/5", &part);
    }

    fn link(&self, send: bool) {
        let record = "/api/v1/cluster/channels/2";
        assert!(try_wait_until(|| self.read(record)["source"] == "cached", SETTLE), "{record}: {}", self.read(record));
        let held = self.read(record);
        self.part(&json!({ "number": held["number"], "birth_mark": held["birth_mark"] }), send);
    }

    fn sent_frames(&self) -> Vec<u16> {
        self.wire.sent_by(self.tap)
    }

    fn stat(&self, counter: &str) -> u64 {
        self.read("/api/v1/stats")["cluster"][counter].as_u64().unwrap_or(u64::MAX)
    }
}

struct Cluster {
    hub: Arc<ClusterHub>,
    shared: Wire,
    apart: Wire,
    owner: Node,
    beside: Node,
    third: Node,
}

fn merged(base: Value, extra: &Value) -> Value {
    let mut base = base;
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        base.extend(extra.clone());
    }
    base
}

fn settings(node_number: u8, extra: &Value) -> Value {
    merged(json!({ "cluster_name": "hall", "node_number": node_number, "enabled": true }), extra)
}

fn cluster_with(owner: &Value, beside: &Value) -> Cluster {
    let hub = Arc::new(ClusterHub::new());
    let (shared, apart) = (shared_wire(), shared_wire());
    let cluster = Cluster {
        owner: node_on(&hub, &OWNER, &shared, PULSE_PERIOD_MS),
        beside: node_on(&hub, &BESIDE, &shared, PULSE_PERIOD_MS),
        third: node_on(&hub, &APART, &apart, PULSE_PERIOD_MS),
        hub,
        shared,
        apart,
    };
    cluster.owner.join(&settings(0, owner));
    cluster.beside.join(&settings(1, &merged(json!({ "wire_same_as": 0 }), beside)));
    cluster.third.join(&settings(2, &json!({})));
    let all_run = || cluster.owner.running() && cluster.beside.running() && cluster.third.running();
    assert!(try_wait_until(all_run, SETTLE), "the three nodes run");
    assert!(try_wait_until(|| cluster.owner.settled(), SETTLE), "the owner's block settled");
    let channel = cluster.owner.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    cluster.owner.part(&json!({ "number": channel["number"], "birth_mark": channel["birth_mark"] }), true);
    cluster.beside.link(true);
    cluster.third.link(true);
    cluster
}

fn forwarding_cluster() -> Cluster {
    let forward = json!({ "forward_foreign_master": true });
    let cluster = cluster_with(&forward, &forward);
    let owns = || cluster.owner.read("/api/v1/cluster/status")["own"]["wire_owner"]["this_node"] == true;
    assert!(try_wait_until(owns, SETTLE), "the lower node owns the shared wire: {}", cluster.owner.read("/api/v1/cluster/status"));
    cluster
}

struct Carried(Vec<([u8; 4], Command)>);

impl Carried {
    fn during(hub: &ClusterHub, window: Duration) -> Self {
        let deadline = Instant::now() + window;
        let mut commands = Vec::new();
        while let Some(sent) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
            if let Ok(Frame::Command(command)) = decode(&sent.bytes).map(|decoded| decoded.frame) {
                commands.push((sent.source, command));
            }
        }
        Self(commands)
    }

    fn from(&self, board: &Board) -> Vec<Command> {
        self.0.iter().filter(|(source, _)| *source == board.address).map(|(_, command)| command.clone()).collect()
    }
}

fn next_pulse_from(hub: &ClusterHub, board: &Board, within: Duration) -> Option<bool> {
    let deadline = Instant::now() + within;
    while let Some(sent) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
        let frame = decode(&sent.bytes).map(|decoded| decoded.frame);
        if let (true, Ok(Frame::Pulse(pulse))) = (sent.source == board.address, frame) {
            return Some(pulse.wire_protection);
        }
    }
    None
}

fn foreign_frames(wire: &Wire) -> usize {
    wire.frames().iter().filter(|heard| heard.speaker == Speaker::Foreign).count()
}

#[test]
fn a_foreign_setpoint_on_the_shared_wire_reaches_the_third_node_once() {
    let cluster = forwarding_cluster();
    assert_eq!(cluster.shared.foreign(GROUP_5_DAPC_200), 2, "both nodes on the wire hear the foreign master");
    let reached = || cluster.third.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(reached, SETTLE), "the third node's wire: {:04X?}", cluster.third.sent_frames());
    let skipped = || cluster.beside.stat("forwards_skipped_wire_total") == 1;
    assert!(try_wait_until(skipped, SETTLE), "the node beside the owner heard it and left it: {}", cluster.beside.read("/api/v1/stats")["cluster"]);
    let sources: Vec<SourceClass> = Carried::during(&cluster.hub, QUIET).from(&OWNER).iter().map(|command| command.source).collect();
    assert!(!sources.is_empty() && sources.iter().all(|source| *source == SourceClass::ForeignMaster), "{sources:?}");
    assert_eq!(cluster.third.sent_frames(), [GROUP_5_DAPC_200], "the third node ran it once");
    assert_eq!(cluster.shared.frames().len(), 1, "nothing ran it again on the shared wire: {:04X?}", cluster.shared.frames());
    assert_eq!(cluster.owner.stat("foreign_forwarded_total"), 1);
    assert_eq!(cluster.owner.stat("foreign_held_total"), 1, "a neighbour shares the wire, so the owner held it");
    assert_eq!(cluster.apart.frames().len(), 1, "the third node's wire carried one frame");
}

#[test]
fn a_command_from_the_third_node_runs_once_on_the_shared_wire_and_never_comes_back() {
    let cluster = forwarding_cluster();
    cluster.third.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    let reached = || cluster.owner.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(reached, SETTLE), "the owner's wire: {:04X?}", cluster.owner.sent_frames());
    let declined = || cluster.beside.stat("commands_dropped_not_wire_owner_total") >= 1;
    assert!(try_wait_until(declined, SETTLE), "the node beside the owner left it: {}", cluster.beside.read("/api/v1/stats")["cluster"]);
    let heard = || cluster.beside.stat("forwards_skipped_wire_total") == 1;
    assert!(try_wait_until(heard, SETTLE), "the node beside heard the owner's frame as foreign: {}", cluster.beside.read("/api/v1/stats")["cluster"]);
    let carried = Carried::during(&cluster.hub, QUIET);
    assert_eq!(carried.from(&OWNER), Vec::<Command>::new(), "the owner sends nothing back");
    assert_eq!(carried.from(&BESIDE), Vec::<Command>::new(), "nor does the node beside it");
    assert_eq!(cluster.shared.frames().len(), 1, "the shared wire ran it once: {:04X?}", cluster.shared.frames());
    assert_eq!(cluster.apart.sent_by(0), [GROUP_5_DAPC_200], "the third node ran its own command once");
    assert_eq!(cluster.owner.stat("foreign_forwarded_total"), 0);
}

#[test]
fn without_protection_a_command_runs_on_both_nodes_of_the_wire_and_the_unprotected_owner_sends_nothing_back() {
    let unprotected = json!({ "forward_foreign_master": true, "wire_protection": false });
    let cluster = cluster_with(&unprotected, &json!({ "wire_protection": false }));
    let fault = || cluster.owner.read("/api/v1/cluster/status")["own"]["unprotected_wire"] == true;
    assert!(try_wait_until(fault, SETTLE), "the owner reports the unprotected wire: {}", cluster.owner.read("/api/v1/cluster/status"));
    cluster.third.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    let both = || cluster.owner.sent_frames() == [GROUP_5_DAPC_200] && cluster.beside.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(both, SETTLE), "without protection both run it: {:04X?}", cluster.shared.frames());
    let heard = || cluster.owner.stat("forwards_skipped_wire_total") == 1 && cluster.beside.stat("forwards_skipped_source_off_total") == 1;
    assert!(try_wait_until(heard, SETTLE), "each heard the other's frame: {} {}", cluster.owner.read("/api/v1/stats")["cluster"], cluster.beside.read("/api/v1/stats")["cluster"]);
    let carried = Carried::during(&cluster.hub, QUIET);
    assert_eq!(carried.from(&OWNER), Vec::<Command>::new(), "the owner sends nothing back");
    assert_eq!(carried.from(&BESIDE), Vec::<Command>::new(), "nor does the node beside it");
    assert_eq!(cluster.apart.sent_by(0), [GROUP_5_DAPC_200], "the third node ran its own command once");
}

#[test]
fn three_foreign_steps_on_the_shared_wire_arrive_as_three() {
    let cluster = forwarding_cluster();
    for _ in 0..STEPS {
        cluster.shared.foreign(GROUP_5_STEP_UP);
    }
    let reached = || cluster.third.sent_frames() == [GROUP_5_STEP_UP; STEPS];
    assert!(try_wait_until(reached, SETTLE), "the third node's wire: {:04X?}", cluster.third.sent_frames());
    assert!(!try_wait_until(|| cluster.third.sent_frames().len() > STEPS, QUIET), "a step ran twice: {:04X?}", cluster.third.sent_frames());
    assert_eq!(cluster.owner.stat("foreign_forwarded_total"), 3);
    assert_eq!(cluster.shared.frames().len(), STEPS, "nothing ran them again on the shared wire");
}

#[test]
fn a_held_foreign_key_does_not_storm_and_every_up_reaches_the_third_node_or_is_counted() {
    let cluster = forwarding_cluster();
    for sent in 1..=HELD_KEY_UPS {
        cluster.shared.foreign(GROUP_5_UP);
        let heard = || cluster.owner.stat("foreign_held_total") >= u64::try_from(sent).unwrap_or(u64::MAX);
        assert!(try_wait_until(heard, SETTLE), "the owner held UP {sent}");
    }
    let total = u64::try_from(HELD_KEY_UPS).unwrap_or(u64::MAX);
    let settled = || {
        let reached = u64::try_from(cluster.third.sent_frames().len()).unwrap_or(0);
        reached + cluster.owner.stat("relative_verbs_dropped_total") == total
    };
    assert!(try_wait_until(settled, SETTLE), "{:04X?} {}", cluster.third.sent_frames(), cluster.owner.read("/api/v1/stats")["cluster"]);
    assert!(cluster.third.sent_frames().iter().all(|frame| *frame == GROUP_5_UP), "{:04X?}", cluster.third.sent_frames());
    assert_eq!(foreign_frames(&cluster.shared), HELD_KEY_UPS);
    assert_eq!(cluster.shared.frames().len(), HELD_KEY_UPS, "nothing ran an UP again on the shared wire");
    assert_eq!(cluster.owner.stat("foreign_forwarded_total"), total, "the hold let every UP through");
    let carried = Carried::during(&cluster.hub, QUIET);
    let copies = usize::try_from(cluster.owner.stat("commands_sent_total")).unwrap_or(usize::MAX) * 2;
    assert_eq!(carried.from(&OWNER).len(), copies, "each UP the owner sent went out as its two copies");
    assert!(carried.from(&OWNER).iter().all(|command| command.source == SourceClass::ForeignMaster));
    assert_eq!(carried.from(&BESIDE), Vec::<Command>::new(), "the node beside the owner forwarded nothing");
    assert_eq!(Carried::during(&cluster.hub, QUIET).0.iter().filter(|(source, _)| *source != APART.address).count(), 0, "the cluster is quiet once the key is released");
}

#[test]
fn protection_switched_off_beside_the_owner_is_pulsed_at_once() {
    let hub = Arc::new(ClusterHub::new());
    let shared = shared_wire();
    let owner = node_on(&hub, &OWNER, &shared, SLOW_PULSE_PERIOD_MS);
    let beside = node_on(&hub, &BESIDE, &shared, SLOW_PULSE_PERIOD_MS);
    owner.join(&settings(0, &json!({ "forward_foreign_master": true })));
    beside.join(&settings(1, &json!({ "wire_same_as": 0 })));
    assert!(try_wait_until(|| owner.running() && beside.running(), SETTLE), "both nodes run");
    while hub.next_sent(Duration::ZERO).is_some() {}
    assert_eq!(next_pulse_from(&hub, &BESIDE, SETTLE), Some(true), "a fresh pulse opens the period");
    beside.join(&json!({ "wire_protection": false }));
    let half = Duration::from_millis(u64::from(SLOW_PULSE_PERIOD_MS / 2));
    assert_eq!(next_pulse_from(&hub, &BESIDE, half), Some(false), "the changed pulse leaves before its period");
    let faulted = || owner.read("/api/v1/cluster/status")["own"]["unprotected_wire"] == true;
    assert!(try_wait_until(faulted, SETTLE), "{}", owner.read("/api/v1/cluster/status"));
}

#[test]
fn a_command_beside_the_owner_that_the_cluster_does_not_carry_stays_on_the_shared_wire() {
    let cluster = cluster_with(&json!({ "forward_foreign_master": true }), &json!({ "forward_operator": false }));
    let owns = || cluster.owner.read("/api/v1/cluster/status")["own"]["wire_owner"]["this_node"] == true;
    assert!(try_wait_until(owns, SETTLE), "{}", cluster.owner.read("/api/v1/cluster/status"));
    cluster.beside.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    let ran = || cluster.beside.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(ran, SETTLE), "the node beside ran it: {:04X?}", cluster.shared.frames());
    let cancelled = || cluster.owner.stat("foreign_cancelled_total") == 1;
    assert!(try_wait_until(cancelled, SETTLE), "the owner took the note: {}", cluster.owner.read("/api/v1/stats")["cluster"]);
    let carried = Carried::during(&cluster.hub, QUIET);
    assert_eq!(carried.from(&OWNER), Vec::<Command>::new(), "the owner forwarded nothing");
    assert_eq!(carried.from(&BESIDE), Vec::<Command>::new(), "the operator's commands stay home on the node beside");
    assert!(cluster.apart.frames().is_empty(), "the third node's wire stays quiet: {:04X?}", cluster.apart.frames());
    assert_eq!(cluster.owner.stat("foreign_forwarded_total"), 0);
    assert_eq!((cluster.beside.stat("wire_notes_sent_total"), cluster.owner.stat("wire_notes_taken_total")), (1, 1));
}

#[test]
fn a_channel_step_typed_beside_the_owner_is_cancelled_once_and_a_held_foreign_step_still_goes() {
    let cluster = forwarding_cluster();
    cluster.shared.foreign(GROUP_5_UP);
    let held = || cluster.owner.stat("foreign_held_total") == 1;
    assert!(try_wait_until(held, SETTLE), "{}", cluster.owner.read("/api/v1/stats")["cluster"]);
    cluster.beside.send("PUT", "/api/v1/cluster/channels/2/target-state", r#"{"dim":"up"}"#);
    let both = || cluster.third.sent_frames() == [GROUP_5_UP, GROUP_5_UP];
    assert!(try_wait_until(both, SETTLE), "the third node steps as often as the shared wire: {:04X?} {}", cluster.third.sent_frames(), cluster.owner.read("/api/v1/stats")["cluster"]);
    assert!(!try_wait_until(|| cluster.third.sent_frames().len() > 2, QUIET), "a step ran three times: {:04X?}", cluster.third.sent_frames());
    assert_eq!(cluster.shared.frames().len(), 2, "{:04X?}", cluster.shared.frames());
    assert_eq!(cluster.beside.stat("wire_notes_sent_total"), 0, "the channel command carries its own frame");
}
