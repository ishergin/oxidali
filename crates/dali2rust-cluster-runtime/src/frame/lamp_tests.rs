use dali2rust_contracts::msg::{ColorMode, DeviceType, PowerState, RuntimeSource};
use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use serde::Serialize;

use super::*;

const LAMP_STATE_KIND: u8 = 5;
const LAMP_INFO_KIND: u8 = 6;
const SET: u32 = 9;
const ADAPTER: AdapterScope = AdapterScope { adapter: 0 };
const MASK: u8 = 255;
const LAST_LAMP: u8 = VIRTUAL_LAMP_COUNT - 1;
const MARK: BirthMark = BirthMark { generation: 2, nonce: 0xa1b2_c3d4 };
const CHANNEL_3: RecordRef = RecordRef { number: 3, mark: MARK };
const WIDEST_RECORD: RecordRef =
    RecordRef { number: RECORD_NUMBER_COUNT - 1, mark: BirthMark { generation: u32::MAX, nonce: u32::MAX } };

const GOLDEN_HEADER: [u8; 17] = [
    0x8e, 0x85, 0xa3, 0x96, 0x0d, 0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4, 0x84, 0x86, 0x88, 0x08, 0x01, 0x07,
];
const GOLDEN_STATE_BODY: [u8; 35] = [
    0x01, 0x00, 0x09, 0x00, 0x00, 0x06, 0x01, 0x01, 0xc8, 0x02, 0x01, 0x8c, 0x15, 0x00, 0x00, 0x00, 0x01, 0xe8,
    0x07, 0x01, 0x02, 0x00, 0x01, 0x05, 0x03, 0x02, 0xd4, 0x87, 0xcb, 0x8d, 0x0a, 0x01, 0x02, 0x03, 0x04,
];
const GOLDEN_INFO_BODY: [u8; 40] = [
    0x01, 0x00, 0x09, 0x00, 0x00, 0x04, 0x63, 0x74, 0x6c, 0x31, 0x02, 0x68, 0x61, 0x01, 0x06, 0x04, 0x48, 0x61,
    0x6c, 0x6c, 0x09, 0x00, 0x82, 0x02, 0x02, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x8c, 0x15,
    0xe4, 0x32, 0x01, 0x01,
];
const OLDEST_SHOWN_AGE_MS: u32 = 180_000;

fn header() -> Header {
    Header { cluster_tag: cluster_tag("home"), sender_mac: [0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4], boot_epoch: 0x0102_0304, node_no: 1, seq: 7 }
}

fn widest_header() -> Header {
    Header { cluster_tag: u32::MAX, sender_mac: [u8::MAX; 6], boot_epoch: u32::MAX, node_no: NODE_COUNT - 1, seq: u32::MAX }
}

fn warm_lamp() -> LampState {
    LampState {
        lamp: 6,
        power: PowerState::On,
        level: Some(200),
        colour: LampColour { mode: ColorMode::Cct, kelvin: Some(2700), ..LampColour::default() },
        age_ms: Some(1000),
        source: Some(RuntimeSource::Api),
        absent: false,
    }
}

fn part() -> PartState {
    PartState { group: 5, channel: CHANNEL_3, commanded: true, lit: 2, known: 3, members: 4 }
}

fn unknown_lamp(lamp: u8) -> LampState {
    LampState {
        lamp,
        power: PowerState::Unknown,
        level: None,
        colour: LampColour { mode: ColorMode::Unknown, ..LampColour::default() },
        age_ms: None,
        source: None,
        absent: false,
    }
}

fn widest_lamp(lamp: u8) -> LampState {
    LampState {
        lamp,
        power: PowerState::Unknown,
        level: Some(MAX_ARC),
        colour: LampColour {
            mode: ColorMode::Unknown,
            kelvin: Some(u16::MAX),
            xy: Some([u16::MAX; 2]),
            rgb: Some([u8::MAX; 3]),
            waf: Some([u8::MAX; 3]),
        },
        age_ms: Some(u32::MAX),
        source: Some(RuntimeSource::ClusterHcl),
        absent: true,
    }
}

const MAX_ARC: u8 = 254;

fn brightness_lamp(lamp: u8) -> LampState {
    LampState {
        lamp,
        power: PowerState::On,
        level: Some(MAX_ARC),
        colour: LampColour { mode: ColorMode::Brightness, ..LampColour::default() },
        age_ms: Some(OLDEST_SHOWN_AGE_MS),
        source: Some(RuntimeSource::Mqtt),
        absent: false,
    }
}

fn hall_info(name: &str) -> LampInfo<'_> {
    LampInfo {
        lamp: 6,
        name,
        device_type: DeviceType::Dt8Color,
        device_type_overridden: false,
        device_types: 0x0102,
        color_mode: ColorMode::Cct,
        color_mode_overridden: false,
        capabilities: LampCapabilities { brightness: true, cct: true, scenes: true, groups: true, ..LampCapabilities::default() },
        kelvin_range: Some(KelvinRange { min: 2700, max: 6500 }),
        bound: true,
        ha_entity: true,
    }
}

fn widest_info(name: &str) -> LampInfo<'_> {
    let all = LampCapabilities { brightness: true, cct: true, xy: true, rgb: true, rgbwaf: true, scenes: true, groups: true };
    LampInfo {
        lamp: LAST_LAMP,
        name,
        device_type: DeviceType::Other,
        device_type_overridden: true,
        device_types: u64::MAX,
        color_mode: ColorMode::Unknown,
        color_mode_overridden: true,
        capabilities: all,
        kelvin_range: Some(KelvinRange { min: u16::MAX, max: u16::MAX }),
        bound: true,
        ha_entity: true,
    }
}

fn golden(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = PROTOCOL_TAG.to_vec();
    bytes.extend_from_slice(&[PROTOCOL_VERSION, kind]);
    bytes.extend_from_slice(&GOLDEN_HEADER);
    bytes.extend_from_slice(body);
    bytes
}

fn state_page(header: &Header, elements: &[StateElement], last: bool) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: ADAPTER, set: SET, page: 0 };
    let mut writer = LampStateWriter::open(&mut buffer, header, place).expect("open");
    for element in elements {
        writer.push(element).expect("push");
    }
    let len = writer.finish(last);
    buffer[..len].to_vec()
}

fn info_page(header: &Header, elements: &[LampInfoElement<'_>]) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: ADAPTER, set: SET, page: 0 };
    let mut writer = LampInfoWriter::open(&mut buffer, header, place).expect("open");
    for element in elements {
        writer.push(element).expect("push");
    }
    let len = writer.finish(true);
    buffer[..len].to_vec()
}

fn states_of(bytes: &[u8]) -> Vec<StateElement> {
    let Frame::LampState(page) = decode(bytes).expect("decode").frame else { panic!("not a state page") };
    page.elements().collect()
}

fn wire<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).expect("encodes")
}

fn element_bytes<T: Serialize>(value: &T) -> usize {
    wire(value).len()
}

fn pages_of(elements: &[StateElement]) -> Vec<Vec<u8>> {
    let mut cursor = PageCursor::new(SET);
    let mut pages = Vec::new();
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    while let Some(len) = cursor.write_next_of::<LampStateKind>(&mut buffer, &widest_header(), ADAPTER, elements.iter().copied()).expect("page") {
        pages.push(buffer[..len].to_vec());
    }
    pages
}

#[test]
fn a_page_of_lamp_and_part_states_has_its_golden_bytes() {
    let bytes = state_page(&header(), &[StateElement::Lamp(warm_lamp()), StateElement::Part(part())], true);
    assert_eq!(bytes, golden(LAMP_STATE_KIND, &GOLDEN_STATE_BODY), "{:02x?}", &bytes[PREAMBLE_BYTES + GOLDEN_HEADER.len()..]);
}

#[test]
fn a_page_of_lamp_info_has_its_golden_bytes() {
    let owner = LampInfoElement::Owner(HaIdentity { controller_id: "ctl1", discovery_prefix: "ha" });
    let bytes = info_page(&header(), &[owner, LampInfoElement::Lamp(hall_info("Hall"))]);
    assert_eq!(bytes, golden(LAMP_INFO_KIND, &GOLDEN_INFO_BODY), "{:02x?}", &bytes[PREAMBLE_BYTES + GOLDEN_HEADER.len()..]);
}

#[test]
fn every_variant_of_the_lamp_frames_keeps_its_index_on_the_wire() {
    assert_eq!(wire(&StateElement::Lamp(warm_lamp()))[0], 0);
    assert_eq!(wire(&StateElement::Part(part()))[0], 1);
    let owner = HaIdentity { controller_id: "", discovery_prefix: "" };
    assert_eq!(wire(&LampInfoElement::Owner(owner))[0], 0);
    assert_eq!(wire(&LampInfoElement::Lamp(hall_info("")))[0], 1);
    assert_eq!(FrameKind::LampState as u8, LAMP_STATE_KIND);
    assert_eq!(FrameKind::LampInfo as u8, LAMP_INFO_KIND);
}

#[test]
fn an_unknown_level_power_colour_and_age_stay_unknown_across_the_wire() {
    let bytes = state_page(&header(), &[StateElement::Lamp(unknown_lamp(4))], true);
    assert_eq!(states_of(&bytes), [StateElement::Lamp(unknown_lamp(4))]);
    let off = LampState { power: PowerState::Off, level: Some(0), ..unknown_lamp(4) };
    let bytes = state_page(&header(), &[StateElement::Lamp(off)], true);
    assert_eq!(states_of(&bytes), [StateElement::Lamp(off)], "a level of 0 stays a level");
}

#[test]
fn a_state_page_reads_back_its_adapter_set_and_place() {
    let bytes = state_page(&header(), &[StateElement::Part(part())], false);
    let Frame::LampState(page) = decode(&bytes).expect("decode").frame else { panic!("not a state page") };
    assert_eq!(page.scope(), ADAPTER);
    assert_eq!(page.meta(), PageMeta { last: false, page: 0, set: SET });
}

#[test]
fn a_mask_level_a_lamp_past_the_last_and_impossible_counts_are_refused_and_rejected() {
    let refused = [
        StateElement::Lamp(LampState { level: Some(MASK), ..warm_lamp() }),
        StateElement::Lamp(LampState { lamp: VIRTUAL_LAMP_COUNT, ..warm_lamp() }),
        StateElement::Part(PartState { lit: 4, known: 3, ..part() }),
        StateElement::Part(PartState { known: 5, ..part() }),
        StateElement::Part(PartState { members: VIRTUAL_LAMP_COUNT + 1, known: 65, lit: 65, ..part() }),
        StateElement::Part(PartState { group: GROUP_COUNT, ..part() }),
        StateElement::Part(PartState { channel: RecordRef { number: RECORD_NUMBER_COUNT, mark: MARK }, ..part() }),
    ];
    for element in refused {
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let place = PagePlace { scope: ADAPTER, set: SET, page: 0 };
        let mut writer = LampStateWriter::open(&mut buffer, &header(), place).expect("open");
        assert_eq!(writer.push(&element), Err(PushError::OutOfRange), "{element:?}");
        let mut bytes = state_page(&header(), &[], true);
        bytes.extend_from_slice(&wire(&element));
        assert_eq!(decode(&bytes), Err(Rejection::Malformed), "{element:?}");
    }
}

#[test]
fn an_adapter_past_the_last_is_refused_on_open_and_rejected_on_decode() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: AdapterScope { adapter: ADAPTER_COUNT }, set: SET, page: 0 };
    assert!(LampStateWriter::open(&mut buffer, &header(), place).is_err());
    let mut bytes = state_page(&header(), &[], true);
    let scope_at = bytes.len() - 1;
    bytes[scope_at] = ADAPTER_COUNT;
    assert_eq!(decode(&bytes), Err(Rejection::Malformed));
}

#[test]
fn an_overlong_name_owner_or_an_inverted_range_is_refused_and_rejected() {
    let long_name = "x".repeat(MAX_NAME_BYTES + 1);
    let long_owner = "x".repeat(MAX_INSTALLATION_BYTES + 1);
    let long_prefix = "x".repeat(MAX_DISCOVERY_PREFIX_BYTES + 1);
    let refused = [
        LampInfoElement::Lamp(hall_info(&long_name)),
        LampInfoElement::Lamp(LampInfo { kelvin_range: Some(KelvinRange { min: 6500, max: 2700 }), ..hall_info("Hall") }),
        LampInfoElement::Lamp(LampInfo { lamp: VIRTUAL_LAMP_COUNT, ..hall_info("Hall") }),
        LampInfoElement::Owner(HaIdentity { controller_id: &long_owner, discovery_prefix: "ha" }),
        LampInfoElement::Owner(HaIdentity { controller_id: "ctl1", discovery_prefix: &long_prefix }),
    ];
    for element in refused {
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let place = PagePlace { scope: ADAPTER, set: SET, page: 0 };
        let mut writer = LampInfoWriter::open(&mut buffer, &header(), place).expect("open");
        assert_eq!(writer.push(&element), Err(PushError::OutOfRange));
        let mut bytes = info_page(&header(), &[]);
        bytes.extend_from_slice(&wire(&element));
        assert_eq!(decode(&bytes), Err(Rejection::Malformed));
    }
}

#[test]
fn sixty_four_brightness_lamps_and_sixteen_parts_go_in_one_datagram_after_the_widest_header() {
    let lamps = (0..VIRTUAL_LAMP_COUNT).map(|lamp| StateElement::Lamp(brightness_lamp(lamp)));
    let last_record = RecordRef { number: RECORD_NUMBER_COUNT - 1, mark: MARK };
    let parts = (0..GROUP_COUNT).map(|group| StateElement::Part(PartState { group, channel: last_record, ..part() }));
    let elements: Vec<_> = lamps.chain(parts).collect();
    let pages = pages_of(&elements);
    eprintln!(
        "lamp state: brightness {} B, widest {} B, unknown {} B; part {} B; 64 lamps + 16 parts: {} B",
        element_bytes(&StateElement::Lamp(brightness_lamp(LAST_LAMP))),
        element_bytes(&StateElement::Lamp(widest_lamp(LAST_LAMP))),
        element_bytes(&StateElement::Lamp(unknown_lamp(LAST_LAMP))),
        element_bytes(&StateElement::Part(PartState { channel: WIDEST_RECORD, ..part() })),
        pages.first().map_or(0, Vec::len),
    );
    assert_eq!(pages.len(), 1);
    assert_eq!(states_of(&pages[0]), elements);
}

#[test]
fn sixty_four_widest_lamps_page_in_order_and_every_page_fits_the_ceiling() {
    let elements: Vec<_> = (0..VIRTUAL_LAMP_COUNT).map(|lamp| StateElement::Lamp(widest_lamp(lamp))).collect();
    let pages = pages_of(&elements);
    assert!(pages.len() > 1);
    assert!(pages.iter().all(|page| page.len() <= MAX_CLUSTER_DATAGRAM_BYTES));
    let read: Vec<_> = pages.iter().flat_map(|page| states_of(page)).collect();
    assert_eq!(read, elements);
}

#[test]
fn the_widest_lamp_info_fits_a_page_and_reads_back() {
    let name = "\u{1}".repeat(MAX_NAME_BYTES);
    let owner = "x".repeat(MAX_INSTALLATION_BYTES);
    let prefix = "x".repeat(MAX_DISCOVERY_PREFIX_BYTES);
    let elements = [
        LampInfoElement::Owner(HaIdentity { controller_id: &owner, discovery_prefix: &prefix }),
        LampInfoElement::Lamp(widest_info(&name)),
    ];
    eprintln!(
        "lamp info: widest {} B, typical {} B, owner {} B",
        element_bytes(&elements[1]),
        element_bytes(&LampInfoElement::Lamp(hall_info("Corridor lamp 12"))),
        element_bytes(&elements[0]),
    );
    let bytes = info_page(&widest_header(), &elements);
    let Frame::LampInfo(page) = decode(&bytes).expect("decode").frame else { panic!("not an info page") };
    assert_eq!(page.elements().collect::<Vec<_>>(), elements);
}
