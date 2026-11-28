use dali2rust_contracts::msg::{ColorMode, ColorValue, LevelTransition, LightSetpoint, PowerState};
use dali2rust_domain::registry::{GROUP_COUNT, SCENE_COUNT, VIRTUAL_LAMP_COUNT};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use serde::Serialize;

use super::codec::{put, write_head};
use super::*;

const COMMAND_KIND: u8 = 0;
const PULSE_KIND: u8 = 1;
const HELLO_KIND: u8 = 2;
const INFO_KIND: u8 = 3;
const FIRST_UNKNOWN_KIND: u8 = 9;

const GOLDEN_HEADER: [u8; 17] = [
    0x8e, 0x85, 0xa3, 0x96, 0x0d, 0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4, 0x84, 0x86, 0x88, 0x08,
    0x01, 0x07,
];
const GOLDEN_COMMAND_BODY: [u8; 29] = [
    0x00, 0x00, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x00, 0x01, 0x01, 0xc8, 0x01, 0x02,
    0x8c, 0x15, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x03,
];
const GOLDEN_PULSE_BODY: [u8; 20] = [
    0x04, 0x68, 0x6f, 0x6d, 0x65, 0x0b, 0x64, 0x61, 0x6c, 0x69, 0x2d, 0x37, 0x62, 0x39, 0x30,
    0x65, 0x34, 0x00, 0x03, 0x01,
];
const GOLDEN_HELLO_BODY: [u8; 1] = [0x01];
const GOLDEN_INFO_BODY: [u8; 77] = [
    0x01, 0x00, 0x09, 0x00, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x04, 0x48, 0x61, 0x6c,
    0x6c, 0x04, 0x01, 0x11, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x03, 0x45, 0x76, 0x65, 0x01,
    0x02, 0x01, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x05, 0x00, 0x01, 0x00, 0x02, 0x0c,
    0x04, 0x03, 0x01, 0x11, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x02, 0x00, 0x01, 0x03, 0x04,
    0x01, 0x21, 0x05, 0x01, 0x05, 0x04, 0x48, 0x61, 0x6c, 0x6c, 0x06, 0x01, 0x02, 0x03, 0x45,
    0x76, 0x65,
];
const GOLDEN_EVERY_COLOUR_FIELD: [u8; 17] = [
    0x01, 0x01, 0xc8, 0x01, 0x05, 0x8c, 0x15, 0xe8, 0x07, 0xd0, 0x0f, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06,
];
const GOLDEN_LAMP_STEP: [u8; 7] = [0x00, 0x02, 0x01, 0x02, 0x03, 0x01, 0x08];
const GOLDEN_CHANNEL_RECALL: [u8; 8] = [0x01, 0x11, 0x02, 0x03, 0x01, 0x04, 0x05, 0x06];
const INFO_SET: u32 = 9;
const SAMPLE_GROUP: u8 = 5;
const SAMPLE_ADAPTER: u8 = 1;
const SAMPLE_WIRE: u8 = 3;
const SAMPLE_PART_ADAPTER_AT: usize = 31;
const SAMPLE_PART_GROUP_AT: usize = 39;
const LAST_LAMP: u8 = VIRTUAL_LAMP_COUNT - 1;
const LAST_GROUP: u8 = GROUP_COUNT - 1;
const LAST_SCENE: u8 = SCENE_COUNT - 1;

const HOME_MAC: [u8; 6] = [0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4];
const MARK: BirthMark = BirthMark { generation: 2, nonce: 0xa1b2_c3d4 };
const WIDEST_MARK: BirthMark = BirthMark { generation: u32::MAX, nonce: u32::MAX };
const CHANNEL_3: RecordRef = RecordRef { number: 3, mark: MARK };
const LAST_RECORD: u8 = RECORD_NUMBER_COUNT - 1;
const WIDEST_RECORD: RecordRef = RecordRef { number: LAST_RECORD, mark: WIDEST_MARK };
const LAST_NODE: u8 = NODE_COUNT - 1;
const LAST_ADAPTER: u8 = ADAPTER_COUNT - 1;

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
        executed_on_wire: Some(SAMPLE_WIRE),
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

fn wire_note() -> WireNote {
    WireNote { wire: 1, adapter: 0, reach: NoteReach::Group(5), deed: NoteDeed::Light(LightVerb::Transition(LevelTransition::Up)) }
}

fn pulse() -> Pulse<'static> {
    Pulse {
        cluster_name: "home",
        installation: "dali-7b90e4",
        claim: NodeClaim::Holding,
        wire: SAMPLE_WIRE,
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
            adapter: SAMPLE_ADAPTER,
            channel: CHANNEL_3,
            group: SAMPLE_GROUP,
            taken: false,
            flags: PartFlags { send: true, accept: false },
            revision: 2,
            desired_lamps: 0b1100,
            written_lamps: 0b0100,
        }),
        InfoElement::ScenePart(ScenePart {
            adapter: SAMPLE_ADAPTER,
            scene: RecordRef { number: 17, mark: MARK },
            node_scene: 2,
            flags: PartFlags { send: false, accept: true },
            revision: 3,
        }),
        InfoElement::GroupMap(GroupMap { adapter: SAMPLE_ADAPTER, occupied: 0b10_0001 }),
        InfoElement::GroupName(LocalName {
            adapter: SAMPLE_ADAPTER,
            number: SAMPLE_GROUP,
            name: "Hall",
        }),
        InfoElement::SceneName(LocalName { adapter: SAMPLE_ADAPTER, number: 2, name: "Eve" }),
    ]
}

fn golden(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = PROTOCOL_TAG.to_vec();
    bytes.extend_from_slice(&[PROTOCOL_VERSION, kind]);
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
            adapter: LAST_ADAPTER,
            channel: WIDEST_RECORD,
            group: LAST_GROUP,
            taken: true,
            flags,
            revision: u32::MAX,
            desired_lamps: u64::MAX,
            written_lamps: u64::MAX,
        }),
        InfoElement::ScenePart(ScenePart {
            adapter: LAST_ADAPTER,
            scene: WIDEST_RECORD,
            node_scene: LAST_SCENE,
            flags,
            revision: u32::MAX,
        }),
        InfoElement::GroupMap(GroupMap { adapter: LAST_ADAPTER, occupied: u16::MAX }),
        InfoElement::GroupName(LocalName { adapter: LAST_ADAPTER, number: LAST_GROUP, name }),
        InfoElement::SceneName(LocalName { adapter: LAST_ADAPTER, number: LAST_SCENE, name }),
    ]
}

fn widest_name() -> String {
    let widest = char::MAX;
    widest.to_string().repeat(MAX_NAME_BYTES / widest.len_utf8())
}

fn wire<T: Serialize>(value: &T) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = put(value, &mut buffer).expect("put");
    buffer[..len].to_vec()
}

fn assert_wire_order<T: Serialize>(variants: &[T]) {
    for (index, variant) in variants.iter().enumerate() {
        assert_eq!(usize::from(wire(variant)[0]), index);
    }
}

fn unchecked(kind: FrameKind, body: &impl Serialize) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let at = write_head(&mut buffer, kind, &header()).expect("head");
    let len = at + put(body, &mut buffer[at..]).expect("body");
    buffer[..len].to_vec()
}

fn written_pages(elements: &[InfoElement<'_>]) -> Vec<Vec<u8>> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut pages = Vec::new();
    let mut cursor = PageCursor::new(INFO_SET);
    let mut seq = 0;
    while !cursor.finished() {
        seq += 1;
        let header = Header { seq, ..widest_header() };
        let len = cursor.write_next(&mut buffer, &header, elements.iter().copied()).expect("page");
        pages.push(buffer[..len.expect("a page while unfinished")].to_vec());
    }
    pages
}

fn page_at(bytes: &[u8], index: usize) -> InfoPage<'_> {
    let decoded = decode(bytes).expect("decode");
    assert_eq!(decoded.header.seq, u32::try_from(index + 1).expect("seq"));
    let Frame::ChannelSceneInfo(page) = decoded.frame else { panic!("not a page") };
    assert_eq!((page.meta().set, usize::from(page.meta().page)), (INFO_SET, index));
    page
}

#[test]
fn a_channel_command_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Command(channel_command()));
    assert_eq!(bytes, golden(COMMAND_KIND, &GOLDEN_COMMAND_BODY));
}

#[test]
fn a_pulse_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Pulse(pulse()));
    assert_eq!(bytes, golden(PULSE_KIND, &GOLDEN_PULSE_BODY));
}

#[test]
fn a_hello_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    assert_eq!(bytes, golden(HELLO_KIND, &GOLDEN_HELLO_BODY));
}

#[test]
fn a_page_of_channel_and_scene_info_has_its_golden_bytes() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    assert_eq!(buffer[..len], golden(INFO_KIND, &GOLDEN_INFO_BODY));
}

#[test]
fn the_preamble_names_the_protocol_its_version_and_the_kind() {
    let bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: false }));
    assert_eq!(bytes[..PREAMBLE_BYTES], [b'D', b'2', b'R', b'C', 1, HELLO_KIND]);
}

#[test]
fn every_field_of_a_command_keeps_its_place_on_the_wire() {
    let every_colour_field = LightSetpoint {
        power: PowerState::On,
        level: Some(200),
        color: Some(ColorValue {
            mode: ColorMode::Rgbwaf,
            color_temperature_kelvin: 2700,
            x: 1000,
            y: 2000,
            r: 1,
            g: 2,
            b: 3,
            w: 4,
            a: 5,
            f: 6,
        }),
    };
    assert_eq!(wire(&every_colour_field), GOLDEN_EVERY_COLOUR_FIELD);
    let lamp = Target::Lamp(LampAddress { node: 1, adapter: 2, lamp: 3 });
    let step = Action::Light { target: lamp, verb: LightVerb::Transition(LevelTransition::Down) };
    assert_eq!(wire(&step), GOLDEN_LAMP_STEP);
    let record =
        |number, generation, nonce| RecordRef { number, mark: BirthMark { generation, nonce } };
    let scope = RecallScope::Channel(record(4, 5, 6));
    let recall = Action::RecallScene { scene: record(17, 2, 3), scope };
    assert_eq!(wire(&recall), GOLDEN_CHANNEL_RECALL);
}

#[test]
fn every_variant_of_a_command_keeps_its_index_on_the_wire() {
    let lamp = LampAddress { node: 0, adapter: 0, lamp: 0 };
    assert_wire_order(&[Target::Channel(CHANNEL_3), Target::All, Target::Lamp(lamp)]);
    assert_wire_order(&[RecallScope::Node, RecallScope::Channel(CHANNEL_3)]);
    assert_wire_order(&[
        LightVerb::Setpoint(warm_white()),
        LightVerb::Transition(LevelTransition::Up),
        LightVerb::StopFade,
    ]);
    assert_wire_order(&[
        Action::Light { target: Target::All, verb: LightVerb::StopFade },
        Action::RecallScene { scene: CHANNEL_3, scope: RecallScope::Node },
    ]);
    assert_wire_order(&[
        SourceClass::Operator,
        SourceClass::HomeAssistant,
        SourceClass::Hcl,
        SourceClass::Rules,
        SourceClass::ForeignMaster,
    ]);
}

#[test]
fn every_variant_of_a_reused_contract_type_keeps_its_index_on_the_wire() {
    assert_wire_order(&[
        LevelTransition::RecallMaxLevel,
        LevelTransition::RecallMinLevel,
        LevelTransition::StepUp,
        LevelTransition::StepDown,
        LevelTransition::StepDownAndOff,
        LevelTransition::OnAndStepUp,
        LevelTransition::GoToLastActiveLevel,
        LevelTransition::Up,
        LevelTransition::Down,
    ]);
    assert_wire_order(&[PowerState::Off, PowerState::On, PowerState::Unknown]);
    assert_wire_order(&[
        ColorMode::None,
        ColorMode::Brightness,
        ColorMode::Cct,
        ColorMode::Xy,
        ColorMode::Rgb,
        ColorMode::Rgbwaf,
        ColorMode::Unknown,
    ]);
}

#[test]
fn every_variant_of_presence_and_info_keeps_its_index_on_the_wire() {
    assert_wire_order(&[NodeClaim::Holding, NodeClaim::Waiting]);
    assert_wire_order(&info_sample());
}

#[test]
fn every_frame_kind_decodes_to_what_was_encoded() {
    round_trip(&Frame::Command(channel_command()));
    round_trip(&Frame::Pulse(pulse()));
    round_trip(&Frame::Hello(Hello { ha_face: true }));
    round_trip(&Frame::WireNote(wire_note()));
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    let page = decode(&buffer[..len]).expect("decode").frame;
    round_trip(&page);
}

#[test]
fn every_verb_and_target_survives_the_round_trip() {
    let lamp = Target::Lamp(LampAddress { node: LAST_NODE, adapter: 1, lamp: LAST_LAMP });
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
fn a_node_s_whole_info_fills_every_page_but_the_last_and_reads_back_in_order() {
    let name = widest_name();
    let elements: Vec<_> = (0..NUMBERS_PER_NODE).flat_map(|_| widest_elements(&name)).collect();
    let pages = written_pages(&elements);
    assert!(pages.len() > 1);
    let mut read_back = Vec::new();
    for (index, bytes) in pages.iter().enumerate() {
        let page = page_at(bytes, index);
        let last = index + 1 == pages.len();
        assert_eq!(page.meta().last, last);
        read_back.extend(page.elements());
        if !last {
            let next = wire(&elements[read_back.len()]).len();
            assert!(bytes.len() + next > MAX_CLUSTER_DATAGRAM_BYTES);
        }
    }
    assert_eq!(read_back, elements);
}

#[test]
fn a_set_with_nothing_in_it_is_one_empty_last_page() {
    let pages = written_pages(&[]);
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
    bytes[PROTOCOL_TAG.len()] = PROTOCOL_VERSION + 1;
    assert_eq!(decode(&bytes), Err(Rejection::NotCluster));
}

#[test]
fn an_unknown_version_is_its_own_rejection_and_names_the_version() {
    let mut bytes = encoded(&header(), &Frame::Pulse(pulse()));
    bytes[PROTOCOL_TAG.len()] = PROTOCOL_VERSION + 1;
    assert_eq!(decode(&bytes), Err(Rejection::UnknownVersion(PROTOCOL_VERSION + 1)));
    bytes[PROTOCOL_TAG.len()] = u8::MAX;
    bytes[PREAMBLE_BYTES - 1] = FIRST_UNKNOWN_KIND;
    assert_eq!(decode(&bytes), Err(Rejection::UnknownVersion(u8::MAX)));
    bytes.truncate(PREAMBLE_BYTES);
    assert_eq!(decode(&bytes), Err(Rejection::UnknownVersion(u8::MAX)));
}

#[test]
fn a_datagram_past_the_ceiling_is_rejected_before_it_is_read() {
    let mut bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    bytes.resize(MAX_CLUSTER_DATAGRAM_BYTES + 1, 0);
    assert_eq!(decode(&bytes), Err(Rejection::TooLarge));
}

#[test]
fn an_unknown_kind_under_a_known_version_is_its_own_rejection_and_names_the_kind() {
    let mut bytes = encoded(&header(), &Frame::Hello(Hello { ha_face: true }));
    for kind in [FIRST_UNKNOWN_KIND, u8::MAX] {
        bytes[PREAMBLE_BYTES - 1] = kind;
        assert_eq!(decode(&bytes), Err(Rejection::UnknownKind(kind)));
    }
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
fn a_command_outside_the_cluster_s_numbers_is_refused_and_rejected() {
    let past_last_record = RecordRef { number: RECORD_NUMBER_COUNT, mark: MARK };
    let past_last_lamp = LampAddress { node: 0, adapter: 0, lamp: VIRTUAL_LAMP_COUNT };
    let past_last_node = LampAddress { node: NODE_COUNT, adapter: 0, lamp: 0 };
    let past_last_adapter = LampAddress { node: 0, adapter: ADAPTER_COUNT, lamp: 0 };
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    assert!(decode(&unchecked(FrameKind::Command, &channel_command())).is_ok());
    for command in [
        Action::Light { target: Target::Channel(past_last_record), verb: LightVerb::StopFade },
        Action::Light { target: Target::Lamp(past_last_lamp), verb: LightVerb::StopFade },
        Action::Light { target: Target::Lamp(past_last_node), verb: LightVerb::StopFade },
        Action::Light { target: Target::Lamp(past_last_adapter), verb: LightVerb::StopFade },
        Action::RecallScene { scene: past_last_record, scope: RecallScope::Node },
        Action::RecallScene { scene: CHANNEL_3, scope: RecallScope::Channel(past_last_record) },
    ]
    .map(|action| Command { action, ..channel_command() })
    .into_iter()
    .chain([Command { executed_on_wire: Some(NODE_COUNT), ..channel_command() }])
    {
        let refused = encode(&header(), &Frame::Command(command.clone()), &mut buffer);
        assert_eq!(refused, Err(EncodeError::OutOfRange));
        assert_eq!(decode(&unchecked(FrameKind::Command, &command)), Err(Rejection::Malformed));
    }
}

#[test]
fn a_pulse_with_an_overlong_name_or_a_wire_past_the_last_node_is_refused_and_rejected() {
    let long_name = "x".repeat(MAX_CLUSTER_NAME_BYTES + 1);
    let long_installation = "x".repeat(MAX_INSTALLATION_BYTES + 1);
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    assert!(decode(&unchecked(FrameKind::Pulse, &pulse())).is_ok());
    for pulse in [
        Pulse { cluster_name: &long_name, ..pulse() },
        Pulse { installation: &long_installation, ..pulse() },
        Pulse { wire: NODE_COUNT, ..pulse() },
    ] {
        let refused = encode(&header(), &Frame::Pulse(pulse), &mut buffer);
        assert_eq!(refused, Err(EncodeError::OutOfRange));
        assert_eq!(decode(&unchecked(FrameKind::Pulse, &pulse)), Err(Rejection::Malformed));
    }
}

#[test]
fn an_element_outside_its_range_is_refused_by_the_page() {
    let long_name = "x".repeat(MAX_NAME_BYTES + 1);
    let [record, _, channel_part, scene_part, group_map, group_name, scene_name] = info_sample();
    let InfoElement::ChannelRecord(record) = record else { panic!("record") };
    let InfoElement::ChannelPart(channel_part) = channel_part else { panic!("part") };
    let InfoElement::ScenePart(scene_part) = scene_part else { panic!("part") };
    let InfoElement::GroupMap(group_map) = group_map else { panic!("map") };
    let InfoElement::GroupName(group_name) = group_name else { panic!("name") };
    let InfoElement::SceneName(scene_name) = scene_name else { panic!("name") };
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut writer = InfoPageWriter::start(&mut buffer, &header(), INFO_SET, 0).expect("start");
    let past_last_record = RecordRef { number: RECORD_NUMBER_COUNT, mark: MARK };
    for element in [
        InfoElement::ChannelRecord(Record { name: &long_name, ..record }),
        InfoElement::SceneRecord(Record { id: past_last_record, ..record }),
        InfoElement::ChannelPart(ChannelPart { group: GROUP_COUNT, ..channel_part }),
        InfoElement::ChannelPart(ChannelPart { channel: past_last_record, ..channel_part }),
        InfoElement::ScenePart(ScenePart { node_scene: SCENE_COUNT, ..scene_part }),
        InfoElement::ScenePart(ScenePart { scene: past_last_record, ..scene_part }),
        InfoElement::GroupName(LocalName { number: GROUP_COUNT, ..group_name }),
        InfoElement::GroupName(LocalName { name: &long_name, ..group_name }),
        InfoElement::SceneName(LocalName { name: &long_name, ..scene_name }),
        InfoElement::SceneName(LocalName { number: SCENE_COUNT, ..scene_name }),
        InfoElement::ChannelPart(ChannelPart { adapter: ADAPTER_COUNT, ..channel_part }),
        InfoElement::ScenePart(ScenePart { adapter: ADAPTER_COUNT, ..scene_part }),
        InfoElement::GroupMap(GroupMap { adapter: ADAPTER_COUNT, ..group_map }),
        InfoElement::GroupName(LocalName { adapter: ADAPTER_COUNT, ..group_name }),
        InfoElement::SceneName(LocalName { adapter: ADAPTER_COUNT, ..scene_name }),
    ] {
        assert_eq!(writer.push(&element), Err(PushError::OutOfRange));
    }
}

#[test]
fn an_element_out_of_range_inside_a_received_page_rejects_the_page() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer, true);
    let body_at = len - GOLDEN_INFO_BODY.len();
    for (at, sample, past_last) in [
        (SAMPLE_PART_GROUP_AT, SAMPLE_GROUP, GROUP_COUNT),
        (SAMPLE_PART_ADAPTER_AT, SAMPLE_ADAPTER, ADAPTER_COUNT),
    ] {
        let mut bytes = buffer[..len].to_vec();
        assert_eq!(bytes[body_at + at], sample);
        bytes[body_at + at] = past_last;
        assert_eq!(decode(&bytes), Err(Rejection::Malformed));
    }
}

#[test]
fn every_variant_of_a_wire_note_keeps_its_index_and_a_note_out_of_range_is_refused() {
    assert_wire_order(&[NoteReach::Group(0), NoteReach::Broadcast]);
    assert_wire_order(&[NoteDeed::Light(LightVerb::StopFade), NoteDeed::Scene(0)]);
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let out_of_range = [
        WireNote { wire: NODE_COUNT, ..wire_note() },
        WireNote { adapter: ADAPTER_COUNT, ..wire_note() },
        WireNote { reach: NoteReach::Group(GROUP_COUNT), ..wire_note() },
        WireNote { deed: NoteDeed::Scene(SCENE_COUNT), ..wire_note() },
    ];
    for note in out_of_range {
        assert_eq!(encode(&header(), &Frame::WireNote(note.clone()), &mut buffer), Err(EncodeError::OutOfRange), "{note:?}");
        assert_eq!(decode(&unchecked(FrameKind::WireNote, &note)), Err(Rejection::Malformed), "{note:?}");
    }
}

