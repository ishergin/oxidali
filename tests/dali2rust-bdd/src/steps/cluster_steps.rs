use std::net::Ipv4Addr;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use dali2rust_adapters::{fixed_boot_epoch, ClusterComposition};
use dali2rust_cluster_runtime::frame::{
    cluster_tag, decode, encode, DatagramBuffer, Frame, Header, NodeClaim, Pulse,
};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_platform::net::node_name;
use dali2rust_test_support::{ClusterHub, ClusterHubBinder, HubDatagram};
use serde_json::{json, Value};

use crate::steps::last_json;
use crate::DaliWorld;

const PULSE_PERIOD_MS: u32 = 200;
const OWN_BOOT_EPOCH: u32 = 0x0C1A_0001;
const NEIGHBOUR_BOOT_EPOCH: u32 = 0x0C1A_0002;
const HUB_DEADLINE: Duration = Duration::from_secs(4);
const IMPOSTOR_ADDRESS: [u8; 4] = [10, 0, 0, 99];

pub struct ClusterRig {
    hub: Arc<ClusterHub>,
    mac: [u8; 6],
    address: [u8; 4],
    neighbours: Vec<Neighbour>,
}

impl std::fmt::Debug for ClusterRig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterRig").field("address", &self.address).finish_non_exhaustive()
    }
}

impl ClusterRig {
    pub fn composition(&self) -> ClusterComposition {
        ClusterComposition {
            binder: Arc::new(ClusterHubBinder::new(Arc::clone(&self.hub), self.address)),
            timing: PresenceTiming::every_ms(PULSE_PERIOD_MS),
            boot_epoch: fixed_boot_epoch(OWN_BOOT_EPOCH),
        }
    }

    pub(crate) fn sent_by_us(&self, window: Duration) -> Option<HubDatagram> {
        let deadline = Instant::now() + window;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let sent = self.hub.next_sent(left)?;
            if sent.source == self.address {
                return Some(sent);
            }
        }
    }

    fn start(&mut self, pulser: Pulser) {
        let (stop, stopped) = mpsc::channel();
        let speaker = pulser.speaker.clone();
        let thread = thread::spawn(move || pulser.run(&stopped));
        self.neighbours.push(Neighbour { speaker, stop: Some(stop), thread: Some(thread) });
    }

    pub(crate) fn speaker(&self, mac: [u8; 6]) -> &Speaker {
        let neighbour = self.neighbours.iter().find(|neighbour| neighbour.speaker.mac == mac);
        &neighbour.expect("the board pulses").speaker
    }

    pub(crate) const fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub(crate) fn impostor(&self, node: u8, cluster_name: String) -> Speaker {
        let (hub, source, mac) = (Arc::clone(&self.hub), IMPOSTOR_ADDRESS, self.mac);
        Speaker { hub, source, mac, node, cluster_name, seq: Arc::new(Mutex::new(0)) }
    }
}

#[derive(Clone)]
pub(crate) struct Speaker {
    hub: Arc<ClusterHub>,
    source: [u8; 4],
    mac: [u8; 6],
    node: u8,
    cluster_name: String,
    seq: Arc<Mutex<u32>>,
}

impl Speaker {
    pub(crate) fn speak(&self, write: impl FnOnce(&Header) -> Vec<u8>) {
        let mut seq = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        let header = Header {
            cluster_tag: cluster_tag(&self.cluster_name),
            sender_mac: self.mac,
            boot_epoch: NEIGHBOUR_BOOT_EPOCH,
            node_no: self.node,
            seq: *seq,
        };
        *seq = seq.wrapping_add(1);
        self.hub.inject(self.source, &write(&header));
    }

    pub(crate) fn repeat(&self, bytes: &[u8]) {
        self.hub.inject(self.source, bytes);
    }

    pub(crate) fn say(&self, buffer: &mut DatagramBuffer, frame: &Frame<'_>) {
        self.speak(|header| {
            let len = encode(header, frame, buffer).expect("the frame fits one datagram");
            buffer.get(..len).expect("the encoded length").to_vec()
        });
    }
}

struct Neighbour {
    speaker: Speaker,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Neighbour {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Pulser {
    speaker: Speaker,
    wire: u8,
    installation: String,
}

impl Pulser {
    fn new(hub: &Arc<ClusterHub>, (source, mac, node): ([u8; 4], [u8; 6], u8), wire: u8, names: [String; 2]) -> Self {
        let [cluster_name, installation] = names;
        let seq = Arc::new(Mutex::new(0));
        let speaker = Speaker { hub: Arc::clone(hub), source, mac, node, cluster_name, seq };
        Self { speaker, wire, installation }
    }

    fn run(self, stop: &Receiver<()>) {
        let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
        let period = Duration::from_millis(u64::from(PULSE_PERIOD_MS));
        loop {
            self.send(&mut buffer);
            if stop.recv_timeout(period) != Err(RecvTimeoutError::Timeout) {
                return;
            }
        }
    }

    fn send(&self, buffer: &mut DatagramBuffer) {
        let pulse = Pulse {
            cluster_name: &self.speaker.cluster_name,
            installation: &self.installation,
            claim: NodeClaim::Holding,
            wire: self.wire,
            wire_protection: true,
        };
        self.speaker.say(buffer, &Frame::Pulse(pulse));
    }
}

pub(crate) fn rig(world: &mut DaliWorld) -> &mut ClusterRig {
    world.cluster.as_mut().expect("the controller runs on a cluster hub")
}

pub(crate) fn hardware_address(text: &str) -> [u8; 6] {
    let bytes: Vec<u8> = text.split(':').map(|part| u8::from_str_radix(part, 16).expect("hex")).collect();
    bytes.try_into().expect("six octets")
}

fn ipv4(text: &str) -> [u8; 4] {
    text.parse::<Ipv4Addr>().expect("an IPv4 address").octets()
}

fn holds_node(datagram: &HubDatagram, mac: [u8; 6], node: u8, cluster_name: &str) -> bool {
    let Ok(decoded) = decode(&datagram.bytes) else {
        return false;
    };
    let header = &decoded.header;
    let names = header.sender_mac == mac && header.node_no == node && header.cluster_tag == cluster_tag(cluster_name);
    matches!(decoded.frame, Frame::Pulse(pulse) if names
        && pulse.claim == NodeClaim::Holding
        && pulse.cluster_name == cluster_name)
}

// CLU-015 CLU-016 CLU-017 CLU-018 CLU-019 CLU-020 CLU-021 CLU-022 CLU-023 CLU-024 CLU-025 CLU-026 CLU-027 CLU-057 CLU-058 CLU-059 CLU-060 CLU-061 CLU-062 CLU-063 CLU-064 CLU-065 CLU-066 CLU-067 CLU-068 CLU-069 CLU-070 CLU-071 CLU-072 CLU-073 CLU-074 CLU-075 CLU-076 CLU-077 CLU-078 CLU-079 CLU-080 CLU-081 CLU-082 CLU-083 CLU-084 CLU-085 CLU-086 CLU-087 CLU-088 CLU-089 CLU-090 CLU-091 CLU-092 CLU-093 CLU-094 CLU-095 CLU-096 CLU-097 CLU-098 CLU-099 CLU-100
#[given(regex = r#"^the controller runs on a cluster hub as board "([0-9a-f:]{17})" at "([0-9.]+)"$"#)]
async fn on_cluster_hub(world: &mut DaliWorld, mac: String, address: String) {
    let (mac, address) = (hardware_address(&mac), ipv4(&address));
    world.cluster = Some(ClusterRig { hub: Arc::new(ClusterHub::new()), mac, address, neighbours: Vec::new() });
    world.run_on_network_link(mac, Some(address), node_name(mac));
}

// CLU-016 CLU-017 CLU-018 CLU-020 CLU-022 CLU-023 CLU-024 CLU-025 CLU-027 CLU-057 CLU-058 CLU-059 CLU-060 CLU-061 CLU-062 CLU-063 CLU-064 CLU-068 CLU-069 CLU-070 CLU-071 CLU-072 CLU-073 CLU-074 CLU-075 CLU-076 CLU-077 CLU-078 CLU-079 CLU-080 CLU-081 CLU-082 CLU-083 CLU-085 CLU-086 CLU-087 CLU-088 CLU-089 CLU-090 CLU-091 CLU-092 CLU-093 CLU-094 CLU-095 CLU-096 CLU-097 CLU-098 CLU-099 CLU-100
#[when(regex = r#"^a board "([0-9a-f:]{17})" at "([0-9.]+)" pulses as node (\d) of cluster "([^"]*)" for installation "([^"]*)"$"#)]
async fn board_pulses(world: &mut DaliWorld, mac: String, source: String, node: u8, name: String, site: String) {
    let rig = rig(world);
    let (mac, source) = (hardware_address(&mac), ipv4(&source));
    let pulser = Pulser::new(&rig.hub, (source, mac, node), node, [name, site]);
    rig.start(pulser);
}

// CLU-026
#[when(regex = r#"^a board "([0-9a-f:]{17})" at "([0-9.]+)" pulses as node (\d) on the wire of node (\d) of cluster "([^"]*)" for installation "([^"]*)"$"#)]
async fn board_pulses_on_a_wire(
    world: &mut DaliWorld,
    mac: String,
    source: String,
    node: u8,
    wire: u8,
    name: String,
    site: String,
) {
    let rig = rig(world);
    let (mac, source) = (hardware_address(&mac), ipv4(&source));
    let pulser = Pulser::new(&rig.hub, (source, mac, node), wire, [name, site]);
    rig.start(pulser);
}

// CLU-019
#[when(regex = r#"^a board at "([0-9.]+)" pulses this controller's hardware address as node (\d) of cluster "([^"]*)"$"#)]
async fn board_pulses_our_address(world: &mut DaliWorld, source: String, node: u8, name: String) {
    let rig = rig(world);
    let (mac, source) = (rig.mac, ipv4(&source));
    let pulser = Pulser::new(&rig.hub, (source, mac, node), node, [name, String::new()]);
    rig.start(pulser);
}

// CLU-020 CLU-071
#[when(regex = r#"^the board "([0-9a-f:]{17})" stops pulsing$"#)]
async fn board_stops(world: &mut DaliWorld, mac: String) {
    let mac = hardware_address(&mac);
    let rig = rig(world);
    let before = rig.neighbours.len();
    rig.neighbours.retain(|neighbour| neighbour.speaker.mac != mac);
    assert!(rig.neighbours.len() < before, "no board {mac:02x?} was pulsing");
}

// CLU-023 CLU-066 CLU-068 CLU-070 CLU-076 CLU-083 CLU-084 CLU-090 CLU-091 CLU-092 CLU-093 CLU-094 CLU-095 CLU-096 CLU-097 CLU-098 CLU-099 CLU-100
#[when("the cluster hub sets aside what this controller has sent so far")]
async fn hub_sets_aside(world: &mut DaliWorld) {
    let hub = &rig(world).hub;
    while hub.next_sent(Duration::ZERO).is_some() {}
}

// CLU-015 CLU-023 CLU-067
#[then(regex = r"^the cluster hub should carry no datagram from this controller within (\d+) ms$")]
async fn hub_silent(world: &mut DaliWorld, window_ms: u64) {
    let sent = rig(world).sent_by_us(Duration::from_millis(window_ms));
    assert!(sent.is_none(), "this controller sent {sent:?}");
}

// CLU-015 CLU-023 CLU-024 CLU-027 CLU-065
#[then(regex = r#"^the cluster hub should carry a pulse from this controller holding node (\d) of cluster "([^"]*)"$"#)]
async fn hub_carries_holding_pulse(world: &mut DaliWorld, node: u8, name: String) {
    let rig = rig(world);
    let mac = rig.mac;
    let deadline = Instant::now() + HUB_DEADLINE;
    let found = std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now())))
        .any(|sent| holds_node(&sent, mac, node, &name));
    assert!(found, "no pulse holding node {node} of {name:?} within {HUB_DEADLINE:?}");
}

// CLU-018 CLU-024
#[then(regex = r"^the cluster hub should carry no pulse from this controller holding node (\d) within (\d+) ms$")]
async fn hub_carries_no_holding_pulse(world: &mut DaliWorld, node: u8, window_ms: u64) {
    let rig = rig(world);
    let mac = rig.mac;
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let pulses: Vec<HubDatagram> =
        std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now()))).collect();
    assert!(!pulses.is_empty(), "a waiting node still pulses; none arrived to judge");
    let holding = pulses.iter().find(|sent| decode(&sent.bytes).is_ok_and(|decoded| {
        decoded.header.sender_mac == mac
            && decoded.header.node_no == node
            && matches!(decoded.frame, Frame::Pulse(pulse) if pulse.claim == NodeClaim::Holding)
    }));
    assert!(holding.is_none(), "a waiting node claimed its number: {holding:?}");
}

// CLU-022
#[then(regex = r#"^the WebSocket client should receive a cluster status frame showing node (\d) heard from board "([^"]+)"$"#)]
async fn ws_cluster_status(world: &mut DaliWorld, node: u64, board: String) {
    let client = world.ws_clients.last_mut().expect("an open WebSocket connection");
    let frame = client.wait_for(|frame| {
        let row = &frame["payload"]["nodes"][0];
        frame["type"] == "ClusterStatus"
            && frame["channel"] == "cluster"
            && row["node"] == node
            && row["board"] == board.as_str()
            && row["heard"] == Value::Bool(true)
    });
    assert!(frame.is_some(), "no ClusterStatus frame showed node {node} heard from {board}");
}

// CLU-046 CLU-052 CLU-065 CLU-084
#[when(regex = r#"^I link "([^"]+)" to the cluster record "([^"]+)" with send (true|false) and accept (true|false)$"#)]
async fn link_part_to_record(world: &mut DaliWorld, part: String, record: String, send: String, accept: String) {
    world.send_http_request("GET", &record, None, "");
    let held = last_json(world);
    let reference = json!({ "number": held["number"], "birth_mark": held["birth_mark"] });
    let body = json!({ "record": reference, "send": send == "true", "accept": accept == "true" }).to_string();
    world.send_http_request("PUT", &part, Some(body.as_bytes()), "application/json");
}
