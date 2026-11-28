use core::mem::size_of;

use dali2rust_contracts::msg::InputEventKind;
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_platform::liveness::monotonic_ms;

use super::*;
use crate::frame::{
    cluster_tag, decode, DatagramBuffer, Frame, Header, InputAddress, InputDevice, InputInfoElement, InputInfoWriter,
    InputInstance, InputOwner, InputValue, PagePlace,
};

const TIMING: PresenceTiming = PresenceTiming::every_ms(1_000);
const NODE: u8 = 2;
const MAC: [u8; 6] = [2, 0, 0, 0, 0, 2];
const NEW_MAC: [u8; 6] = [2, 0, 0, 0, 0, 7];
const EPOCH: u32 = 11;
const START_MS: u32 = 400_000;
const OLD_LUX: u16 = 120;
const NEW_LUX: u16 = 480;
const SENSOR: u8 = 1;
const EXTRA: u8 = 2;
const NODES_BESIDE_THIS_ONE: usize = 7;
const INPUT_CACHE_CEILING_BYTES: usize = 312 * 1024;

fn header(mac: [u8; 6], seq: u32) -> Header {
    Header { cluster_tag: cluster_tag("home"), sender_mac: mac, boot_epoch: EPOCH, node_no: NODE, seq }
}

fn arrival(mac: [u8; 6], now_ms: u32) -> InputArrival {
    InputArrival { node: NODE, mac, epoch: EPOCH, now_ms }
}

fn device(name: &str) -> InputInfoElement<'_> {
    InputInfoElement::Device(InputDevice { adapter: 0, device: 4, name, ha_entity: true, present: true, instances: 2 })
}

fn instance(instance: u8, last: Option<InputValue>) -> InputInfoElement<'static> {
    let groups = [Some(3), None, None];
    InputInfoElement::Instance(InputInstance { adapter: 0, device: 4, instance, instance_type: Some(4), groups, last })
}

fn sensor(last: Option<InputValue>) -> InputInfoElement<'static> {
    instance(SENSOR, last)
}

fn lux(value: u16, age_ms: u32) -> Option<InputValue> {
    Some(InputValue { event_info: 0, value: Some(value), age_ms })
}

fn elements(name: &str, last: Option<InputValue>) -> Vec<InputInfoElement<'_>> {
    let owner = InputInfoElement::Owner(InputOwner { controller_id: "hall", discovery_prefix: "homeassistant" });
    vec![owner, device(name), sensor(last)]
}

fn pages(mac: [u8; 6], set: u32, elements: &[InputInfoElement<'_>], per_page: usize) -> Vec<Vec<u8>> {
    let chunks: Vec<&[InputInfoElement<'_>]> = elements.chunks(per_page).collect();
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let mut out = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let place = PagePlace { scope: (), set, page: u8::try_from(index).expect("few pages") };
        let mut writer = InputInfoWriter::open(&mut buffer, &header(mac, set), place).expect("open");
        chunk.iter().for_each(|element| writer.push(element).expect("fits"));
        let len = writer.finish(index + 1 == chunks.len());
        out.push(buffer[..len].to_vec());
    }
    out
}

fn take(cache: &mut InputCache, mac: [u8; 6], now_ms: u32, bytes: &[u8]) -> InputTally {
    let Frame::InputInfo(page) = decode(bytes).expect("decodes").frame else { panic!("an input info page") };
    cache.take_page(arrival(mac, now_ms), &page)
}

fn shown_value(cache: &InputCache, now_ms: u32) -> Option<(Option<u16>, u32)> {
    let held = cache.node(NODE)?.shown_set()?.instance_at(0, 0)?;
    held.last.map(|last| (last.value, now_ms.wrapping_sub(last.heard_ms)))
}

fn event(value: u16) -> InputEvent {
    InputEvent {
        address: InputAddress { adapter: 0, device: 4, instance: Some(SENSOR) },
        instance_type: Some(4),
        groups: [Some(3), None, None],
        scheme: 1,
        kind: InputEventKind::Illuminance,
        event_info: 0,
        value,
    }
}

#[test]
fn a_node_s_inputs_show_only_once_the_last_page_of_its_set_arrived() {
    let mut cache = placed_input_cache();
    let paged = pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 2);
    assert_eq!(paged.len(), 2);
    assert!(!take(&mut cache, MAC, START_MS, &paged[0]).taken);
    assert!(cache.node(NODE).is_none(), "half a set is not shown");
    let tally = take(&mut cache, MAC, START_MS, &paged[1]);
    assert!(tally.taken && tally.changed);
    assert_eq!(cache.node(NODE).and_then(|inputs| inputs.shown_set()).map(InputSet::device_count), Some(1));
}

#[test]
fn a_set_with_a_missing_page_is_abandoned_and_the_shown_one_stays() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", None), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    let renamed = pages(MAC, 2, &elements("porch", None), 1);
    take(&mut cache, MAC, START_MS, &renamed[0]);
    let tally = take(&mut cache, MAC, START_MS, &renamed[2]);
    assert!(tally.stale && tally.abandoned && !tally.taken);
    let name = cache.node(NODE).and_then(|inputs| inputs.shown_set()?.device_at(0).map(|held| held.name.as_str().to_owned()));
    assert_eq!(name.as_deref(), Some("hall"));
}

#[test]
fn an_event_heard_after_a_page_is_not_overwritten_by_the_next_page_s_older_value() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    assert_eq!(cache.hear_event(arrival(MAC, START_MS + 100), 5, &event(NEW_LUX)), InputHeard::Applied);
    assert_eq!(shown_value(&cache, START_MS + 100), Some((Some(NEW_LUX), 0)));
    for bytes in pages(MAC, 2, &elements("hall", lux(OLD_LUX, 200)), 3) {
        take(&mut cache, MAC, START_MS + 200, &bytes);
    }
    assert_eq!(shown_value(&cache, START_MS + 200), Some((Some(NEW_LUX), 100)), "the older paged value lost");
    for bytes in pages(MAC, 3, &elements("hall", lux(OLD_LUX, 10)), 3) {
        take(&mut cache, MAC, START_MS + 300, &bytes);
    }
    assert_eq!(shown_value(&cache, START_MS + 300), Some((Some(OLD_LUX), 10)), "a value heard later than the event wins");
}

#[test]
fn an_event_from_a_new_board_heard_while_its_first_set_is_staged_reaches_that_set() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    let mut staged = elements("hall", lux(OLD_LUX, 0));
    staged.push(instance(EXTRA, None));
    let next = pages(NEW_MAC, 1, &staged, 3);
    assert_eq!(next.len(), 2);
    take(&mut cache, NEW_MAC, START_MS + 10, &next[0]);
    assert_eq!(cache.hear_event(arrival(NEW_MAC, START_MS + 20), 9, &event(NEW_LUX)), InputHeard::Applied);
    assert_eq!(shown_value(&cache, START_MS + 20).map(|(value, _)| value), Some(Some(OLD_LUX)), "the old board's set is left alone");
    take(&mut cache, NEW_MAC, START_MS + 30, &next[1]);
    assert_eq!(shown_value(&cache, START_MS + 30), Some((Some(NEW_LUX), 10)));
}

#[test]
fn an_input_event_older_than_one_applied_from_the_same_run_is_refused() {
    let mut cache = placed_input_cache();
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(OLD_LUX)), InputHeard::Applied);
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 9, &event(OLD_LUX)), InputHeard::Older);
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(OLD_LUX)), InputHeard::Older, "the same frame twice");
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 11, &event(OLD_LUX)), InputHeard::Applied);
    let restarted = InputArrival { epoch: EPOCH + 1, ..arrival(MAC, START_MS) };
    assert_eq!(cache.hear_event(restarted, 1, &event(OLD_LUX)), InputHeard::Applied, "a new run starts afresh");
    assert_eq!(cache.hear_event(arrival(NEW_MAC, START_MS), 1, &event(OLD_LUX)), InputHeard::Applied, "so does a new board");
}

#[test]
fn a_new_board_on_the_node_number_replaces_the_old_board_s_inputs() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 40, &elements("hall", None), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    for bytes in pages(NEW_MAC, 0, &elements("porch", None), 3) {
        assert!(!take(&mut cache, NEW_MAC, START_MS, &bytes).stale, "a first set from a new board is taken");
    }
    let shown = cache.node(NODE).and_then(|inputs| inputs.shown).map(|shown| shown.id.mac);
    assert_eq!(shown, Some(NEW_MAC));
    cache.hear_event(arrival(MAC, START_MS), 1, &event(NEW_LUX));
    assert_eq!(shown_value(&cache, START_MS), None, "the old board's event does not reach the new board's set");
}

#[test]
fn forgetting_a_node_drops_its_inputs_and_clearing_drops_every_node() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", None), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    let before = cache.changes();
    cache.forget(NODE);
    assert!(cache.node(NODE).is_none());
    assert_ne!(cache.changes(), before);
    for bytes in pages(MAC, 2, &elements("hall", None), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    cache.clear();
    assert!(cache.node(NODE).is_none());
}

#[test]
fn a_node_heard_long_ago_is_held_but_no_longer_fresh() {
    let inputs = ClusterInputs::new(TIMING);
    let stale_after_ms = TIMING.info_refresh_ms() * STALE_REFRESH_PERIODS;
    let long_ago = monotonic_ms().wrapping_sub(stale_after_ms + 1_000);
    for bytes in pages(MAC, 1, &elements("hall", None), 3) {
        let Frame::InputInfo(page) = decode(&bytes).expect("decodes").frame else { panic!("a page") };
        inputs.take_page(arrival(MAC, long_ago), &page, inputs.era(NODE));
    }
    let held = inputs.inputs_held(NODE).expect("still held");
    assert!(!held.fresh);
    assert_eq!(held.devices, 1);
    assert_eq!(inputs.cached_input_device(NODE, 0).map(|device| device.device), Some(4), "its devices stay listed");
}

#[test]
fn an_event_or_page_heard_before_a_clear_does_not_land_after_it() {
    let inputs = ClusterInputs::new(TIMING);
    let era = inputs.era(NODE);
    inputs.clear();
    assert_eq!(inputs.hear_event(arrival(MAC, START_MS), 1, &event(NEW_LUX), era), None);
    for bytes in pages(MAC, 1, &elements("hall", None), 3) {
        let Frame::InputInfo(page) = decode(&bytes).expect("decodes").frame else { panic!("a page") };
        inputs.take_page(arrival(MAC, START_MS), &page, era);
    }
    assert!(inputs.inputs_held(NODE).is_none());
}

fn take_run(cache: &mut InputCache, epoch: u32, now_ms: u32, bytes: &[u8]) -> InputTally {
    let Frame::InputInfo(page) = decode(bytes).expect("decodes").frame else { panic!("an input info page") };
    cache.take_page(InputArrival { epoch, ..arrival(MAC, now_ms) }, &page)
}

#[test]
fn an_event_heard_before_its_device_s_page_of_a_first_set_outlives_the_older_paged_value() {
    let mut cache = placed_input_cache();
    let paged = pages(MAC, 1, &elements("hall", lux(OLD_LUX, 50)), 1);
    assert_eq!(paged.len(), 3, "the owner, the device and the sensor each on a page");
    take(&mut cache, MAC, START_MS, &paged[0]);
    assert_eq!(cache.hear_event(arrival(MAC, START_MS + 10), 5, &event(NEW_LUX)), InputHeard::Applied);
    take(&mut cache, MAC, START_MS + 20, &paged[1]);
    assert!(take(&mut cache, MAC, START_MS + 30, &paged[2]).taken);
    assert_eq!(shown_value(&cache, START_MS + 30), Some((Some(NEW_LUX), 20)), "the press outlives the snapshot taken before it");
}

#[test]
fn an_event_replayed_into_a_set_loses_to_a_value_the_set_heard_later() {
    let mut cache = placed_input_cache();
    let paged = pages(MAC, 1, &elements("hall", lux(OLD_LUX, 5)), 1);
    take(&mut cache, MAC, START_MS, &paged[0]);
    cache.hear_event(arrival(MAC, START_MS + 10), 5, &event(NEW_LUX));
    take(&mut cache, MAC, START_MS + 20, &paged[1]);
    take(&mut cache, MAC, START_MS + 30, &paged[2]);
    assert_eq!(shown_value(&cache, START_MS + 30), Some((Some(OLD_LUX), 5)));
}

#[test]
fn a_rebooted_board_s_set_does_not_inherit_the_values_of_its_previous_run() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    assert_eq!(shown_value(&cache, START_MS).map(|(value, _)| value), Some(Some(OLD_LUX)));
    for bytes in pages(MAC, 0, &elements("hall", None), 3) {
        assert!(take_run(&mut cache, EPOCH + 1, START_MS + 100, &bytes).taken);
    }
    assert_eq!(shown_value(&cache, START_MS + 100), None, "the new run holds no value yet");
}

#[test]
fn a_pulse_of_a_new_boot_drops_the_board_s_inputs_and_another_board_s_pulse_does_not() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    let before = cache.changes();
    cache.note_run(arrival(MAC, START_MS));
    cache.note_run(InputArrival { epoch: EPOCH + 1, ..arrival(NEW_MAC, START_MS) });
    assert!(cache.node(NODE).is_some(), "the same run, or another board, keeps the inputs");
    assert_eq!(cache.changes(), before);
    cache.note_run(InputArrival { epoch: EPOCH + 1, ..arrival(MAC, START_MS) });
    assert!(cache.node(NODE).is_none(), "the board booted again, so what it said before is gone");
    assert_ne!(cache.changes(), before);
}

#[test]
fn a_set_an_earlier_run_was_staging_is_dropped_by_the_new_boot_even_with_nothing_shown() {
    let mut cache = placed_input_cache();
    let paged = pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 1);
    take(&mut cache, MAC, START_MS, &paged[0]);
    cache.note_run(InputArrival { epoch: EPOCH + 1, ..arrival(MAC, START_MS) });
    take(&mut cache, MAC, START_MS + 10, &paged[1]);
    assert!(!take(&mut cache, MAC, START_MS + 20, &paged[2]).taken, "the earlier run's set cannot close after the reboot");
    assert!(cache.node(NODE).is_none());
}

#[test]
fn what_the_new_run_said_before_its_first_pulse_survives_that_pulse() {
    let mut cache = placed_input_cache();
    for bytes in pages(MAC, 1, &elements("hall", lux(OLD_LUX, 0)), 3) {
        take(&mut cache, MAC, START_MS, &bytes);
    }
    let rebooted = InputArrival { epoch: EPOCH + 1, ..arrival(MAC, START_MS + 10) };
    assert_eq!(cache.hear_event(rebooted, 1, &event(NEW_LUX)), InputHeard::Applied);
    let paged = pages(MAC, 0, &elements("hall", None), 1);
    take_run(&mut cache, EPOCH + 1, START_MS + 20, &paged[0]);
    cache.note_run(rebooted);
    take_run(&mut cache, EPOCH + 1, START_MS + 30, &paged[1]);
    assert!(take_run(&mut cache, EPOCH + 1, START_MS + 40, &paged[2]).taken, "the set staged before the pulse completes");
    assert_eq!(shown_value(&cache, START_MS + 40), Some((Some(NEW_LUX), 30)), "and keeps the new run's press");
}

#[test]
fn forgetting_one_node_does_not_drop_an_event_another_node_is_hearing() {
    let inputs = ClusterInputs::new(TIMING);
    let era = inputs.era(NODE);
    inputs.forget(NODE + 1);
    assert_eq!(inputs.hear_event(arrival(MAC, START_MS), 1, &event(NEW_LUX), era), Some(InputHeard::Applied));
}

#[test]
fn an_event_applied_before_a_forget_or_a_clear_is_still_refused_after_it() {
    let mut cache = placed_input_cache();
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(NEW_LUX)), InputHeard::Applied);
    cache.forget(NODE);
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(NEW_LUX)), InputHeard::Older, "its copy after a forget");
    cache.clear();
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(NEW_LUX)), InputHeard::Older, "its copy after a clear");
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 11, &event(NEW_LUX)), InputHeard::Applied);
}

#[test]
fn an_event_heard_before_a_forget_is_not_replayed_into_the_next_set() {
    let mut cache = placed_input_cache();
    assert_eq!(cache.hear_event(arrival(MAC, START_MS), 10, &event(NEW_LUX)), InputHeard::Applied);
    cache.forget(NODE);
    for bytes in pages(MAC, 1, &elements("hall", None), 3) {
        take(&mut cache, MAC, START_MS + 10, &bytes);
    }
    assert_eq!(shown_value(&cache, START_MS + 10), None, "the set after a forget is the sender's own");
}

#[test]
fn the_cache_of_seven_neighbours_weighs_what_adr_035_states() {
    let per_node = size_of::<NodeInputs>();
    eprintln!("input cache: {per_node} B per node, {} B for seven neighbours, {} B placed", per_node * NODES_BESIDE_THIS_ONE, size_of::<InputCache>());
    assert!(size_of::<InputCache>() <= INPUT_CACHE_CEILING_BYTES);
}
