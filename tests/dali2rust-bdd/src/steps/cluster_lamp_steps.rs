use std::time::{Duration, Instant};

use cucumber::{then, when};
use dali2rust_cluster_runtime::frame::{
    decode, AdapterScope, BirthMark, DatagramBuffer, Frame, HaIdentity, Header, KelvinRange, LampCapabilities, LampColour,
    LampInfo, LampInfoElement, LampInfoWriter, LampState, LampStateWriter, PagePlace, PartState, RecordRef,
    StateElement,
};
use dali2rust_contracts::msg::{ColorMode, DeviceType, PowerState};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_test_support::HubDatagram;
use serde::Deserialize;

use crate::steps::cluster_steps::{hardware_address, rig};
use crate::DaliWorld;

const ADAPTER: AdapterScope = AdapterScope { adapter: 0 };
const DISCOVERY_PREFIX: &str = "homeassistant";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateSpec {
    lamp: Option<u8>,
    #[serde(default)]
    power: String,
    level: Option<u8>,
    #[serde(default)]
    absent: bool,
    part: Option<u8>,
    channel: Option<u8>,
    #[serde(default)]
    mark: [u32; 2],
    #[serde(default)]
    commanded: bool,
    #[serde(default)]
    lit: u8,
    #[serde(default)]
    known: u8,
    #[serde(default)]
    members: u8,
}

impl StateSpec {
    fn element(&self) -> StateElement {
        if let Some(group) = self.part {
            let [generation, nonce] = self.mark;
            let number = self.channel.expect("a part names its channel");
            let channel = RecordRef { number, mark: BirthMark { generation, nonce } };
            let (commanded, lit, known, members) = (self.commanded, self.lit, self.known, self.members);
            return StateElement::Part(PartState { group, channel, commanded, lit, known, members });
        }
        StateElement::Lamp(LampState {
            lamp: self.lamp.expect("an element names its lamp or its part"),
            power: match self.power.as_str() {
                "on" => PowerState::On,
                "off" => PowerState::Off,
                _ => PowerState::Unknown,
            },
            level: self.level,
            colour: LampColour { mode: ColorMode::Brightness, ..LampColour::default() },
            age_ms: self.level.map(|_| 0),
            source: None,
            absent: self.absent,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoSpec {
    owner: Option<String>,
    lamp: Option<u8>,
    #[serde(default)]
    name: String,
    bound: Option<bool>,
    cct: Option<[u16; 2]>,
}

impl InfoSpec {
    fn element(&self) -> LampInfoElement<'_> {
        if let Some(controller_id) = &self.owner {
            return LampInfoElement::Owner(HaIdentity { controller_id, discovery_prefix: DISCOVERY_PREFIX });
        }
        LampInfoElement::Lamp(LampInfo {
            lamp: self.lamp.expect("an element names the owner or a lamp"),
            name: &self.name,
            device_type: DeviceType::Dt6Led,
            device_type_overridden: false,
            device_types: 0,
            color_mode: ColorMode::Brightness,
            color_mode_overridden: false,
            capabilities: LampCapabilities { brightness: true, cct: self.cct.is_some(), ..LampCapabilities::default() },
            kelvin_range: self.cct.map(|[min, max]| KelvinRange { min, max }),
            bound: self.bound.unwrap_or(true),
            ha_entity: true,
        })
    }
}

fn place(set: u32, page: u8) -> PagePlace<AdapterScope> {
    PagePlace { scope: ADAPTER, set, page }
}

fn state_datagram(header: &Header, (set, page, last): (u32, u8, bool), specs: &[StateSpec]) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let mut writer = LampStateWriter::open(&mut buffer, header, place(set, page)).expect("the page opens");
    for spec in specs {
        writer.push(&spec.element()).expect("the element fits the page");
    }
    let len = writer.finish(last);
    buffer.get(..len).expect("the encoded length").to_vec()
}

fn info_datagram(header: &Header, (set, page, last): (u32, u8, bool), specs: &[InfoSpec]) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let mut writer = LampInfoWriter::open(&mut buffer, header, place(set, page)).expect("the page opens");
    for spec in specs {
        writer.push(&spec.element()).expect("the element fits the page");
    }
    let len = writer.finish(last);
    buffer.get(..len).expect("the encoded length").to_vec()
}

// CLU-200 CLU-201 CLU-202 CLU-203 CLU-204 CLU-205 CLU-209 CLU-210 CLU-227 CLU-228 CLU-230
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends lamp state page (\d+) of set (\d+) as (the last page|a page before the last) with (\[.*\])$"#)]
async fn board_sends_state(world: &mut DaliWorld, mac: String, page: u8, set: u32, last: String, json: String) {
    let specs: Vec<StateSpec> = serde_json::from_str(&json).expect("a JSON array of lamp state elements");
    let at = (set, page, last == "the last page");
    rig(world).speaker(hardware_address(&mac)).speak(|header| state_datagram(header, at, &specs));
}

// CLU-200 CLU-201 CLU-202 CLU-203 CLU-204 CLU-205 CLU-210 CLU-227 CLU-228 CLU-230
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends lamp info page (\d+) of set (\d+) as (the last page|a page before the last) with (\[.*\])$"#)]
async fn board_sends_info(world: &mut DaliWorld, mac: String, page: u8, set: u32, last: String, json: String) {
    let specs: Vec<InfoSpec> = serde_json::from_str(&json).expect("a JSON array of lamp info elements");
    let at = (set, page, last == "the last page");
    rig(world).speaker(hardware_address(&mac)).speak(|header| info_datagram(header, at, &specs));
}

fn own_pages(world: &mut DaliWorld, window: Duration) -> Vec<HubDatagram> {
    let rig = rig(world);
    let deadline = Instant::now() + window;
    std::iter::from_fn(|| rig.sent_by_us(deadline.saturating_duration_since(Instant::now()))).collect()
}

fn state_levels(datagram: &HubDatagram) -> Option<Vec<(u8, Option<u8>)>> {
    let Frame::LampState(page) = decode(&datagram.bytes).ok()?.frame else {
        return None;
    };
    let lamps = page.elements().filter_map(|element| match element {
        StateElement::Lamp(lamp) => Some((lamp.lamp, lamp.level)),
        StateElement::Part(_) => None,
    });
    Some(lamps.collect())
}

fn info_names(datagram: &HubDatagram) -> Option<Vec<(u8, String)>> {
    let Frame::LampInfo(page) = decode(&datagram.bytes).ok()?.frame else {
        return None;
    };
    let lamps = page.elements().filter_map(|element| match element {
        LampInfoElement::Lamp(lamp) => Some((lamp.lamp, lamp.name.to_owned())),
        LampInfoElement::Owner(_) => None,
    });
    Some(lamps.collect())
}

// CLU-206
#[then(regex = r"^the cluster hub should carry lamp state pages from this controller no two within (\d+) ms of each other, the last showing lamp (\d+) at level (\d+), within (\d+) ms$")]
async fn hub_carries_coalesced_state(world: &mut DaliWorld, gap_ms: u64, lamp: u8, level: u8, window_ms: u64) {
    let pages: Vec<(Instant, Vec<(u8, Option<u8>)>)> = own_pages(world, Duration::from_millis(window_ms))
        .iter()
        .filter_map(|sent| state_levels(sent).map(|levels| (sent.at, levels)))
        .collect();
    let last = pages.last().map(|(_, levels)| levels.clone()).unwrap_or_default();
    assert!(last.contains(&(lamp, Some(level))), "the last state page shows {last:?}");
    let gap = Duration::from_millis(gap_ms);
    let close = pages.windows(2).filter(|pair| pair[1].0.saturating_duration_since(pair[0].0) < gap).count();
    assert_eq!(close, 0, "{close} of {} state pages came within {gap_ms} ms of the one before", pages.len());
}

// CLU-208
#[then(regex = r"^the cluster hub should carry exactly one lamp state page from this controller showing a lamp at level (\d+), and it shows lamps (\[.*\]) at that level, within (\d+) ms$")]
async fn hub_carries_one_page_for_all(world: &mut DaliWorld, level: u8, json: String, window_ms: u64) {
    let lamps: Vec<u8> = serde_json::from_str(&json).expect("a JSON array of lamp numbers");
    let pages: Vec<Vec<(u8, Option<u8>)>> =
        own_pages(world, Duration::from_millis(window_ms)).iter().filter_map(state_levels).collect();
    let at_level: Vec<&Vec<(u8, Option<u8>)>> =
        pages.iter().filter(|levels| levels.iter().any(|(_, shown)| *shown == Some(level))).collect();
    assert_eq!(at_level.len(), 1, "state pages showing a lamp at {level}: {at_level:?}");
    let shows_all = at_level.iter().all(|levels| lamps.iter().all(|lamp| levels.contains(&(*lamp, Some(level)))));
    assert!(shows_all, "the page does not show lamps {lamps:?} at {level}: {at_level:?}");
}

fn state_parts(datagram: &HubDatagram) -> Option<Vec<(u8, u8)>> {
    let Frame::LampState(page) = decode(&datagram.bytes).ok()?.frame else {
        return None;
    };
    let parts = page.elements().filter_map(|element| match element {
        StateElement::Part(part) => Some((part.group, part.channel.number)),
        StateElement::Lamp(_) => None,
    });
    Some(parts.collect())
}

// CLU-211
#[then(regex = r"^the cluster hub should carry a lamp state page from this controller showing part (\d+) of channel (\d+) within (\d+) ms$")]
async fn hub_carries_part(world: &mut DaliWorld, group: u8, channel: u8, window_ms: u64) {
    let rig = rig(world);
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let wanted = (group, channel);
    let mut seen = Vec::new();
    while let Some(sent) = rig.sent_by_us(deadline.saturating_duration_since(Instant::now())) {
        let Some(parts) = state_parts(&sent) else { continue };
        if parts.contains(&wanted) {
            return;
        }
        seen.push(parts);
    }
    panic!("no lamp state page showed part {group} of channel {channel} within {window_ms} ms: {seen:?}");
}

// CLU-207
#[then(regex = r#"^the cluster hub should carry a lamp info page from this controller naming lamp (\d+) "([^"]*)" within (\d+) ms$"#)]
async fn hub_carries_lamp_name(world: &mut DaliWorld, lamp: u8, name: String, window_ms: u64) {
    let pages: Vec<Vec<(u8, String)>> =
        own_pages(world, Duration::from_millis(window_ms)).iter().filter_map(info_names).collect();
    let wanted = (lamp, name.clone());
    assert!(pages.iter().any(|names| names.contains(&wanted)), "no info page named lamp {lamp} {name:?}: {pages:?}");
}

// CLU-210
#[then(regex = r#"^the WebSocket client should receive a cluster lamps frame showing node (\d) as "([a-z]+)" with (\d+) lamps?$"#)]
async fn ws_cluster_lamps(world: &mut DaliWorld, node: u64, source: String, lamps: u64) {
    let client = world.ws_clients.last_mut().expect("an open WebSocket connection");
    let frame = client.wait_for(|frame| {
        let rows = frame["payload"]["nodes"].as_array().cloned().unwrap_or_default();
        frame["type"] == "ClusterLamps"
            && frame["channel"] == "cluster"
            && rows.iter().any(|row| row["node"] == node && row["source"] == source.as_str() && row["lamps"] == lamps)
    });
    assert!(frame.is_some(), "no ClusterLamps frame showed node {node} as {source} with {lamps} lamps");
}
