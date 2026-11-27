use std::net::Ipv4Addr;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use dali2rust_adapters::ClusterComposition;
use dali2rust_cluster_runtime::frame::{
    cluster_tag, decode, encode, DatagramBuffer, Frame, Header, NodeClaim, Pulse,
};
use dali2rust_cluster_runtime::presence::PresenceTiming;
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_platform::net::node_name;
use dali2rust_test_support::{ClusterHub, ClusterHubBinder, HubDatagram};
use serde_json::Value;

use crate::DaliWorld;

const PULSE_PERIOD_MS: u32 = 200;
const OWN_BOOT_EPOCH: u32 = 0x0C1A_0001;
const NEIGHBOUR_BOOT_EPOCH: u32 = 0x0C1A_0002;
const HUB_DEADLINE: Duration = Duration::from_secs(4);

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
            boot_epoch: OWN_BOOT_EPOCH,
        }
    }

    fn sent_by_us(&self, window: Duration) -> Option<HubDatagram> {
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
        let mac = pulser.mac;
        let thread = thread::spawn(move || pulser.run(&stopped));
        self.neighbours.push(Neighbour { mac, stop: Some(stop), thread: Some(thread) });
    }
}

struct Neighbour {
    mac: [u8; 6],
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
    hub: Arc<ClusterHub>,
    source: [u8; 4],
    mac: [u8; 6],
    node: u8,
    cluster_name: String,
    installation: String,
}

impl Pulser {
    fn run(self, stop: &Receiver<()>) {
        let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
        let period = Duration::from_millis(u64::from(PULSE_PERIOD_MS));
        let mut seq = 0_u32;
        loop {
            self.send(&mut buffer, seq);
            seq = seq.wrapping_add(1);
            if stop.recv_timeout(period) != Err(RecvTimeoutError::Timeout) {
                return;
            }
        }
    }

    fn send(&self, buffer: &mut DatagramBuffer, seq: u32) {
        let header = Header {
            cluster_tag: cluster_tag(&self.cluster_name),
            sender_mac: self.mac,
            boot_epoch: NEIGHBOUR_BOOT_EPOCH,
            node_no: self.node,
            seq,
        };
        let pulse = Pulse {
            cluster_name: &self.cluster_name,
            installation: &self.installation,
            claim: NodeClaim::Holding,
            wire: self.node,
            wire_protection: true,
        };
        let len = encode(&header, &Frame::Pulse(pulse), buffer).expect("a pulse fits one datagram");
        self.hub.inject(self.source, buffer.get(..len).expect("the encoded length"));
    }
}

fn rig(world: &mut DaliWorld) -> &mut ClusterRig {
    world.cluster.as_mut().expect("the controller runs on a cluster hub")
}

fn hardware_address(text: &str) -> [u8; 6] {
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

// CLU-015 CLU-016 CLU-017 CLU-018 CLU-019 CLU-020 CLU-021 CLU-022 CLU-023
#[given(regex = r#"^the controller runs on a cluster hub as board "([0-9a-f:]{17})" at "([0-9.]+)"$"#)]
async fn on_cluster_hub(world: &mut DaliWorld, mac: String, address: String) {
    let (mac, address) = (hardware_address(&mac), ipv4(&address));
    world.cluster = Some(ClusterRig { hub: Arc::new(ClusterHub::new()), mac, address, neighbours: Vec::new() });
    world.run_on_network_link(mac, Some(address), node_name(mac));
}

// CLU-016 CLU-017 CLU-018 CLU-020 CLU-022 CLU-023
#[when(regex = r#"^a board "([0-9a-f:]{17})" at "([0-9.]+)" pulses as node (\d) of cluster "([^"]*)" for installation "([^"]*)"$"#)]
async fn board_pulses(world: &mut DaliWorld, mac: String, source: String, node: u8, name: String, site: String) {
    let rig = rig(world);
    let hub = Arc::clone(&rig.hub);
    let (mac, source) = (hardware_address(&mac), ipv4(&source));
    rig.start(Pulser { hub, source, mac, node, cluster_name: name, installation: site });
}

// CLU-019
#[when(regex = r#"^a board at "([0-9.]+)" pulses this controller's hardware address as node (\d) of cluster "([^"]*)"$"#)]
async fn board_pulses_our_address(world: &mut DaliWorld, source: String, node: u8, name: String) {
    let rig = rig(world);
    let (hub, mac, source) = (Arc::clone(&rig.hub), rig.mac, ipv4(&source));
    rig.start(Pulser { hub, source, mac, node, cluster_name: name, installation: String::new() });
}

// CLU-020
#[when(regex = r#"^the board "([0-9a-f:]{17})" stops pulsing$"#)]
async fn board_stops(world: &mut DaliWorld, mac: String) {
    let mac = hardware_address(&mac);
    let rig = rig(world);
    let before = rig.neighbours.len();
    rig.neighbours.retain(|neighbour| neighbour.mac != mac);
    assert!(rig.neighbours.len() < before, "no board {mac:02x?} was pulsing");
}

// CLU-023
#[when("the cluster hub sets aside what this controller has sent so far")]
async fn hub_sets_aside(world: &mut DaliWorld) {
    let hub = &rig(world).hub;
    while hub.next_sent(Duration::ZERO).is_some() {}
}

// CLU-015 CLU-023
#[then(regex = r"^the cluster hub should carry no datagram from this controller within (\d+) ms$")]
async fn hub_silent(world: &mut DaliWorld, window_ms: u64) {
    let sent = rig(world).sent_by_us(Duration::from_millis(window_ms));
    assert!(sent.is_none(), "this controller sent {sent:?}");
}

// CLU-015 CLU-023
#[then(regex = r#"^the cluster hub should carry a pulse from this controller holding node (\d) of cluster "([^"]*)"$"#)]
async fn hub_carries_holding_pulse(world: &mut DaliWorld, node: u8, name: String) {
    let rig = rig(world);
    let mac = rig.mac;
    let deadline = Instant::now() + HUB_DEADLINE;
    let found = std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now())))
        .any(|sent| holds_node(&sent, mac, node, &name));
    assert!(found, "no pulse holding node {node} of {name:?} within {HUB_DEADLINE:?}");
}

// CLU-018
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
