use std::time::{Duration, Instant};

use cucumber::{then, when};
use dali2rust_cluster_runtime::frame::{
    decode, BirthMark, ChannelPart, DatagramBuffer, Frame, GroupMap, Header, Hello, InfoElement, InfoPageWriter, LocalName,
    PartFlags, Record, RecordRef, ScenePart,
};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_test_support::wait_until;
use serde::Deserialize;

use crate::steps::cluster_steps::{hardware_address, rig};
use crate::steps::polling::fetch_json;
use crate::DaliWorld;

const INFO_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementSpec {
    channel: Option<u8>,
    scene: Option<u8>,
    channel_part: Option<[u8; 2]>,
    scene_part: Option<[u8; 2]>,
    group_map: Option<u8>,
    group_name: Option<[u8; 2]>,
    scene_name: Option<[u8; 2]>,
    record: Option<u8>,
    #[serde(default)]
    mark: [u32; 2],
    #[serde(default)]
    name: String,
    #[serde(default)]
    send: bool,
    #[serde(default)]
    accept: bool,
    #[serde(default)]
    lamps: u64,
}

impl ElementSpec {
    fn reference(&self, number: u8) -> RecordRef {
        let [generation, nonce] = self.mark;
        RecordRef { number, mark: BirthMark { generation, nonce } }
    }

    fn record(&self, number: u8) -> Record<'_> {
        Record { id: self.reference(number), name: &self.name, revision: 1 }
    }

    fn named(&self) -> RecordRef {
        self.reference(self.record.expect("a part names its record"))
    }

    fn flags(&self) -> PartFlags {
        PartFlags { send: self.send, accept: self.accept }
    }

    fn element(&self) -> InfoElement<'_> {
        let name = |[adapter, number]: [u8; 2]| LocalName { adapter, number, name: &self.name };
        if let Some(number) = self.channel {
            return InfoElement::ChannelRecord(self.record(number));
        }
        if let Some(number) = self.scene {
            return InfoElement::SceneRecord(self.record(number));
        }
        if let Some(key) = self.channel_part {
            return InfoElement::ChannelPart(self.channel_part(key));
        }
        if let Some([adapter, node_scene]) = self.scene_part {
            let (scene, flags) = (self.named(), self.flags());
            return InfoElement::ScenePart(ScenePart { adapter, scene, node_scene, flags, revision: 1 });
        }
        if let Some(key) = self.group_name {
            return InfoElement::GroupName(name(key));
        }
        if let Some(key) = self.scene_name {
            return InfoElement::SceneName(name(key));
        }
        let adapter = self.group_map.expect("an element names its kind");
        InfoElement::GroupMap(GroupMap { adapter, occupied: 1 })
    }

    fn channel_part(&self, [adapter, group]: [u8; 2]) -> ChannelPart {
        ChannelPart {
            adapter,
            channel: self.named(),
            group,
            taken: false,
            flags: self.flags(),
            revision: 1,
            desired_lamps: self.lamps,
            written_lamps: 0,
        }
    }
}

fn parse_elements(json: &str) -> Vec<ElementSpec> {
    serde_json::from_str(json).expect("a JSON array of info elements")
}

fn page_datagram(header: &Header, (set, page, last): (u32, u8, bool), specs: &[ElementSpec]) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let mut writer = InfoPageWriter::start(&mut buffer, header, set, page).expect("the page opens");
    for spec in specs {
        writer.push(&spec.element()).expect("the element fits the page");
    }
    let len = writer.finish(last);
    buffer.get(..len).expect("the encoded length").to_vec()
}

// CLU-057 CLU-058 CLU-059 CLU-060 CLU-061 CLU-062 CLU-063 CLU-064 CLU-068 CLU-071 CLU-072 CLU-073 CLU-074 CLU-075 CLU-083
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends info page (\d+) of set (\d+) as (the last page|a page before the last) with (\[.*\])$"#)]
async fn board_sends_page(world: &mut DaliWorld, mac: String, page: u8, set: u32, last: String, json: String) {
    let specs = parse_elements(&json);
    let place = (set, page, last == "the last page");
    rig(world).speaker(hardware_address(&mac)).speak(|header| page_datagram(header, place, &specs));
}

// CLU-068 CLU-070
#[when(regex = r#"^the board "([0-9a-f:]{17})" says hello$"#)]
async fn board_says_hello(world: &mut DaliWorld, mac: String) {
    let rig = rig(world);
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    rig.speaker(hardware_address(&mac)).say(&mut buffer, &Frame::Hello(Hello { ha_face: false }));
}

fn carries(datagram: &[u8], mac: [u8; 6], wanted: &ElementSpec) -> bool {
    let Ok(decoded) = decode(datagram) else {
        return false;
    };
    let Frame::ChannelSceneInfo(page) = decoded.frame else {
        return false;
    };
    decoded.header.sender_mac == mac && page.elements().any(|element| same_entry(&element, &wanted.element()))
}

fn same_entry(sent: &InfoElement<'_>, wanted: &InfoElement<'_>) -> bool {
    match (sent, wanted) {
        (InfoElement::ChannelRecord(sent), InfoElement::ChannelRecord(wanted))
        | (InfoElement::SceneRecord(sent), InfoElement::SceneRecord(wanted)) => {
            (sent.id.number, sent.name) == (wanted.id.number, wanted.name)
        }
        (InfoElement::ChannelPart(sent), InfoElement::ChannelPart(wanted)) => {
            let key = |part: &ChannelPart| (part.adapter, part.group, part.channel.number, part.flags);
            key(sent) == key(wanted)
        }
        (InfoElement::ScenePart(sent), InfoElement::ScenePart(wanted)) => {
            let key = |part: &ScenePart| (part.adapter, part.node_scene, part.scene.number, part.flags);
            key(sent) == key(wanted)
        }
        _ => false,
    }
}

fn is_info_page(datagram: &[u8]) -> bool {
    decode(datagram).is_ok_and(|decoded| matches!(decoded.frame, Frame::ChannelSceneInfo(_)))
}

// CLU-065 CLU-066 CLU-068 CLU-070
#[then(regex = r#"^the cluster hub should carry an info page from this controller with (\{.*\}) within (\d+) ms$"#)]
async fn hub_carries_info(world: &mut DaliWorld, json: String, window_ms: u64) {
    let wanted: ElementSpec = serde_json::from_str(&json).expect("a JSON info element");
    let rig = rig(world);
    let mac = rig.mac();
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let found = std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now())))
        .any(|sent| carries(&sent.bytes, mac, &wanted));
    assert!(found, "no info page from this controller carried {json} within {window_ms} ms");
}

// CLU-068 CLU-069
#[then(regex = r"^the cluster hub should carry no info page from this controller within (\d+) ms$")]
async fn hub_carries_no_info(world: &mut DaliWorld, window_ms: u64) {
    let rig = rig(world);
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let pages = std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now())))
        .filter(|sent| is_info_page(&sent.bytes))
        .count();
    assert_eq!(pages, 0, "this controller sent {pages} info pages");
}

// CLU-073
#[then(regex = r#"^the WebSocket client should receive a cluster info frame showing node (\d) as "([a-z]+)" with (\d+) channels?$"#)]
async fn ws_cluster_info(world: &mut DaliWorld, node: u64, source: String, channels: u64) {
    let client = world.ws_clients.last_mut().expect("an open WebSocket connection");
    let frame = client.wait_for(|frame| {
        let rows = frame["payload"]["nodes"].as_array().cloned().unwrap_or_default();
        frame["type"] == "ClusterInfo"
            && frame["channel"] == "cluster"
            && rows.iter().any(|row| row["node"] == node && row["source"] == source.as_str() && row["channels"] == channels)
    });
    assert!(frame.is_some(), "no ClusterInfo frame showed node {node} as {source} with {channels} channels");
}

// CLU-074 CLU-075
#[then(regex = r#"^the JSON pointer "([^"]*)" at "([^"]+)" should eventually be absent$"#)]
async fn json_pointer_eventually_absent(world: &mut DaliWorld, pointer: String, path: String) {
    let port = world.server_port();
    let read = || fetch_json(port, &path).map(|json| json.pointer(&pointer).cloned());
    wait_until(|| matches!(read(), Some(None)), INFO_TIMEOUT);
    let found = read();
    assert!(matches!(found, Some(None)), "{path}{pointer} is still there: {found:?}");
}
