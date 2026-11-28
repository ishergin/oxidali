use core::mem::size_of;

use dali2rust_contracts::msg::{ColorMode, DeviceType, PowerState, RuntimeSource};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{
    cluster_tag, decode, AdapterScope, BirthMark, Frame, HaIdentity, Header, LampColour, LampInfo, LampInfoElement,
    LampInfoWriter, LampStateWriter, PagePlace, RecordRef, StateElement, ADAPTER_COUNT,
};

const TIMING: PresenceTiming = PresenceTiming::every_ms(1_000);
const NODE: u8 = 2;
const MAC: [u8; 6] = [0x30, 0xed, 0xa0, 0x7b, 0x90, 0x02];
const NEW_BOARD: [u8; 6] = [0x30, 0xed, 0xa0, 0x7b, 0x90, 0x22];
const EPOCH: u32 = 7;
const NOW_MS: u32 = 50_000;
const CHANNEL: RecordRef = RecordRef { number: 3, mark: BirthMark { generation: 1, nonce: 0x0bad_cafe } };
const SEVEN_NODES: usize = 7;
const LAMP: u8 = 6;
const OTHER_LAMP: u8 = 9;
const GROUP: u8 = 5;

fn header(mac: [u8; 6]) -> Header {
    Header { cluster_tag: cluster_tag("home"), sender_mac: mac, boot_epoch: EPOCH, node_no: NODE, seq: 1 }
}

const fn arrival(mac: [u8; 6], now_ms: u32) -> LampArrival {
    LampArrival { node: NODE, mac, epoch: EPOCH, now_ms }
}

fn lit(lamp: u8, level: Option<u8>) -> StateElement {
    StateElement::Lamp(LampState {
        lamp,
        power: PowerState::On,
        level,
        colour: LampColour { mode: ColorMode::Brightness, ..LampColour::default() },
        age_ms: Some(1_000),
        source: Some(RuntimeSource::Api),
        absent: false,
    })
}

const fn part(lit: u8) -> StateElement {
    StateElement::Part(PartState { group: GROUP, channel: CHANNEL, commanded: true, lit, known: 2, members: 2 })
}

struct Built {
    bytes: [u8; MAX_CLUSTER_DATAGRAM_BYTES],
    len: usize,
}

fn state_page(mac: [u8; 6], (adapter, set, page, last): (u8, u32, u8, bool), elements: &[StateElement]) -> Built {
    let mut bytes = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: AdapterScope { adapter }, set, page };
    let mut writer = LampStateWriter::open(&mut bytes, &header(mac), place).expect("open");
    elements.iter().for_each(|element| writer.push(element).expect("push"));
    let len = writer.finish(last);
    Built { bytes, len }
}

fn info_page(mac: [u8; 6], set: u32, elements: &[LampInfoElement<'_>]) -> Built {
    let mut bytes = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: AdapterScope { adapter: 0 }, set, page: 0 };
    let mut writer = LampInfoWriter::open(&mut bytes, &header(mac), place).expect("open");
    elements.iter().for_each(|element| writer.push(element).expect("push"));
    let len = writer.finish(true);
    Built { bytes, len }
}

fn take_state(lamps: &ClusterLamps, at: LampArrival, built: &Built) -> LampTally {
    let Frame::LampState(page) = decode(&built.bytes[..built.len]).expect("decode").frame else { panic!("state") };
    lamps.take_state(at, &page, lamps.era())
}

fn take_info(lamps: &ClusterLamps, at: LampArrival, built: &Built) -> LampTally {
    let Frame::LampInfo(page) = decode(&built.bytes[..built.len]).expect("decode").frame else { panic!("info") };
    lamps.take_info(at, &page, lamps.era())
}

fn lamp_at(lamps: &ClusterLamps, lamp: u8, now_ms: u32) -> Option<CachedLampView> {
    let cache = lamps.lock();
    lamps.lamp_view((NODE, 0, lamp), cache.node(NODE)?, now_ms)
}

fn held_at(lamps: &ClusterLamps, now_ms: u32) -> Option<LampsHeldView> {
    let cache = lamps.lock();
    Some(lamps.held_view(NODE, cache.node(NODE)?, now_ms))
}

fn named(lamp: u8, name: &str) -> LampInfoElement<'_> {
    LampInfoElement::Lamp(LampInfo {
        lamp,
        name,
        device_type: DeviceType::Dt6Led,
        device_type_overridden: false,
        device_types: 0,
        color_mode: ColorMode::Brightness,
        color_mode_overridden: false,
        capabilities: crate::frame::LampCapabilities { brightness: true, ..Default::default() },
        kelvin_range: None,
        bound: true,
        ha_entity: true,
    })
}

#[test]
fn a_paged_state_set_is_shown_only_when_its_last_page_arrives() {
    let lamps = ClusterLamps::new(TIMING);
    let level = |lamps: &ClusterLamps| lamp_at(lamps, LAMP, NOW_MS).and_then(|view| view.state).and_then(|state| state.level);
    let first = take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, false), &[lit(LAMP, Some(200))]));
    assert!(!first.taken);
    assert_eq!(lamp_at(&lamps, LAMP, NOW_MS), None, "half a set is never shown");
    let last = take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 1, true), &[part(1)]));
    assert!(last.taken && last.changed);
    assert_eq!(level(&lamps), Some(200), "the whole set is shown");
    assert!(lamps.cached_part(NODE, 0, GROUP).is_some());
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 2, 0, false), &[lit(LAMP, Some(50))]));
    assert_eq!(level(&lamps), Some(200), "the shown set holds while the next one is half in");
    assert!(lamps.cached_part(NODE, 0, GROUP).is_some(), "and keeps its parts");
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 2, 1, true), &[]));
    assert_eq!(level(&lamps), Some(50));
    assert_eq!(lamps.cached_part(NODE, 0, GROUP), None, "a set is taken whole, so a part it lacks goes");
}

#[test]
fn a_lost_page_keeps_the_previous_set() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, Some(100))]));
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 2, 0, false), &[lit(LAMP, Some(254))]));
    let late = take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 2, 2, true), &[part(1)]));
    assert!(late.abandoned && !late.taken, "page 1 was lost, so set 2 is abandoned");
    let shown = lamp_at(&lamps, LAMP, NOW_MS).expect("set 1 is still shown");
    assert_eq!(shown.state.and_then(|state| state.level), Some(100));
}

#[test]
fn an_unknown_level_stays_unknown_in_the_cache() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, None)]));
    let state = lamp_at(&lamps, LAMP, NOW_MS).and_then(|view| view.state).expect("held");
    assert_eq!(state.level, None);
}

#[test]
fn a_set_turns_stale_after_three_refresh_periods() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, Some(10))]));
    let limit_ms = NOW_MS + TIMING.info_refresh_ms() * STALE_REFRESH_PERIODS;
    assert!(lamp_at(&lamps, LAMP, limit_ms).expect("held").fresh);
    let stale = lamp_at(&lamps, LAMP, limit_ms + 1).expect("a stale lamp stays listed");
    assert!(!stale.fresh);
    assert!(!held_at(&lamps, limit_ms + 1).expect("held").fresh);
}

#[test]
fn the_served_age_counts_the_time_since_the_set_was_heard() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, Some(10))]));
    let later_ms = NOW_MS + 4_000;
    let state = lamp_at(&lamps, LAMP, later_ms).and_then(|view| view.state).expect("held");
    assert_eq!(state.age_ms, Some(5_000));
}

#[test]
fn a_new_board_on_the_node_number_replaces_the_old_boards_lamps_at_once() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 9, 0, true), &[lit(LAMP, Some(10))]));
    let changes = lamps.changes(NOW_MS);
    let first = take_state(&lamps, arrival(NEW_BOARD, NOW_MS), &state_page(NEW_BOARD, (0, 1, 0, false), &[]));
    assert!(!first.stale, "the new board's low set number is not stale against the old board");
    assert_eq!(lamp_at(&lamps, LAMP, NOW_MS), None, "the old board's lamps are gone before the new set is whole");
    assert_ne!(lamps.changes(NOW_MS), changes);
    take_state(&lamps, arrival(NEW_BOARD, NOW_MS), &state_page(NEW_BOARD, (0, 1, 1, true), &[lit(OTHER_LAMP, Some(5))]));
    assert_eq!(lamp_at(&lamps, OTHER_LAMP, NOW_MS).map(|view| view.mac), Some(NEW_BOARD));
}

#[test]
fn forget_drops_the_nodes_lamps_and_clear_refuses_a_page_read_before_it() {
    let lamps = ClusterLamps::new(TIMING);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, Some(10))]));
    let built = state_page(MAC, (0, 2, 0, true), &[lit(LAMP, Some(10))]);
    let Frame::LampState(page) = decode(&built.bytes[..built.len]).expect("decode").frame else { panic!("state") };
    let before_forget = lamps.era();
    lamps.forget(NODE);
    assert_eq!(lamps.lamps_held(NODE), None);
    assert!(!lamps.take_state(arrival(MAC, NOW_MS), &page, before_forget).taken, "a page admitted before Forget is refused");
    assert_eq!(lamps.lamps_held(NODE), None, "and does not bring the node back");
    let era = lamps.era();
    lamps.clear();
    let tally = lamps.take_state(arrival(MAC, NOW_MS), &page, era);
    assert!(!tally.taken, "a page admitted before the clear is refused");
    assert_eq!(lamps.lamps_held(NODE), None);
}

#[test]
fn a_pulse_of_a_new_boot_drops_the_board_s_lamps_and_another_board_s_pulse_does_not() {
    let lamps = ClusterLamps::new(TIMING);
    take_info(&lamps, arrival(MAC, NOW_MS), &info_page(MAC, 1, &[named(LAMP, "Hall")]));
    lamps.note_run(arrival(MAC, NOW_MS));
    lamps.note_run(LampArrival { mac: NEW_BOARD, epoch: EPOCH + 1, ..arrival(NEW_BOARD, NOW_MS) });
    assert_eq!(lamps.cached_lamp_ids(NODE, 0), 1 << LAMP, "the same run, or another board, keeps the lamps");
    lamps.note_run(LampArrival { epoch: EPOCH + 1, ..arrival(MAC, NOW_MS) });
    assert_eq!(lamps.lamps_held(NODE), None, "the board booted again, so what it said before is gone");
}

#[test]
fn a_set_for_an_adapter_that_is_not_held_is_refused() {
    let lamps = ClusterLamps::new(TIMING);
    let unheld = ADAPTER_COUNT - 1;
    assert!(unheld >= LAMP_ADAPTERS);
    let tally = take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (unheld, 1, 0, true), &[lit(LAMP, Some(1))]));
    assert!(tally.unheld);
    assert_eq!(lamps.cached_lamp_ids(NODE, unheld), 0);
}

#[test]
fn lamp_info_carries_the_owner_identity_and_the_name() {
    let lamps = ClusterLamps::new(TIMING);
    let owner = LampInfoElement::Owner(HaIdentity { controller_id: "ctl2", discovery_prefix: "homeassistant" });
    take_info(&lamps, arrival(MAC, NOW_MS), &info_page(MAC, 1, &[owner, named(LAMP, "Hall")]));
    let view = lamp_at(&lamps, LAMP, NOW_MS).expect("info alone lists the lamp");
    assert_eq!(view.state, None, "no state was heard, so none is shown");
    assert_eq!(view.info.map(|info| info.name.as_str().to_owned()).as_deref(), Some("Hall"));
    assert_eq!(lamps.cached_identity(NODE).map(|owner| owner.controller_id.as_str().to_owned()).as_deref(), Some("ctl2"));
    take_info(&lamps, arrival(MAC, NOW_MS), &info_page(MAC, 2, &[owner, named(LAMP, "Kitchen")]));
    let renamed = lamp_at(&lamps, LAMP, NOW_MS).and_then(|view| view.info).expect("held");
    assert_eq!(renamed.name.as_str(), "Kitchen");
}

#[test]
fn a_lamp_is_listed_only_once_its_description_has_arrived() {
    let lamps = ClusterLamps::new(TIMING);
    let held_lamps = |lamps: &ClusterLamps| held_at(lamps, NOW_MS).map(|held| held.lamps);
    take_state(&lamps, arrival(MAC, NOW_MS), &state_page(MAC, (0, 1, 0, true), &[lit(LAMP, Some(10))]));
    assert_eq!(lamps.cached_lamp_ids(NODE, 0), 0, "a lamp with a state and no description is not listed");
    assert_eq!(lamps.cached_lamp(NODE, 0, LAMP), None, "nor served");
    assert_eq!(held_lamps(&lamps), Some(0), "nor counted");
    take_info(&lamps, arrival(MAC, NOW_MS), &info_page(MAC, 1, &[named(LAMP, "Hall")]));
    assert_eq!(lamps.cached_lamp_ids(NODE, 0), 1 << LAMP);
    assert!(lamps.cached_lamp(NODE, 0, LAMP).is_some_and(|view| view.state.is_some()), "described, it shows its state");
    assert_eq!(held_lamps(&lamps), Some(1));
}

#[test]
fn the_cache_weight_for_seven_nodes_is_measured_and_frozen() {
    let per_node = size_of::<NodeLamps>();
    let seven = per_node * SEVEN_NODES;
    println!("lamp cache: {per_node} B per node, {seven} B for seven nodes, {} B placed", size_of::<LampCache>());
    assert!(size_of::<LampCache>() <= MAX_LAMP_CACHE_BYTES, "the lamp cache outgrew its PSRAM budget");
}

const MAX_LAMP_CACHE_BYTES: usize = 160 * 1024;
