use std::time::{Duration, Instant};

use cucumber::{then, when};
use dali2rust_cluster_runtime::frame::{
    decode, encode, DatagramBuffer, Frame, Header, InputAddress, InputDevice, InputEvent, InputInfoElement,
    InputInfoWriter, InputInstance, InputOwner, InputValue, PagePlace,
};
use dali2rust_contracts::msg::InputEventKind;
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_test_support::remains_false_for;
use serde::Deserialize;
use serde_json::Value;

use crate::steps::cluster_steps::{hardware_address, rig, ClusterRig};
use crate::steps::polling::fetch_json;
use crate::DaliWorld;

const HUB_DEADLINE: Duration = Duration::from_secs(4);
const COPIES: usize = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventSpec {
    adapter: u8,
    device: u8,
    instance: Option<u8>,
    instance_type: Option<u8>,
    groups: [Option<u8>; 3],
    scheme: u8,
    kind: String,
    event_info: u16,
    value: u16,
}

impl EventSpec {
    fn parse(json: &str) -> Self {
        serde_json::from_str(json).expect("an input event spec")
    }

    fn event(&self) -> InputEvent {
        let kind = match self.kind.as_str() {
            "generic" => InputEventKind::Generic,
            "button" => InputEventKind::Button,
            "occupancy" => InputEventKind::Occupancy,
            "position" => InputEventKind::Position,
            "illuminance" => InputEventKind::Illuminance,
            other => panic!("no input event kind {other}"),
        };
        InputEvent {
            address: InputAddress { adapter: self.adapter, device: self.device, instance: self.instance },
            instance_type: self.instance_type,
            groups: self.groups,
            scheme: self.scheme,
            kind,
            event_info: self.event_info,
            value: self.value,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoSpec {
    owner: Option<[String; 2]>,
    device: Option<u8>,
    instance: Option<[u8; 2]>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    instances: u8,
    #[serde(default)]
    ha_entity: bool,
    instance_type: Option<u8>,
    #[serde(default)]
    groups: [Option<u8>; 3],
    last: Option<(u16, Option<u16>, u32)>,
}

impl InfoSpec {
    fn element(&self) -> InputInfoElement<'_> {
        if let Some([controller_id, discovery_prefix]) = &self.owner {
            return InputInfoElement::Owner(InputOwner { controller_id, discovery_prefix });
        }
        if let Some(device) = self.device {
            let (name, ha_entity, instances) = (self.name.as_str(), self.ha_entity, self.instances);
            return InputInfoElement::Device(InputDevice { adapter: 0, device, name, ha_entity, present: true, instances });
        }
        let [device, instance] = self.instance.expect("an element names its kind");
        let last = self.last.map(|(event_info, value, age_ms)| InputValue { event_info, value, age_ms });
        let (instance_type, groups) = (self.instance_type, self.groups);
        InputInfoElement::Instance(InputInstance { adapter: 0, device, instance, instance_type, groups, last })
    }
}

fn event_datagram(header: &Header, event: InputEvent) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let len = encode(header, &Frame::InputEvent(event), &mut buffer).expect("the event fits");
    buffer.get(..len).expect("the encoded length").to_vec()
}

fn info_datagram(header: &Header, (set, page, last): (u32, u8, bool), specs: &[InfoSpec]) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let mut writer = InputInfoWriter::open(&mut buffer, header, PagePlace { scope: (), set, page }).expect("the page opens");
    for spec in specs {
        writer.push(&spec.element()).expect("the element fits the page");
    }
    let len = writer.finish(last);
    buffer.get(..len).expect("the encoded length").to_vec()
}

fn sent_inputs(rig: &ClusterRig, window: Duration, enough: usize) -> Vec<(u32, InputEvent, Vec<u8>)> {
    let deadline = Instant::now() + window;
    let mut seen = Vec::new();
    while seen.len() < enough {
        let Some(sent) = rig.sent_by_us(deadline.saturating_duration_since(Instant::now())) else {
            break;
        };
        if let Ok(decoded) = decode(&sent.bytes) {
            if let Frame::InputEvent(event) = decoded.frame {
                seen.push((decoded.header.seq, event, sent.bytes.clone()));
            }
        }
    }
    seen
}

fn input_pairs(sent: &[(u32, InputEvent, Vec<u8>)]) -> Vec<(u32, InputEvent)> {
    sent.chunks(COPIES)
        .map(|pair| {
            let [(seq, event, bytes), (copy_seq, copy, copy_bytes)] = pair else {
                panic!("an input event without its copy: {pair:?}");
            };
            assert_eq!((seq, event), (copy_seq, copy), "the copy follows its event");
            assert_eq!(bytes, copy_bytes, "the copies are the same datagram");
            (*seq, *event)
        })
        .collect()
}

// CLU-240 CLU-243 CLU-244 CLU-245
#[then(regex = r"^the cluster hub should carry from this controller the input events (\[.*\]) in this order, each twice with one sequence number$")]
async fn hub_carries_inputs_in_order(world: &mut DaliWorld, json: String) {
    let specs: Vec<serde_json::Value> = serde_json::from_str(&json).expect("a list of input event specs");
    let wanted: Vec<InputEvent> = specs.iter().map(|spec| EventSpec::parse(&spec.to_string()).event()).collect();
    let pairs = input_pairs(&sent_inputs(rig(world), HUB_DEADLINE, wanted.len() * COPIES));
    let seqs: Vec<u32> = pairs.iter().map(|(seq, _)| *seq).collect();
    assert!(seqs.windows(2).all(|two| two.first() < two.get(1)), "the events left in their order: {seqs:?}");
    let events: Vec<InputEvent> = pairs.into_iter().map(|(_, event)| event).collect();
    assert_eq!(events, wanted, "the input events on the hub");
}

// CLU-240 CLU-241 CLU-242 CLU-243 CLU-244 CLU-245 CLU-250
#[then(regex = r"^the cluster hub should carry no input event from this controller within (\d+) ms$")]
async fn hub_carries_no_input(world: &mut DaliWorld, window_ms: u64) {
    let sent = sent_inputs(rig(world), Duration::from_millis(window_ms), 1);
    assert!(sent.is_empty(), "this controller sent an input event: {:?}", sent.first().map(|(_, event, _)| event));
}

// CLU-242 CLU-250
#[then(regex = r"^the cluster hub should carry no input info page from this controller within (\d+) ms$")]
async fn hub_carries_no_input_info(world: &mut DaliWorld, window_ms: u64) {
    let rig = rig(world);
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let pages = std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now())))
        .filter(|sent| decode(&sent.bytes).is_ok_and(|decoded| matches!(decoded.frame, Frame::InputInfo(_))))
        .count();
    assert_eq!(pages, 0, "this controller sent {pages} input info pages");
}

// CLU-246 CLU-247 CLU-251
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends the input event (\{.*\}) twice with one sequence number$"#)]
async fn board_sends_input_twice(world: &mut DaliWorld, mac: String, json: String) {
    let event = EventSpec::parse(&json).event();
    let speaker = rig(world).speaker(hardware_address(&mac));
    let mut bytes = Vec::new();
    speaker.speak(|header| {
        bytes = event_datagram(header, event);
        bytes.clone()
    });
    speaker.repeat(&bytes);
}

// CLU-252
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends the input events (\{.*\}) and then (\{.*\}), the later one arriving first$"#)]
async fn board_sends_inputs_reordered(world: &mut DaliWorld, mac: String, earlier: String, later: String) {
    let speaker = rig(world).speaker(hardware_address(&mac));
    let earlier_bytes = event_datagram(&speaker.take_header(), EventSpec::parse(&earlier).event());
    let later_bytes = event_datagram(&speaker.take_header(), EventSpec::parse(&later).event());
    speaker.repeat(&later_bytes);
    speaker.repeat(&earlier_bytes);
}

// CLU-247
#[when(regex = r#"^a datagram with this controller's own hardware address carries the input event (\{.*\}) as node (\d) of cluster "([^"]*)"$"#)]
async fn own_address_sends_input(world: &mut DaliWorld, json: String, node: u8, name: String) {
    let event = EventSpec::parse(&json).event();
    rig(world).impostor(node, name).speak(|header| event_datagram(header, event));
}

// CLU-248 CLU-249 CLU-251
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends input info page (\d+) of set (\d+) as (the last page|a page before the last) with (\[.*\])$"#)]
async fn board_sends_input_page(world: &mut DaliWorld, mac: String, page: u8, set: u32, last: String, json: String) {
    let specs: Vec<InfoSpec> = serde_json::from_str(&json).expect("a JSON array of input info elements");
    let place = (set, page, last == "the last page");
    rig(world).speaker(hardware_address(&mac)).speak(|header| info_datagram(header, place, &specs));
}

// CLU-246 CLU-252
#[then(regex = r#"^the JSON pointer "([^"]*)" of the remembered frame should be (.+)$"#)]
async fn remembered_frame_pointer(world: &mut DaliWorld, pointer: String, json: String) {
    let wanted: Value = serde_json::from_str(&json).expect("a JSON value");
    let frame = world.remembered_json.clone().expect("a remembered frame");
    assert_eq!(frame.pointer(&pointer), Some(&wanted), "{pointer} of {frame}");
}

// CLU-248
#[then(regex = r#"^the JSON pointer "([^"]*)" at "([^"]+)" should stay absent for (\d+) ms$"#)]
async fn json_pointer_stays_absent(world: &mut DaliWorld, pointer: String, path: String, window_ms: u64) {
    let port = world.server_port();
    let present = || fetch_json(port, &path).is_some_and(|json| json.pointer(&pointer).is_some());
    assert!(remains_false_for(present, Duration::from_millis(window_ms)), "{path}{pointer} appeared: {:?}", fetch_json(port, &path));
}

// CLU-251
#[then(regex = r#"^the WebSocket client should receive a cluster inputs frame showing node (\d) as "([a-z]+)" with (\d+) devices?$"#)]
async fn ws_cluster_inputs(world: &mut DaliWorld, node: u64, source: String, devices: u64) {
    let client = world.ws_clients.last_mut().expect("an open WebSocket connection");
    let frame = client.wait_for(|frame| {
        let rows = frame["payload"]["nodes"].as_array().cloned().unwrap_or_default();
        frame["type"] == "ClusterInputs"
            && frame["channel"] == "cluster"
            && rows.iter().any(|row| row["node"] == node && row["source"] == source.as_str() && row["device_count"] == devices)
    });
    assert!(frame.is_some(), "no ClusterInputs frame showed node {node} as {source} with {devices} devices");
}
