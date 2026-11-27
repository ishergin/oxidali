use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{
    build_http_test_stack, fixed_boot_epoch, BusStackRuntime, ClusterComposition, DaliRuntimeConfig, HttpTestPorts,
};
use dali2rust_api::http::router::Router;
use dali2rust_bus::BusConfig;
use dali2rust_cluster_runtime::frame::{decode, Command, Frame, SourceClass};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_test_support::{try_wait_until, ClusterHub, ClusterHubBinder, MockNetworkLink};
use serde_json::{json, Value};

const PULSE_PERIOD_MS: u32 = 250;
const SETTLE: Duration = Duration::from_secs(10);
const OK: u16 = 200;
const UNAVAILABLE: u16 = 503;
const CREATED: u16 = 201;
const ACCEPTED: u16 = 202;
const GROUP_5_DAPC_200: u16 = 0x8AC8;
const BROADCAST_RECALL_SCENE_6: u16 = 0xFF16;
const GROUP_5_DAPC_80: u16 = 0x8A50;
const GROUP_5_DAPC_100: u16 = 0x8A64;
const GROUP_5_STEP_UP: u16 = 0x8B03;
const STEPS: usize = 3;
const HELD_STEPS: usize = 5;
const HELD_STEPS_COUNT: u64 = 5;
const QUIET: Duration = Duration::from_secs(1);
const SLIDER_LEVELS: [u8; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const SCENARIO_MIDNIGHT_UNIX_MS: u64 = 1_782_259_200_000;
const TEN_O_CLOCK_MS: u64 = 600 * 60_000;

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
        assert!([OK, CREATED, ACCEPTED].contains(&status), "{method} {path} answered {status}: {answer}");
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

    fn stat(&self, counter: &str) -> u64 {
        self.read("/api/v1/stats")["cluster"][counter].as_u64().unwrap_or(u64::MAX)
    }
}

fn commands_from(hub: &ClusterHub, board: &Board, window: Duration) -> Vec<Command> {
    let deadline = Instant::now() + window;
    let mut commands = Vec::new();
    while let Some(sent) = hub.next_sent(deadline.saturating_duration_since(Instant::now())) {
        let frame = decode(&sent.bytes).map(|decoded| decoded.frame);
        if let (true, Ok(Frame::Command(command))) = (sent.source == board.address, frame) {
            commands.push(command);
        }
    }
    commands
}

fn both_send_and_accept_group_5(first: &Node, second: &Node) {
    let channel = first.send("POST", "/api/v1/cluster/channels", r#"{"number":2,"name":"Corridor"}"#);
    let reference = json!({ "number": channel["number"], "birth_mark": channel["birth_mark"] });
    let part = json!({ "record": reference, "send": true, "accept": true }).to_string();
    first.send("PUT", "/api/v1/cluster/channel-parts/0/5", &part);
    second.link("/api/v1/cluster/channel-parts/0/5", "/api/v1/cluster/channels/2");
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

#[test]
fn a_group_command_on_one_node_runs_on_the_other_and_nothing_comes_back() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    first.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    let reached = || second.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(reached, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    let declined = || second.stat("forwards_skipped_cluster_origin_total") == 1;
    assert!(try_wait_until(declined, SETTLE), "the second node saw its executed fact: {}", second.read("/api/v1/stats")["cluster"]);
    assert_eq!(commands_from(&hub, &SECOND, QUIET), Vec::<Command>::new(), "the second node sends nothing back");
    assert_eq!(first.sent_frames(), [GROUP_5_DAPC_200], "the first node ran its group once");
    assert_eq!(first.stat("commands_forwarded_total"), 1);
    assert_eq!(first.stat("commands_received_total"), 0);
    assert_eq!(second.sent_frames(), [GROUP_5_DAPC_200], "the second node ran the channel once");
}

#[test]
fn a_slider_on_one_node_reaches_the_other_as_its_last_setpoint_and_not_as_a_burst() {
    let (_hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    for level in SLIDER_LEVELS {
        first.send("PUT", "/api/v1/adapters/0/groups/5/target-state", &json!({ "power": "on", "level": level }).to_string());
    }
    let settled = || {
        let ran_here = u64::try_from(first.sent_frames().len()).unwrap_or(0);
        second.sent_frames().last() == Some(&GROUP_5_DAPC_100) && first.stat("commands_forwarded_total") == ran_here
    };
    assert!(try_wait_until(settled, SETTLE), "{:04X?} {}", second.sent_frames(), first.read("/api/v1/stats")["cluster"]);
    let forwarded = first.stat("commands_forwarded_total");
    let (sent, superseded) = (first.stat("commands_sent_total"), first.stat("commands_superseded_total"));
    assert!(superseded > 0, "setpoints inside the spacing were superseded: sent {sent}, superseded {superseded}");
    assert_eq!(sent + superseded, forwarded, "every forwarded setpoint was sent or superseded, none dropped");
    assert!(second.sent_frames().len() <= usize::try_from(sent).unwrap_or(0), "the second node ran no more than was sent");
}

#[test]
fn with_hcl_forwarded_a_tick_on_one_node_reaches_the_other_as_an_hcl_tick() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    first.send("PATCH", "/api/v1/cluster", r#"{"forward_hcl":true}"#);
    let clock = json!({ "unix_ms": SCENARIO_MIDNIGHT_UNIX_MS + TEN_O_CLOCK_MS, "timezone": "UTC0" });
    first.send("PUT", "/api/v1/time", &clock.to_string());
    let schedule = json!({ "schedule_id": "house", "enabled": true, "algorithm": "stepped",
        "active_days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"], "location": null,
        "targets": [{ "adapter_id": 0, "scope": "group", "group_ids": [5] }],
        "points": [{ "time_ref": "absolute", "offset_minutes": 540, "level_mode": "absolute", "level": 80, "color_temperature_kelvin": null }] });
    first.send("POST", "/api/v1/hcl-schedules", &schedule.to_string());
    let reached = || second.sent_frames().contains(&GROUP_5_DAPC_80);
    assert!(try_wait_until(reached, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    let sources: Vec<SourceClass> = commands_from(&hub, &FIRST, QUIET).iter().map(|command| command.source).collect();
    assert!(sources.contains(&SourceClass::Hcl), "the tick left as an HCL tick: {sources:?}");
}

fn step_up_group_5(node: &Node) {
    node.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"dim":"step_up"}"#);
}

#[test]
fn three_steps_on_one_node_are_three_steps_on_the_other_and_nothing_comes_back() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    for _ in 0..STEPS {
        step_up_group_5(&first);
    }
    let reached = || second.sent_frames() == [GROUP_5_STEP_UP; STEPS];
    assert!(try_wait_until(reached, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    let declined = || second.stat("forwards_skipped_cluster_origin_total") == 3;
    assert!(try_wait_until(declined, SETTLE), "the second node saw its executed facts: {}", second.read("/api/v1/stats")["cluster"]);
    assert_eq!(commands_from(&hub, &SECOND, QUIET), Vec::<Command>::new(), "the second node sends nothing back");
    assert_eq!(first.sent_frames(), [GROUP_5_STEP_UP; STEPS], "the first node ran each step once");
    assert_eq!((first.stat("commands_sent_total"), first.stat("commands_superseded_total")), (3, 0));
    assert_eq!(second.sent_frames(), [GROUP_5_STEP_UP; STEPS], "no step ran twice on the second node");
}

fn lose_the_first_command_from(hub: &ClusterHub, board: &Board) {
    let (source, copies) = (board.address, Arc::new(AtomicUsize::new(0)));
    hub.lose_sends(move |sent| {
        let command = matches!(decode(&sent.bytes).map(|decoded| decoded.frame), Ok(Frame::Command(_)));
        command && sent.source == source && copies.fetch_add(1, Ordering::Relaxed) < 2
    });
}

#[test]
fn a_lost_step_leaves_the_segments_apart_until_the_next_setpoint_brings_them_together() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    lose_the_first_command_from(&hub, &FIRST);
    step_up_group_5(&first);
    assert!(try_wait_until(|| first.stat("commands_sent_total") == 1, SETTLE), "the step left the first node");
    assert!(!try_wait_until(|| !second.sent_frames().is_empty(), QUIET), "the lost step ran: {:04X?}", second.sent_frames());
    assert_eq!(first.sent_frames(), [GROUP_5_STEP_UP], "the segments are apart: the first node stepped");
    first.send("PUT", "/api/v1/adapters/0/groups/5/target-state", r#"{"power":"on","level":200}"#);
    let together = || second.sent_frames() == [GROUP_5_DAPC_200];
    assert!(try_wait_until(together, SETTLE), "the second node's wire: {:04X?}", second.sent_frames());
    assert_eq!(first.sent_frames(), [GROUP_5_STEP_UP, GROUP_5_DAPC_200], "both segments end on the setpoint");
}

#[test]
fn a_step_still_waiting_when_the_cluster_goes_off_is_counted_and_never_sent_late() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    for _ in 0..HELD_STEPS {
        step_up_group_5(&first);
    }
    first.send("PATCH", "/api/v1/cluster", r#"{"enabled":false}"#);
    let settled = || {
        let (sent, dropped) = (first.stat("commands_sent_total"), first.stat("relative_verbs_dropped_total"));
        first.stat("commands_forwarded_total") == HELD_STEPS_COUNT && sent.saturating_add(dropped) == HELD_STEPS_COUNT
    };
    assert!(try_wait_until(settled, SETTLE), "every step was sent or counted: {}", first.read("/api/v1/stats")["cluster"]);
    let dropped = first.stat("relative_verbs_dropped_total");
    assert!(dropped > 0, "the cluster went off while steps waited: {}", first.read("/api/v1/stats")["cluster"]);
    let sent = usize::try_from(first.stat("commands_sent_total")).unwrap_or(usize::MAX);
    let carried = commands_from(&hub, &FIRST, QUIET).len();
    assert!((sent..=sent * 2).contains(&carried), "{carried} datagrams for {sent} sent: nothing went out after the cluster went off");
    assert_eq!(first.sent_frames(), [GROUP_5_STEP_UP; HELD_STEPS], "every step ran here once");
}

#[test]
fn a_step_that_cannot_leave_is_counted_as_dropped() {
    let (hub, first, second) = running_pair();
    both_send_and_accept_group_5(&first, &second);
    hub.refuse_sends(true);
    step_up_group_5(&first);
    let counted = || first.stat("relative_verbs_dropped_total") == 1;
    assert!(try_wait_until(counted, SETTLE), "the unsent step is counted: {}", first.read("/api/v1/stats")["cluster"]);
    assert_eq!(first.stat("commands_sent_total"), 0, "nothing left the first node");
    assert_eq!(first.sent_frames(), [GROUP_5_STEP_UP], "the first node stepped its own group");
}
