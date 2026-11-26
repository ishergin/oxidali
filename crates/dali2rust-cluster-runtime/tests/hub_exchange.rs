use std::time::Duration;

use dali2rust_cluster_runtime::frame::{
    cluster_tag, decode, encode, write_pages, Action, BirthMark, Command, DatagramBuffer,
    Decoded, Frame, Header, Hello, InfoElement, LightVerb, NodeClaim, Pulse, Record, RecordRef,
    SourceClass, Target,
};
use dali2rust_cluster_runtime::{Arrival, SequenceWindow};
use dali2rust_contracts::msg::LevelTransition;
use dali2rust_platform::cluster_link::{
    ClusterLink, ClusterReceiveBuffer, Received, CLUSTER_RECEIVE_BUFFER_BYTES,
    MAX_CLUSTER_DATAGRAM_BYTES,
};
use dali2rust_test_support::{ClusterHub, HubEnd};

const PATIENCE: Duration = Duration::from_secs(5);
const CLUSTER: &str = "home";
const NODE_0_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x10];
const NODE_1_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x11];
const NEIGHBOUR: [u8; 4] = [192, 168, 1, 77];
const NEIGHBOUR_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x12];
const EPOCH: u32 = 0x00c0_ffee;
const CHANNEL_3: RecordRef = RecordRef { number: 3, mark: BirthMark { generation: 1, nonce: 9 } };

fn header(mac: [u8; 6], node_no: u8, seq: u32) -> Header {
    Header { cluster_tag: cluster_tag(CLUSTER), sender_mac: mac, boot_epoch: EPOCH, node_no, seq }
}

fn datagram(header: &Header, frame: &Frame<'_>) -> Vec<u8> {
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(header, frame, &mut buffer).expect("encode");
    buffer[..len].to_vec()
}

fn received<'b>(end: &HubEnd, buffer: &'b mut ClusterReceiveBuffer) -> Decoded<'b> {
    match end.receive(buffer, PATIENCE).expect("receive") {
        Received::Datagram { len, .. } => decode(&buffer[..len]).expect("decode"),
        other => panic!("expected a datagram, got {other:?}"),
    }
}

fn step_up() -> Command {
    Command {
        action: Action::Light {
            target: Target::Channel(CHANNEL_3),
            verb: LightVerb::Transition(LevelTransition::Up),
        },
        source: SourceClass::Operator,
        hold_hcl: false,
        executed_on_wire: Some(0),
    }
}

#[test]
fn a_command_sent_twice_crosses_the_hub_and_its_copy_is_a_duplicate() {
    let hub = ClusterHub::new();
    let (node_0, node_1) = (hub.attach(), hub.attach());
    let bytes = datagram(&header(NODE_0_MAC, 0, 1), &Frame::Command(step_up()));
    node_0.send(&bytes).expect("first copy");
    node_0.send(&bytes).expect("second copy");
    let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
    let first = received(&node_1, &mut buffer);
    assert_eq!(first.frame, Frame::Command(step_up()));
    assert_eq!(first.header.sender_mac, NODE_0_MAC);
    let mut window = SequenceWindow::starting_at(first.header.boot_epoch, first.header.seq);
    let second = received(&node_1, &mut buffer);
    assert_eq!(window.admit(second.header.boot_epoch, second.header.seq), Arrival::Duplicate);
}

#[test]
fn pulses_cross_the_hub_both_ways_and_carry_what_decides_a_wire_owner() {
    let hub = ClusterHub::new();
    let (node_0, node_1) = (hub.attach(), hub.attach());
    for (from, to, mac, node_no) in [(&node_0, &node_1, NODE_0_MAC, 0), (&node_1, &node_0, NODE_1_MAC, 1)] {
        let pulse = Pulse {
            cluster_name: CLUSTER,
            installation: "dali-000010",
            claim: NodeClaim::Holding,
            wire: 0,
            wire_protection: true,
        };
        from.send(&datagram(&header(mac, node_no, 1), &Frame::Pulse(pulse))).expect("send");
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let got = received(to, &mut buffer);
        assert_eq!((got.header.sender_mac, got.header.node_no), (mac, node_no));
        assert_eq!(got.frame, Frame::Pulse(pulse));
    }
}

#[test]
fn a_neighbour_played_by_the_test_reaches_every_node_in_frames_of_the_production_codec() {
    let hub = ClusterHub::new();
    let (node_0, node_1) = (hub.attach(), hub.attach());
    let hello = Frame::Hello(Hello { ha_face: false });
    hub.inject(NEIGHBOUR, &datagram(&header(NEIGHBOUR_MAC, 2, 1), &hello));
    for node in [&node_0, &node_1] {
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let got = received(node, &mut buffer);
        assert_eq!((got.header.sender_mac, &got.frame), (NEIGHBOUR_MAC, &hello));
    }
}

#[test]
fn a_node_s_info_pages_reach_its_peer_and_the_test_sees_them_leave() {
    let hub = ClusterHub::new();
    let (node_0, node_1) = (hub.attach(), hub.attach());
    let record = InfoElement::ChannelRecord(Record { id: CHANNEL_3, name: "Hall", revision: 1 });
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let next_header = || header(NODE_0_MAC, 0, 2);
    write_pages(&mut buffer, 1, next_header, [record], |page| node_0.send(page).expect("send"))
        .expect("pages");
    let sent = hub.next_sent(PATIENCE).expect("the page left node 0");
    assert_eq!(sent.source, node_0.source());
    let mut inbox = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
    let Frame::ChannelSceneInfo(page) = received(&node_1, &mut inbox).frame else {
        panic!("not a page")
    };
    assert!(page.meta().last);
    assert_eq!(page.elements().collect::<Vec<_>>(), [record]);
}
