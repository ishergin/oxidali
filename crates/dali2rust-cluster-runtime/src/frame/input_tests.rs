use dali2rust_contracts::msg::InputEventKind;
use dali2rust_domain::registry::{CLUSTER_INPUT_DEVICES, CLUSTER_INPUT_INSTANCES, MAX_SHORT_ADDRESSES};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use serde::Serialize;

use super::codec::{put, write_head};
use super::*;

const INPUT_EVENT_KIND: u8 = 7;
const INPUT_INFO_KIND: u8 = 8;
const SET: u32 = 9;
const LAST_DEVICE: u8 = MAX_SHORT_ADDRESSES as u8 - 1;
const LAST_INSTANCE: u8 = CLUSTER_INPUT_INSTANCES as u8 - 1;
const LAST_TYPE: u8 = 31;
const LAST_GROUP: u8 = 31;
const PRESS: u16 = 0x001;

const GOLDEN_HEADER: [u8; 17] = [
    0x8e, 0x85, 0xa3, 0x96, 0x0d, 0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4, 0x84, 0x86, 0x88, 0x08, 0x01, 0x07,
];
const GOLDEN_EVENT_BODY: [u8; 15] = [
    0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x04, 0x00, 0x01, 0x09, 0x02, 0x01, 0x01, 0x01,
];
const GOLDEN_INFO_BODY: [u8; 49] = [
    0x01, 0x00, 0x09, 0x00, 0x04, 0x63, 0x74, 0x6c, 0x31, 0x02, 0x68, 0x61, 0x01, 0x00, 0x03, 0x04, 0x48, 0x61,
    0x6c, 0x6c, 0x01, 0x01, 0x02, 0x02, 0x00, 0x03, 0x00, 0x01, 0x01, 0x01, 0x04, 0x00, 0x00, 0x01, 0x01, 0x00,
    0xdc, 0x0b, 0x02, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x04, 0x00, 0x00, 0x00,
];

fn header() -> Header {
    Header { cluster_tag: cluster_tag("home"), sender_mac: [0x30, 0xed, 0xa0, 0x7b, 0x90, 0xe4], boot_epoch: 0x0102_0304, node_no: 1, seq: 7 }
}

fn widest_header() -> Header {
    Header { cluster_tag: u32::MAX, sender_mac: [u8::MAX; 6], boot_epoch: u32::MAX, node_no: NODE_COUNT - 1, seq: u32::MAX }
}

fn press() -> InputEvent {
    InputEvent {
        address: InputAddress { adapter: 0, device: 3, instance: Some(1) },
        instance_type: Some(1),
        groups: [Some(4), None, Some(9)],
        scheme: 2,
        kind: InputEventKind::Button,
        event_info: PRESS,
        value: PRESS,
    }
}

fn widest_event() -> InputEvent {
    InputEvent {
        address: InputAddress { adapter: ADAPTER_COUNT - 1, device: LAST_DEVICE, instance: Some(LAST_INSTANCE) },
        instance_type: Some(LAST_TYPE),
        groups: [Some(LAST_GROUP); 3],
        scheme: MAX_INPUT_SCHEME,
        kind: InputEventKind::Illuminance,
        event_info: INPUT_FIELD_MASK,
        value: INPUT_FIELD_MASK,
    }
}

fn owner() -> InputInfoElement<'static> {
    InputInfoElement::Owner(InputOwner { controller_id: "ctl1", discovery_prefix: "ha" })
}

fn panel(name: &str) -> InputInfoElement<'_> {
    InputInfoElement::Device(InputDevice { adapter: 0, device: 3, name, ha_entity: true, present: true, instances: 2 })
}

fn button(instance: u8, last: Option<InputValue>) -> InputInfoElement<'static> {
    InputInfoElement::Instance(InputInstance {
        adapter: 0,
        device: 3,
        instance,
        instance_type: Some(1),
        groups: [Some(4), None, None],
        last,
    })
}

fn info_sample() -> [InputInfoElement<'static>; 4] {
    let pressed = InputValue { event_info: PRESS, value: None, age_ms: 1500 };
    [owner(), panel("Hall"), button(0, Some(pressed)), button(1, None)]
}

fn widest_device(device: u8, name: &str) -> InputInfoElement<'_> {
    InputInfoElement::Device(InputDevice {
        adapter: ADAPTER_COUNT - 1,
        device,
        name,
        ha_entity: true,
        present: true,
        instances: CLUSTER_INPUT_INSTANCES as u8,
    })
}

fn widest_instance(device: u8, instance: u8) -> InputInfoElement<'static> {
    InputInfoElement::Instance(InputInstance {
        adapter: ADAPTER_COUNT - 1,
        device,
        instance,
        instance_type: Some(LAST_TYPE),
        groups: [Some(LAST_GROUP); 3],
        last: Some(InputValue { event_info: INPUT_FIELD_MASK, value: Some(INPUT_FIELD_MASK), age_ms: u32::MAX }),
    })
}

fn widest_name() -> String {
    "\u{10FFFF}".repeat(MAX_NAME_BYTES / 4)
}

fn golden(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = PROTOCOL_TAG.to_vec();
    bytes.extend_from_slice(&[PROTOCOL_VERSION, kind]);
    bytes.extend_from_slice(&GOLDEN_HEADER);
    bytes.extend_from_slice(body);
    bytes
}

fn encoded(header: &Header, frame: &Frame<'_>) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(header, frame, &mut buffer).expect("encodes");
    buffer[..len].to_vec()
}

fn wire<T: Serialize>(value: &T) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = put(value, &mut buffer).expect("fits");
    buffer[..len].to_vec()
}

fn unchecked(kind: FrameKind, body: &impl Serialize) -> Vec<u8> {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let at = write_head(&mut buffer, kind, &header()).expect("head");
    let len = at + put(body, &mut buffer[at..]).expect("body");
    buffer[..len].to_vec()
}

fn sample_page(buffer: &mut DatagramBuffer) -> usize {
    let mut writer = InputInfoWriter::open(buffer, &header(), PagePlace { scope: (), set: SET, page: 0 }).expect("open");
    for element in info_sample() {
        writer.push(&element).expect("fits");
    }
    writer.finish(true)
}

fn written_pages(elements: &[InputInfoElement<'_>]) -> Vec<Vec<u8>> {
    let mut cursor = PageCursor::new(SET);
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut pages = Vec::new();
    while let Some(len) =
        cursor.write_next_of::<InputInfoKind>(&mut buffer, &widest_header(), (), elements.iter().copied()).expect("page")
    {
        pages.push(buffer[..len].to_vec());
    }
    pages
}

fn info_page(bytes: &[u8]) -> InputInfoPage<'_> {
    match decode(bytes).expect("decodes").frame {
        Frame::InputInfo(page) => page,
        other => panic!("not an input info page: {other:?}"),
    }
}

#[test]
fn an_input_event_has_its_golden_bytes() {
    let bytes = encoded(&header(), &Frame::InputEvent(press()));
    assert_eq!(bytes, golden(INPUT_EVENT_KIND, &GOLDEN_EVENT_BODY));
}

#[test]
fn a_page_of_input_info_has_its_golden_bytes() {
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer);
    assert_eq!(buffer[..len], golden(INPUT_INFO_KIND, &GOLDEN_INFO_BODY));
}

#[test]
fn every_variant_of_an_input_frame_keeps_its_index_on_the_wire() {
    let kinds = [
        InputEventKind::Generic,
        InputEventKind::Button,
        InputEventKind::Occupancy,
        InputEventKind::Position,
        InputEventKind::Illuminance,
    ];
    for (index, kind) in kinds.iter().enumerate() {
        assert_eq!(usize::from(wire(kind)[0]), index, "{kind:?}");
    }
    let elements = [owner(), panel("Hall"), button(0, None)];
    for (index, element) in elements.iter().enumerate() {
        assert_eq!(usize::from(wire(element)[0]), index, "{element:?}");
    }
}

#[test]
fn an_input_event_and_an_info_page_decode_to_what_was_encoded() {
    for event in [press(), widest_event()] {
        let bytes = encoded(&widest_header(), &Frame::InputEvent(event));
        let decoded = decode(&bytes).expect("decodes");
        assert_eq!((decoded.header, decoded.frame), (widest_header(), Frame::InputEvent(event)));
    }
    let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = sample_page(&mut buffer);
    let page = info_page(&buffer[..len]);
    assert_eq!(page.meta(), PageMeta { last: true, page: 0, set: SET });
    assert_eq!(page.elements().collect::<Vec<_>>(), info_sample());
}

#[test]
fn the_widest_event_and_the_widest_element_of_each_kind_fit_under_the_ceiling() {
    let event = encoded(&widest_header(), &Frame::InputEvent(widest_event()));
    assert!(event.len() <= MAX_CLUSTER_DATAGRAM_BYTES, "{} B", event.len());
    let name = widest_name();
    let prefix = "\u{10FFFF}".repeat(MAX_INPUT_PREFIX_BYTES / 4);
    let controller = "\u{10FFFF}".repeat(MAX_INSTALLATION_BYTES / 4);
    let widest_owner = InputInfoElement::Owner(InputOwner { controller_id: &controller, discovery_prefix: &prefix });
    for element in [widest_owner, widest_device(LAST_DEVICE, &name), widest_instance(LAST_DEVICE, LAST_INSTANCE)] {
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let mut writer =
            InputInfoWriter::open(&mut buffer, &widest_header(), PagePlace { scope: (), set: u32::MAX, page: u8::MAX })
                .expect("open");
        assert_eq!(writer.push(&element), Ok(()), "{element:?}");
    }
}

#[test]
fn a_node_s_widest_inputs_fill_every_page_but_the_last_and_read_back_in_order() {
    let name = widest_name();
    let mut elements = vec![owner()];
    for device in 0..CLUSTER_INPUT_DEVICES as u8 {
        elements.push(widest_device(device, &name));
        elements.extend((0..CLUSTER_INPUT_INSTANCES as u8).map(|instance| widest_instance(device, instance)));
    }
    let pages = written_pages(&elements);
    let mut read = Vec::new();
    for (index, bytes) in pages.iter().enumerate() {
        let page = info_page(bytes);
        assert_eq!(page.meta(), PageMeta { last: index + 1 == pages.len(), page: index as u8, set: SET });
        read.extend(page.elements());
    }
    assert_eq!(read, elements);
    assert!(pages.len() < usize::from(u8::MAX), "{} pages", pages.len());
}

#[test]
fn each_field_of_an_event_out_of_range_is_refused_on_encode_and_rejected_on_decode() {
    type Break = fn(&mut InputEvent);
    let breaks: [(&str, Break); 9] = [
        ("adapter", |e| e.address.adapter = ADAPTER_COUNT),
        ("device", |e| e.address.device = LAST_DEVICE + 1),
        ("instance", |e| e.address.instance = Some(LAST_INSTANCE + 1)),
        ("type", |e| e.instance_type = Some(LAST_TYPE + 1)),
        ("group", |e| e.groups[2] = Some(LAST_GROUP + 1)),
        ("scheme", |e| e.scheme = MAX_INPUT_SCHEME + 1),
        ("event_info", |e| e.event_info = INPUT_FIELD_MASK + 1),
        ("value", |e| e.value = INPUT_FIELD_MASK + 1),
        ("none", |_| {}),
    ];
    for (field, broken) in breaks {
        let mut event = widest_event();
        broken(&mut event);
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let refused = encode(&header(), &Frame::InputEvent(event), &mut buffer);
        let bytes = unchecked(FrameKind::InputEvent, &event);
        let rejected = decode(&bytes).map(|_| ());
        if field == "none" {
            assert!(refused.is_ok() && rejected.is_ok(), "the widest event is in range");
        } else {
            assert_eq!(refused, Err(EncodeError::OutOfRange), "{field}");
            assert_eq!(rejected, Err(Rejection::Malformed), "{field}");
        }
    }
}

#[test]
fn an_event_with_no_instance_and_no_type_is_in_range() {
    let event = InputEvent {
        address: InputAddress { instance: None, ..press().address },
        instance_type: None,
        groups: [None; 3],
        ..press()
    };
    let bytes = encoded(&header(), &Frame::InputEvent(event));
    assert_eq!(decode(&bytes).expect("decodes").frame, Frame::InputEvent(event));
}

#[test]
fn an_info_element_out_of_range_is_refused_by_the_page_and_rejects_a_received_page() {
    let long = "x".repeat(MAX_NAME_BYTES + 1);
    let prefix = "x".repeat(MAX_INPUT_PREFIX_BYTES + 1);
    let broken_instance = |change: fn(&mut InputInstance)| {
        let InputInfoElement::Instance(mut instance) = widest_instance(0, 0) else { unreachable!() };
        change(&mut instance);
        InputInfoElement::Instance(instance)
    };
    let unnamed = "";
    let broken = [
        InputInfoElement::Owner(InputOwner { controller_id: "ctl1", discovery_prefix: &prefix }),
        panel(&long),
        InputInfoElement::Device(InputDevice { adapter: 0, device: 3, name: unnamed, ha_entity: false, present: false, instances: 33 }),
        broken_instance(|i| i.instance = LAST_INSTANCE + 1),
        broken_instance(|i| i.device = LAST_DEVICE + 1),
        broken_instance(|i| i.groups[0] = Some(LAST_GROUP + 1)),
        broken_instance(|i| i.instance_type = Some(LAST_TYPE + 1)),
        broken_instance(|i| i.last = Some(InputValue { event_info: INPUT_FIELD_MASK + 1, value: None, age_ms: 0 })),
        broken_instance(|i| i.last = Some(InputValue { event_info: 0, value: Some(INPUT_FIELD_MASK + 1), age_ms: 0 })),
    ];
    for element in broken {
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let mut writer = InputInfoWriter::open(&mut buffer, &header(), PagePlace { scope: (), set: SET, page: 0 }).expect("open");
        assert_eq!(writer.push(&element), Err(PushError::OutOfRange), "{element:?}");
        let mut body = wire(&PageMeta { last: true, page: 0, set: SET });
        body.extend(wire(&()));
        body.extend(wire(&element));
        let mut buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        let at = write_head(&mut buffer, FrameKind::InputInfo, &header()).expect("head");
        buffer[at..at + body.len()].copy_from_slice(&body);
        assert_eq!(decode(&buffer[..at + body.len()]).map(|_| ()), Err(Rejection::Malformed), "{element:?}");
    }
}
