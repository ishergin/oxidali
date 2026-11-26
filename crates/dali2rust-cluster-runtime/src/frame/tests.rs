use dali2rust_contracts::msg::{ColorMode, ColorValue, LevelTransition, LightSetpoint, PowerState};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;

const GOLDEN_HEADER: [u8; 17] = [
    0x8e, 0x85, 0xa3, 0x96, 0x0d, 0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4, 0x84, 0x86, 0x88, 0x08,
    0x01, 0x07,
];
const GOLDEN_COMMAND_BODY: [u8; 29] = [
    0x00, 0x00, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x00, 0x01, 0x01, 0xc8, 0x01, 0x02,
    0x8c, 0x15, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01,
];
const GOLDEN_PULSE_BODY: [u8; 20] = [
    0x04, 0x68, 0x6f, 0x6d, 0x65, 0x0b, 0x64, 0x61, 0x6c, 0x69, 0x2d, 0x37, 0x62, 0x39, 0x30,
    0x65, 0x34, 0x00, 0x01, 0x01,
];
const GOLDEN_HELLO_BODY: [u8; 1] = [0x01];
const GOLDEN_INFO_BODY: [u8; 77] = [
    0x01, 0x00, 0x09, 0x00, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x04, 0x48, 0x61, 0x6c,
    0x6c, 0x04, 0x01, 0x11, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x03, 0x45, 0x76, 0x65, 0x01,
    0x02, 0x00, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x05, 0x00, 0x01, 0x01, 0x02, 0x0c,
    0x04, 0x03, 0x00, 0x11, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x02, 0x00, 0x01, 0x03, 0x04,
    0x00, 0x21, 0x05, 0x00, 0x05, 0x04, 0x48, 0x61, 0x6c, 0x6c, 0x06, 0x00, 0x02, 0x03, 0x45,
    0x76, 0x65,
];
const INFO_SET: u32 = 9;
const SAMPLE_PART_GROUP_AT: usize = 39;

const HOME_MAC: [u8; 6] = [0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4];
const MARK: BirthMark = BirthMark { generation: 2, nonce: 0xa1b2_c3d4 };
const WIDEST_MARK: BirthMark = BirthMark { generation: u32::MAX, nonce: u32::MAX };
const CHANNEL_3: RecordRef = RecordRef { number: 3, mark: MARK };
const LAST_RECORD: u8 = RECORD_NUMBER_COUNT - 1;
const WIDEST_RECORD: RecordRef = RecordRef { number: LAST_RECORD, mark: WIDEST_MARK };
const LAST_NODE: u8 = NODE_COUNT - 1;

fn header() -> Header {
    Header {
        cluster_tag: cluster_tag("home"),
        sender_mac: HOME_MAC,
        boot_epoch: 0x0102_0304,
        node_no: 1,
        seq: 7,
    }
}

fn widest_header() -> Header {
    Header {
        cluster_tag: u32::MAX,
        sender_mac: [u8::MAX; 6],
        boot_epoch: u32::MAX,
        node_no: LAST_NODE,
        seq: u32::MAX,
    }
}

fn warm_white() -> LightSetpoint {
    LightSetpoint {
        power: PowerState::On,
        level: Some(200),
        color: Some(ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 2700,
            ..ColorValue::default()
        }),
    }
}

fn widest_setpoint() -> LightSetpoint {
    LightSetpoint {
        power: PowerState::Unknown,
        level: Some(u8::MAX),
        color: Some(ColorValue {
            mode: ColorMode::Unknown,
            color_temperature_kelvin: u16::MAX,
            x: u16::MAX,
            y: u16::MAX,
            r: u8::MAX,
            g: u8::MAX,
            b: u8::MAX,
            w: u8::MAX,
            a: u8::MAX,
            f: u8::MAX,
        }),
    }
}

fn channel_command() -> Command {
    Command {
        action: Action::Light {
            target: Target::Channel(CHANNEL_3),
            verb: LightVerb::Setpoint(warm_white()),
        },
        source: SourceClass::Operator,
        hold_hcl: true,
        executed_on_wire: Some(1),
    }
}

fn widest_command() -> Command {
    Command {
        action: Action::Light {
            target: Target::Channel(WIDEST_RECORD),
            verb: LightVerb::Setpoint(widest_setpoint()),
        },
        source: SourceClass::ForeignMaster,
        hold_hcl: true,
        executed_on_wire: Some(LAST_NODE),
    }
}

fn pulse() -> Pulse<'static> {
    Pulse {
        cluster_name: "home",
        installation: "dali-7b90e4",
        claim: NodeClaim::Holding,
        wire: 1,
        wire_protection: true,
    }
}

fn encoded(header: &Header, frame: &Frame<'_>) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(header, frame, &mut buffer).expect("encode");
    buffer[..len].to_vec()
}

fn round_trip(frame: &Frame<'_>) {
    let bytes = encoded(&header(), frame);
    let decoded = decode(&bytes).expect("decode");
    assert_eq!(decoded.header, header());
    assert_eq!(&decoded.frame, frame);
}

fn info_sample() -> [InfoElement<'static>; 7] {
    [
        InfoElement::ChannelRecord(Record { id: CHANNEL_3, name: "Hall", revision: 4 }),
        InfoElement::SceneRecord(Record {
            id: RecordRef { number: 17, mark: MARK },
            name: "Eve",
            revision: 1,
        }),
        InfoElement::ChannelPart(ChannelPart {
            adapter: 0,
            channel: CHANNEL_3,
            group: 5,
            taken: false,
            flags: PartFlags { send: true, accept: true },
            revision: 2,
            desired_lamps: 0b1100,
            written_lamps: 0b0100,
        }),
        InfoElement::ScenePart(ScenePart {
            adapter: 0,
            scene: RecordRef { number: 17, mark: MARK },
            node_scene: 2,
            flags: PartFlags { send: false, accept: true },
            revision: 3,
        }),
        InfoElement::GroupMap(GroupMap { adapter: 0, occupied: 0b10_0001 }),
        InfoElement::GroupName(LocalName { adapter: 0, number: 5, name: "Hall" }),
        InfoElement::SceneName(LocalName { adapter: 0, number: 2, name: "Eve" }),
    ]
}

fn golden(kind: FrameKind, body: &[u8]) -> Vec<u8> {
    let mut bytes = PROTOCOL_TAG.to_vec();
    bytes.extend_from_slice(&[PROTOCOL_VERSION, kind as u8]);
    bytes.extend_from_slice(&GOLDEN_HEADER);
    bytes.extend_from_slice(body);
    bytes
}

fn sample_page(buffer: &mut DatagramBuffer, last: bool) -> usize {
    let mut writer = InfoPageWriter::start(buffer, &header(), INFO_SET, 0).expect("start");
    for element in info_sample() {
        writer.push(&element).expect("push");
    }
    writer.finish(last)
}

fn widest_elements(name: &str) -> [InfoElement<'_>; 7] {
    let flags = PartFlags { send: true, accept: true };
    [
        InfoElement::ChannelRecord(Record { id: WIDEST_RECORD, name, revision: u32::MAX }),
        InfoElement::SceneRecord(Record { id: WIDEST_RECORD, name, revision: u32::MAX }),
        InfoElement::ChannelPart(ChannelPart {
            adapter: u8::MAX,
            channel: WIDEST_RECORD,
            group: 15,
            taken: true,
            flags,
            revision: u32::MAX,
            desired_lamps: u64::MAX,
            written_lamps: u64::MAX,
        }),
        InfoElement::ScenePart(ScenePart {
            adapter: u8::MAX,
            scene: WIDEST_RECORD,
            node_scene: 15,
            flags,
            revision: u32::MAX,
        }),
        InfoElement::GroupMap(GroupMap { adapter: u8::MAX, occupied: u16::MAX }),
        InfoElement::GroupName(LocalName { adapter: u8::MAX, number: 15, name }),
        InfoElement::SceneName(LocalName { adapter: u8::MAX, number: 15, name }),
    ]
}

fn widest_name() -> String {
    let widest = char::MAX;
    widest.to_string().repeat(MAX_NAME_BYTES / widest.len_utf8())
}

#[test]
fn a_channel_command_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Command(channel_command()));
    assert_eq!(bytes, golden(FrameKind::Command, &GOLDEN_COMMAND_BODY));
}

#[test]
fn a_pulse_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Pulse(pulse()));
    assert_eq!(bytes, golden(FrameKind::Pulse, &GOLDEN_PULSE_BODY));
}

#[test]
fn a_hello_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    assert_eq!(bytes, golden(FrameKind::Hello, &GOLDEN_HELLO_BODY));
}

#[test]
fn a_page_of_channel_and_scene_info_has_its_golden_bytes() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    assert_eq!(buffer[..len], golden(FrameKind::ChannelSceneInfo, &GOLDEN_INFO_BODY));
}

#[test]
fn the_preamble_names_the_protocol_its_version_and_the_kind() {
    let bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: false }));
    assert_eq!(bytes[..PREAMBLE_BYTES], [b'D', b'2', b'R', b'C', 1, FrameKind::Hello as u8]);
}

#[test]
fn every_frame_kind_decodes_to_what_was_encoded() {
    round_trip(&Frame::Command(channel_command()));
    round_trip(&Frame::Pulse(pulse()));
    round_trip(&Frame::Hello(Hello { ha_face: true }));
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    let page = decode(&buffer[..len]).expect("decode").frame;
    round_trip(&page);
}

#[test]
fn every_verb_and_target_survives_the_round_trip() {
    let lamp = Target::Lamp(LampAddress { node: LAST_NODE, adapter: 1, lamp: 63 });
    let verbs = [
        LightVerb::Setpoint(warm_white()),
        LightVerb::Transition(LevelTransition::Up),
        LightVerb::Transition(LevelTransition::StepDownAndOff),
        LightVerb::StopFade,
    ];
    for target in [Target::Channel(CHANNEL_3), Target::All, lamp] {
        for verb in verbs.clone() {
            let action = Action::Light { target, verb };
            round_trip(&Frame::Command(Command { action, ..channel_command() }));
        }
    }
}

#[test]
fn a_scene_recall_reaches_a_whole_node_or_a_channel() {
    let scene = RecordRef { number: 17, mark: MARK };
    for scope in [RecallScope::Node, RecallScope::Channel(CHANNEL_3)] {
        let action = Action::RecallScene { scene, scope };
        let command = Command { action, executed_on_wire: None, ..channel_command() };
        round_trip(&Frame::Command(command));
    }
}

#[test]
fn the_widest_single_frames_stay_under_the_datagram_ceiling() {
    let name = widest_name();
    let installation = "x".repeat(MAX_INSTALLATION_BYTES);
    let widest_pulse = Pulse { cluster_name: &name, installation: &installation, ..pulse() };
    for frame in [
        Frame::Command(widest_command()),
        Frame::Pulse(widest_pulse),
        Frame::Hello(Hello { ha_face: true }),
    ] {
        let bytes = encoded(&widest_header(), &frame);
        assert!(bytes.len() <= MAX_CLUSTER_DATAGRAM_BYTES);
        assert_eq!(decode(&bytes).expect("decode").frame, frame);
    }
}

#[test]
fn the_widest_element_of_every_kind_fits_a_page_after_the_widest_header() {
    let name = widest_name();
    for element in widest_elements(&name) {
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let mut writer =
            InfoPageWriter::start(&mut buffer, &widest_header(), u32::MAX, u8::MAX).expect("start");
        assert_eq!(writer.push(&element), Ok(()));
        let len = writer.finish(true);
        let page = decode(&buffer[..len]).expect("decode");
        let Frame::ChannelSceneInfo(page) = page.frame else { panic!("not a page") };
        assert_eq!(page.elements().collect::<Vec<_>>(), [element]);
    }
}

#[test]
fn a_node_s_whole_info_splits_into_pages_under_the_ceiling_and_reads_back_in_order() {
    let name = widest_name();
    let elements: Vec<_> = (0..NUMBERS_PER_NODE).flat_map(|_| widest_elements(&name)).collect();
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut seq = 0;
    let next_header = || {
        seq += 1;
        Header { seq, ..widest_header() }
    };
    write_pages(&mut buffer, INFO_SET, next_header, elements.clone(), |page| pages.push(page.to_vec()))
        .expect("pages");
    assert!(pages.len() > 1);
    let mut read_back = Vec::new();
    for (index, bytes) in pages.iter().enumerate() {
        assert!(bytes.len() <= MAX_CLUSTER_DATAGRAM_BYTES);
        let decoded = decode(bytes).expect("decode");
        assert_eq!(decoded.header.seq, u32::try_from(index + 1).expect("seq"));
        let Frame::ChannelSceneInfo(page) = decoded.frame else { panic!("not a page") };
        let meta = page.meta();
        assert_eq!((meta.set, usize::from(meta.page), meta.last), (INFO_SET, index, index + 1 == pages.len()));
        read_back.extend(page.elements());
    }
    assert_eq!(read_back, elements);
}

#[test]
fn a_set_with_nothing_in_it_is_one_empty_last_page() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut pages = Vec::new();
    write_pages(&mut buffer, INFO_SET, header, [], |page| pages.push(page.to_vec())).expect("pages");
    assert_eq!(pages.len(), 1);
    let Frame::ChannelSceneInfo(page) = decode(&pages[0]).expect("decode").frame else {
        panic!("not a page")
    };
    assert_eq!(page.meta(), PageMeta { last: true, page: 0, set: INFO_SET });
    assert_eq!(page.elements().count(), 0);
}

#[test]
fn a_page_that_is_not_the_last_says_so() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, false);
    let Frame::ChannelSceneInfo(page) = decode(&buffer[..len]).expect("decode").frame else {
        panic!("not a page")
    };
    assert!(!page.meta().last);
    assert_eq!(page.elements().count(), info_sample().len());
}

#[test]
fn a_full_page_refuses_the_next_element_and_keeps_what_it_holds() {
    let name = widest_name();
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut writer = InfoPageWriter::start(&mut buffer, &header(), INFO_SET, 0).expect("start");
    let record = widest_elements(&name)[0];
    let mut held = 0;
    while writer.push(&record).is_ok() {
        held += 1;
    }
    assert_eq!(writer.push(&record), Err(PushError::Full));
    let len = writer.finish(true);
    let Frame::ChannelSceneInfo(page) = decode(&buffer[..len]).expect("decode").frame else {
        panic!("not a page")
    };
    assert_eq!(page.elements().count(), held);
}

#[test]
fn a_datagram_of_another_protocol_is_not_a_cluster_frame() {
    assert_eq!(decode(b""), Err(Rejection::NotCluster));
    assert_eq!(decode(b"D2R"), Err(Rejection::NotCluster));
    let mut bytes = encoded(&header(), &Frame::Pulse(pulse()));
    bytes[0] = b'X';
    assert_eq!(decode(&bytes), Err(Rejection::NotCluster));
}

#[test]
fn an_unknown_version_is_its_own_rejection_and_names_the_version() {
    let mut bytes = encoded(&header(), &Frame::Pulse(pulse()));
    bytes[PROTOCOL_TAG.len()] = PROTOCOL_VERSION + 1;
    assert_eq!(decode(&bytes), Err(Rejection::UnknownVersion(PROTOCOL_VERSION + 1)));
    bytes.truncate(PREAMBLE_BYTES);
    assert_eq!(decode(&bytes), Err(Rejection::UnknownVersion(PROTOCOL_VERSION + 1)));
}

#[test]
fn a_datagram_past_the_ceiling_is_rejected_before_it_is_read() {
    let mut bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    bytes.resize(MAX_CLUSTER_DATAGRAM_BYTES + 1, 0);
    assert_eq!(decode(&bytes), Err(Rejection::TooLarge));
}

#[test]
fn an_unknown_kind_under_a_known_version_is_malformed() {
    let mut bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    bytes[PREAMBLE_BYTES - 1] = FrameKind::ChannelSceneInfo as u8 + 1;
    assert_eq!(decode(&bytes), Err(Rejection::Malformed));
}

#[test]
fn a_cut_short_or_padded_body_is_malformed() {
    let bytes = encoded(&header(), &Frame::Command(channel_command()));
    assert_eq!(decode(&bytes[..bytes.len() - 1]), Err(Rejection::Malformed));
    let mut padded = bytes.clone();
    padded.push(0);
    assert_eq!(decode(&padded), Err(Rejection::Malformed));
    assert_eq!(decode(&bytes[..PREAMBLE_BYTES + 3]), Err(Rejection::Malformed));
}

#[test]
fn a_page_with_a_broken_element_is_rejected_whole() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    assert_eq!(decode(&buffer[..len - 1]), Err(Rejection::Malformed));
    let mut bytes = buffer[..len].to_vec();
    bytes.push(u8::MAX);
    assert_eq!(decode(&bytes), Err(Rejection::Malformed));
}

#[test]
fn a_value_out_of_its_range_is_refused_on_encode_and_rejected_on_decode() {
    let past_last_node = Header { node_no: NODE_COUNT, ..header() };
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let refused = encode(&past_last_node, &Frame::Hello(Hello { ha_face: true }), &mut buffer);
    assert_eq!(refused, Err(EncodeError::OutOfRange));
    let mut bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    let node_at = PREAMBLE_BYTES + GOLDEN_HEADER.len() - 2;
    bytes[node_at] = NODE_COUNT;
    assert_eq!(decode(&bytes), Err(Rejection::Malformed));
}

#[test]
fn a_command_outside_the_cluster_s_numbers_is_refused() {
    let past_last_record = RecordRef { number: RECORD_NUMBER_COUNT, mark: MARK };
    let past_last_lamp = LampAddress { node: 0, adapter: 0, lamp: 64 };
    let past_last_node = LampAddress { node: NODE_COUNT, adapter: 0, lamp: 0 };
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    for action in [
        Action::Light { target: Target::Channel(past_last_record), verb: LightVerb::StopFade },
        Action::Light { target: Target::Lamp(past_last_lamp), verb: LightVerb::StopFade },
        Action::Light { target: Target::Lamp(past_last_node), verb: LightVerb::StopFade },
        Action::RecallScene { scene: past_last_record, scope: RecallScope::Node },
        Action::RecallScene { scene: CHANNEL_3, scope: RecallScope::Channel(past_last_record) },
    ] {
        let frame = Frame::Command(Command { action, ..channel_command() });
        assert_eq!(encode(&header(), &frame, &mut buffer), Err(EncodeError::OutOfRange));
    }
    let wire_past_last = Command { executed_on_wire: Some(NODE_COUNT), ..channel_command() };
    let refused = encode(&header(), &Frame::Command(wire_past_last), &mut buffer);
    assert_eq!(refused, Err(EncodeError::OutOfRange));
}

#[test]
fn a_pulse_with_an_overlong_name_or_a_wire_past_the_last_node_is_refused() {
    let long_name = "x".repeat(MAX_CLUSTER_NAME_BYTES + 1);
    let long_installation = "x".repeat(MAX_INSTALLATION_BYTES + 1);
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    for pulse in [
        Pulse { cluster_name: &long_name, ..pulse() },
        Pulse { installation: &long_installation, ..pulse() },
        Pulse { wire: NODE_COUNT, ..pulse() },
    ] {
        assert_eq!(encode(&header(), &Frame::Pulse(pulse), &mut buffer), Err(EncodeError::OutOfRange));
    }
}

#[test]
fn an_element_outside_its_range_is_refused_by_the_page() {
    let long_name = "x".repeat(MAX_NAME_BYTES + 1);
    let [record, _, channel_part, scene_part, _, group_name, scene_name] = info_sample();
    let InfoElement::ChannelRecord(record) = record else { panic!("record") };
    let InfoElement::ChannelPart(channel_part) = channel_part else { panic!("part") };
    let InfoElement::ScenePart(scene_part) = scene_part else { panic!("part") };
    let InfoElement::GroupName(group_name) = group_name else { panic!("name") };
    let InfoElement::SceneName(scene_name) = scene_name else { panic!("name") };
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut writer = InfoPageWriter::start(&mut buffer, &header(), INFO_SET, 0).expect("start");
    for element in [
        InfoElement::ChannelRecord(Record { name: &long_name, ..record }),
        InfoElement::SceneRecord(Record { id: RecordRef { number: RECORD_NUMBER_COUNT, mark: MARK }, ..record }),
        InfoElement::ChannelPart(ChannelPart { group: 16, ..channel_part }),
        InfoElement::ScenePart(ScenePart { node_scene: 16, ..scene_part }),
        InfoElement::GroupName(LocalName { number: 16, ..group_name }),
        InfoElement::SceneName(LocalName { name: &long_name, ..scene_name }),
    ] {
        assert_eq!(writer.push(&element), Err(PushError::OutOfRange));
    }
}

#[test]
fn an_element_out_of_range_inside_a_received_page_rejects_the_page() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    let mut bytes = buffer[..len].to_vec();
    let group_at = bytes.len() - GOLDEN_INFO_BODY.len() + SAMPLE_PART_GROUP_AT;
    assert_eq!(bytes[group_at], 5);
    bytes[group_at] = 16;
    assert_eq!(decode(&bytes), Err(Rejection::Malformed));
}

#[test]
fn a_command_is_byte_identical_when_sent_twice() {
    let first = encoded(&header(), &Frame::Command(channel_command()));
    let second = encoded(&header(), &Frame::Command(channel_command()));
    assert_eq!(first, second);
}
