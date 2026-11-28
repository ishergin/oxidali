use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use dali2rust_contracts::msg::{ColorMode, DeviceType};

use crate::frame::{
    cluster_tag, encode, AdapterScope, DatagramBuffer, Header, InfoPageWriter, InputDevice, InputInfoElement,
    InputInfoWriter, InputOwner, LampCapabilities, LampInfo, LampInfoElement, LampInfoWriter, NodeClaim, PagePlace, Pulse,
};
use crate::inputs::ClusterInputsReadPort;
use crate::lamps::ClusterLampsReadPort;
use crate::runtime::fixed_boot_epoch;

const TIMING: PresenceTiming = PresenceTiming::every_ms(1_000);
const NAME: &str = "home";
const HOUSE: &str = "dali-house";
const BARN: &str = "dali-barn";
const OWN_MAC: [u8; 6] = [2, 0, 0, 0, 0, 5];
const NEIGHBOUR_MAC: [u8; 6] = [2, 0, 0, 0, 0, 9];
const SOURCE: [u8; 4] = [10, 0, 0, 9];
const START_MS: u32 = 100_000;
const STEP_MS: u32 = 1 << 28;
const STEPS_PAST_A_WRAP: u32 = 20;
const DRAWN_EPOCH: u32 = 0x0e90_c400;

fn identity(node: u8) -> OwnFacts {
    OwnFacts {
        cluster_name: fixed_text_64(NAME),
        cluster_tag: cluster_tag(NAME),
        installation: fixed_text_32(HOUSE),
        mac: OWN_MAC,
        node_no: node,
        wire: node,
        wire_protection: true,
        forwards_foreign: false,
        running_since_ms: None,
    }
}

fn pulse(cluster_name: &str, installation: &str, node: u8, seq: u32) -> Vec<u8> {
    let header =
        Header { cluster_tag: cluster_tag(cluster_name), sender_mac: NEIGHBOUR_MAC, boot_epoch: 7, node_no: node, seq };
    let frame = Frame::Pulse(Pulse { cluster_name, installation, claim: NodeClaim::Holding, wire: node, wire_protection: true });
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(&header, &frame, &mut buffer).expect("a pulse fits");
    buffer[..len].to_vec()
}

fn info_page(cluster_name: &str, node: u8, seq: u32) -> Vec<u8> {
    info_page_from(NEIGHBOUR_MAC, cluster_name, node, seq)
}

fn info_page_from(sender_mac: [u8; 6], cluster_name: &str, node: u8, seq: u32) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(cluster_name), sender_mac, boot_epoch: 7, node_no: node, seq };
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = InfoPageWriter::start(&mut buffer, &header, seq, 0).expect("a page starts").finish(true);
    buffer[..len].to_vec()
}

fn present() -> (ClusterPresence, ClusterCounters) {
    let presence = ClusterPresence::new(TIMING, fixed_boot_epoch(1));
    presence.settle(identity(0), Some(OwnMode::Standby), 0, START_MS);
    (presence, ClusterCounters::default())
}

fn deliver(presence: &ClusterPresence, counters: &ClusterCounters, bytes: &[u8], link: u32, now_ms: u32) {
    let (info, lamps, inputs) = (ClusterInfo::new(), ClusterLamps::new(TIMING), ClusterInputs::new(TIMING));
    presence.take_datagram(bytes, Delivery { source: SOURCE, link, now_ms }, counters, Caches { info: &info, lamps: &lamps, inputs: &inputs });
}

fn boards(presence: &ClusterPresence) -> usize {
    presence.lock().table.iter().count()
}

fn stepped(step: u32) -> u32 {
    START_MS.wrapping_add(step.wrapping_mul(STEP_MS))
}

#[test]
fn a_datagram_read_from_a_link_already_given_up_is_not_taken() {
    let (presence, counters) = present();
    let given_up = presence.link();
    presence.clear();
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 1), given_up, START_MS);
    assert_eq!(boards(&presence), 0);
    assert_eq!(counters.stale_link_frames_total.load(Ordering::Relaxed), 1);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 2), presence.link(), START_MS);
    assert_eq!(boards(&presence), 1);
    assert_eq!(counters.stale_link_frames_total.load(Ordering::Relaxed), 1);
}

#[test]
fn a_board_stamped_after_the_judging_clock_was_read_is_live() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &pulse("elsewhere", BARN, 1, 1), presence.link(), START_MS + 1);
    presence.prune(START_MS);
    assert_eq!(boards(&presence), 1, "a live ignored board is kept");
    deliver(&presence, &counters, &pulse(NAME, BARN, 2, 2), presence.link(), START_MS + 1);
    assert_eq!(presence.forget(2, START_MS), Forgotten::StillHeard);
}

#[test]
fn a_kept_silent_holder_never_comes_back_as_heard_when_the_clock_wraps() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &pulse(NAME, BARN, 2, 1), presence.link(), START_MS);
    for step in 1..=STEPS_PAST_A_WRAP {
        let now = stepped(step);
        presence.prune(now);
        assert_eq!(boards(&presence), 1, "a silent holder is kept, step {step}");
        let state = presence.lock();
        assert!(!state.roster(now, TIMING).heard(2), "step {step}");
    }
    assert_eq!(presence.forget(2, stepped(STEPS_PAST_A_WRAP)), Forgotten::Removed);
}

#[test]
fn a_node_that_has_run_longer_than_half_the_clock_still_sees_its_split_pair() {
    let presence = ClusterPresence::new(TIMING, fixed_boot_epoch(1));
    let counters = ClusterCounters::default();
    assert!(presence.settle(identity(0), None, TIMING.listen_ms(), START_MS).entered_running);
    for step in 1..=STEPS_PAST_A_WRAP {
        let now = stepped(step);
        presence.settle(identity(0), None, TIMING.listen_ms(), now);
        presence.prune(now);
        deliver(&presence, &counters, &pulse(NAME, HOUSE, 0, step), presence.link(), now);
        let state = presence.lock();
        assert!(state.roster(now, TIMING).faults().split_pair, "step {step}");
    }
}

#[test]
fn the_boot_epoch_is_drawn_once_and_not_before_it_is_asked_for() {
    let draws = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&draws);
    let presence =
        ClusterPresence::new(TIMING, Arc::new(move || DRAWN_EPOCH + counted.fetch_add(1, Ordering::Relaxed)));
    assert_eq!(presence.own_status().boot_epoch, None);
    assert_eq!(draws.load(Ordering::Relaxed), 0);
    assert_eq!(presence.draw_boot_epoch(), DRAWN_EPOCH);
    assert_eq!(presence.draw_boot_epoch(), DRAWN_EPOCH);
    assert_eq!(presence.own_status().boot_epoch, Some(DRAWN_EPOCH));
    assert_eq!(draws.load(Ordering::Relaxed), 1);
}

#[test]
fn a_late_page_of_a_member_is_stale_and_a_page_of_another_cluster_is_ignored() {
    let (presence, counters) = present();
    let (info, inputs) = (ClusterInfo::new(), ClusterInputs::new(TIMING));
    let take = |bytes: &[u8]| {
        let caches = Caches { info: &info, lamps: &ClusterLamps::new(TIMING), inputs: &inputs };
        presence.take_datagram(bytes, Delivery { source: SOURCE, link: 0, now_ms: START_MS }, &counters, caches);
    };
    let read = |counter: &AtomicU32| counter.load(Ordering::Relaxed);
    take(&pulse(NAME, BARN, 1, 10));
    take(&pulse(NAME, BARN, 1, 15));
    take(&info_page(NAME, 1, 16));
    assert_eq!(read(&counters.info_sets_taken_total), 1);
    take(&info_page(NAME, 1, 12));
    assert_eq!(read(&counters.info_pages_stale_total), 1);
    assert_eq!(read(&counters.info_pages_ignored_total), 0);
    take(&info_page("garage", 1, 17));
    assert_eq!(read(&counters.info_pages_ignored_total), 1);
    assert_eq!(read(&counters.info_sets_taken_total), 1);
}


#[test]
fn this_node_s_own_page_heard_back_is_dropped_and_not_counted_as_ignored() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &info_page_from(OWN_MAC, NAME, 0, 1), 0, START_MS);
    assert_eq!(counters.own_frames_dropped_total.load(Ordering::Relaxed), 1);
    assert_eq!(counters.info_pages_ignored_total.load(Ordering::Relaxed), 0);
    deliver(&presence, &counters, &info_page_from(NEIGHBOUR_MAC, NAME, 1, 1), 0, START_MS);
    assert_eq!(counters.info_pages_ignored_total.load(Ordering::Relaxed), 1);
}

fn command_from(sender_mac: [u8; 6], cluster_name: &str, node: u8, seq: u32) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(cluster_name), sender_mac, boot_epoch: 7, node_no: node, seq };
    let mark = crate::frame::BirthMark { generation: 1, nonce: 2 };
    let scene = crate::frame::RecordRef { number: 3, mark };
    let action = crate::frame::Action::RecallScene { scene, scope: crate::frame::RecallScope::Node };
    let source = crate::frame::SourceClass::Operator;
    let frame = Frame::Command(Command { action, source, hold_hcl: true, executed_on_wire: None });
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(&header, &frame, &mut buffer).expect("a command fits");
    buffer[..len].to_vec()
}

fn heard(presence: &ClusterPresence, counters: &ClusterCounters, bytes: &[u8]) -> Option<CommandAdmission> {
    let delivery = Delivery { source: SOURCE, link: presence.link(), now_ms: START_MS };
    let (info, lamps, inputs) = (ClusterInfo::new(), ClusterLamps::new(TIMING), ClusterInputs::new(TIMING));
    match presence.take_datagram(bytes, delivery, counters, Caches { info: &info, lamps: &lamps, inputs: &inputs })? {
        Arrived::Command(heard) => Some(heard.admission),
        Arrived::Input(_) => None,
    }
}

#[test]
fn a_member_s_command_is_taken_once_and_its_copy_is_a_duplicate() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    let command = command_from(NEIGHBOUR_MAC, NAME, 1, 11);
    let standby = CommandStanding { running: false, node: 0, owns_wire: false, wire: 0 };
    assert_eq!(heard(&presence, &counters, &command), Some(CommandAdmission::Taken(standby)));
    assert_eq!(heard(&presence, &counters, &command), Some(CommandAdmission::Duplicate));
}

#[test]
fn a_member_s_command_that_arrives_late_is_still_taken() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 12), presence.link(), START_MS);
    let standby = CommandStanding { running: false, node: 0, owns_wire: false, wire: 0 };
    let late = command_from(NEIGHBOUR_MAC, NAME, 1, 11);
    assert_eq!(heard(&presence, &counters, &late), Some(CommandAdmission::Taken(standby)));
    assert_eq!(heard(&presence, &counters, &late), Some(CommandAdmission::Duplicate));
}

#[test]
fn a_command_from_another_cluster_or_an_unheard_board_is_not_a_member_s_and_this_node_s_own_is_never_heard() {
    let (presence, counters) = present();
    deliver(&presence, &counters, &pulse("garage", BARN, 1, 10), presence.link(), START_MS);
    let foreign = command_from(NEIGHBOUR_MAC, "garage", 1, 11);
    assert_eq!(heard(&presence, &counters, &foreign), Some(CommandAdmission::NotMember));
    let unheard = command_from([2, 0, 0, 0, 0, 7], NAME, 2, 1);
    assert_eq!(heard(&presence, &counters, &unheard), Some(CommandAdmission::NotMember));
    assert_eq!(heard(&presence, &counters, &command_from(OWN_MAC, NAME, 0, 1)), None);
}

#[test]
fn a_running_node_that_owns_its_wire_stands_to_execute_a_member_s_command() {
    let presence = ClusterPresence::new(TIMING, fixed_boot_epoch(1));
    let counters = ClusterCounters::default();
    assert!(presence.settle(identity(0), None, TIMING.listen_ms(), START_MS).entered_running);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    let running = CommandStanding { running: true, node: 0, owns_wire: true, wire: 0 };
    let admission = heard(&presence, &counters, &command_from(NEIGHBOUR_MAC, NAME, 1, 11));
    assert_eq!(admission, Some(CommandAdmission::Taken(running)));
}

fn lamp_info_page(node: u8, boot_epoch: u32, seq: u32) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(NAME), sender_mac: NEIGHBOUR_MAC, boot_epoch, node_no: node, seq };
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: AdapterScope { adapter: 0 }, set: 1, page: 0 };
    let mut writer = LampInfoWriter::open(&mut buffer, &header, place).expect("a page opens");
    let lamp = LampInfo {
        lamp: 1,
        name: "Hall",
        device_type: DeviceType::Dt6Led,
        device_type_overridden: false,
        device_types: 0,
        color_mode: ColorMode::Brightness,
        color_mode_overridden: false,
        capabilities: LampCapabilities::default(),
        kelvin_range: None,
        bound: true,
        ha_entity: true,
    };
    writer.push(&LampInfoElement::Lamp(lamp)).expect("the lamp fits");
    let len = writer.finish(true);
    buffer[..len].to_vec()
}

fn pulse_of_epoch(node: u8, boot_epoch: u32, seq: u32) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(NAME), sender_mac: NEIGHBOUR_MAC, boot_epoch, node_no: node, seq };
    let frame = Frame::Pulse(Pulse { cluster_name: NAME, installation: BARN, claim: NodeClaim::Holding, wire: node, wire_protection: true });
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(&header, &frame, &mut buffer).expect("a pulse fits");
    buffer[..len].to_vec()
}

#[test]
fn a_neighbour_s_first_pulse_after_a_reboot_drops_its_cached_lamps() {
    let (presence, counters) = present();
    let lamps = ClusterLamps::new(TIMING);
    let take = |bytes: &[u8]| {
        let delivery = Delivery { source: SOURCE, link: 0, now_ms: START_MS };
        presence.take_datagram(bytes, delivery, &counters, Caches { info: &ClusterInfo::new(), lamps: &lamps, inputs: &ClusterInputs::new(TIMING) });
    };
    take(&pulse_of_epoch(1, 7, 1));
    take(&lamp_info_page(1, 7, 2));
    take(&pulse_of_epoch(1, 7, 3));
    assert_eq!(lamps.cached_lamp_ids(1, 0), 1 << 1, "the board's lamp is held across its pulses");
    take(&pulse_of_epoch(1, 8, 0));
    assert_eq!(lamps.cached_lamp_ids(1, 0), 0, "the board booted again, so its old lamps are gone");
    take(&lamp_info_page(1, 8, 1));
    take(&pulse_of_epoch(1, 7, 4));
    assert_eq!(lamps.cached_lamp_ids(1, 0), 1 << 1, "a late pulse from before the reboot leaves the new run alone");
}

fn input_info_page(node: u8, boot_epoch: u32, seq: u32) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(NAME), sender_mac: NEIGHBOUR_MAC, boot_epoch, node_no: node, seq };
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let place = PagePlace { scope: (), set: 1, page: 0 };
    let mut writer = InputInfoWriter::open(&mut buffer, &header, place).expect("a page opens");
    let owner = InputInfoElement::Owner(InputOwner { controller_id: "hall", discovery_prefix: "homeassistant" });
    let device = InputDevice { adapter: 0, device: 4, name: "Hall", ha_entity: true, present: true, instances: 0 };
    writer.push(&owner).expect("the owner fits");
    writer.push(&InputInfoElement::Device(device)).expect("the device fits");
    let len = writer.finish(true);
    buffer[..len].to_vec()
}

#[test]
fn a_neighbour_s_first_pulse_after_a_reboot_drops_its_cached_inputs() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    let take = |bytes: &[u8]| {
        let delivery = Delivery { source: SOURCE, link: 0, now_ms: START_MS };
        let caches = Caches { info: &ClusterInfo::new(), lamps: &ClusterLamps::new(TIMING), inputs: &inputs };
        presence.take_datagram(bytes, delivery, &counters, caches);
    };
    take(&pulse_of_epoch(1, 7, 1));
    take(&input_info_page(1, 7, 2));
    take(&pulse_of_epoch(1, 7, 3));
    assert!(inputs.cached_input_device(1, 0).is_some(), "the board's device is held across its pulses");
    take(&pulse_of_epoch(1, 8, 0));
    assert!(inputs.inputs_held(1).is_none(), "the board booted again, so its old devices are gone");
    take(&input_info_page(1, 8, 1));
    take(&pulse_of_epoch(1, 7, 4));
    assert!(inputs.cached_input_device(1, 0).is_some(), "a late pulse from before the reboot leaves the new run alone");
}

#[test]
fn a_neighbour_s_new_boot_heard_first_in_another_frame_still_drops_its_cached_inputs() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    let take = |bytes: &[u8]| {
        let delivery = Delivery { source: SOURCE, link: 0, now_ms: START_MS };
        let caches = Caches { info: &ClusterInfo::new(), lamps: &ClusterLamps::new(TIMING), inputs: &inputs };
        presence.take_datagram(bytes, delivery, &counters, caches);
    };
    take(&pulse_of_epoch(1, 7, 1));
    take(&input_info_page(1, 7, 2));
    assert!(inputs.inputs_held(1).is_some());
    take(&lamp_info_page(1, 8, 0));
    take(&pulse_of_epoch(1, 8, 1));
    assert!(inputs.inputs_held(1).is_none(), "the new boot came first in a lamp page, and the pulse after it is no restart");
}

const PRESS: u16 = 1;
const RELEASE: u16 = 0;

fn input_from(sender_mac: [u8; 6], cluster_name: &str, node: u8, seq: u32, event_info: u16) -> Vec<u8> {
    let header = Header { cluster_tag: cluster_tag(cluster_name), sender_mac, boot_epoch: 7, node_no: node, seq };
    let event = InputEvent {
        address: crate::frame::InputAddress { adapter: 0, device: 3, instance: Some(0) },
        instance_type: Some(1),
        groups: [None; 3],
        scheme: 1,
        kind: dali2rust_contracts::msg::InputEventKind::Button,
        event_info,
        value: 0,
    };
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let len = encode(&header, &Frame::InputEvent(event), &mut buffer).expect("an input event fits");
    buffer[..len].to_vec()
}

fn input_heard(presence: &ClusterPresence, counters: &ClusterCounters, inputs: &ClusterInputs, bytes: &[u8]) -> Option<u16> {
    let delivery = Delivery { source: SOURCE, link: presence.link(), now_ms: START_MS };
    let info = ClusterInfo::new();
    match presence.take_datagram(bytes, delivery, counters, Caches { info: &info, lamps: &ClusterLamps::new(TIMING), inputs })? {
        Arrived::Input(heard) => Some(heard.event.event_info),
        Arrived::Command(_) => None,
    }
}

fn dropped(counters: &ClusterCounters) -> u32 {
    counters.input_events_dropped_total.load(Ordering::Relaxed)
}

#[test]
fn a_member_s_press_arrives_once_and_its_copy_does_nothing() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    let press = input_from(NEIGHBOUR_MAC, NAME, 1, 11, PRESS);
    assert_eq!(input_heard(&presence, &counters, &inputs, &press), Some(PRESS));
    assert_eq!(input_heard(&presence, &counters, &inputs, &press), None);
    assert_eq!(dropped(&counters), 0, "a copy is not a dropped event");
}

#[test]
fn a_press_whose_first_copy_was_lost_arrives_from_its_second() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 12), presence.link(), START_MS);
    let second_copy = input_from(NEIGHBOUR_MAC, NAME, 1, 11, PRESS);
    assert_eq!(input_heard(&presence, &counters, &inputs, &second_copy), Some(PRESS));
}

#[test]
fn a_press_arriving_after_its_own_release_is_dropped_and_counted() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    deliver(&presence, &counters, &pulse(NAME, BARN, 1, 10), presence.link(), START_MS);
    assert_eq!(input_heard(&presence, &counters, &inputs, &input_from(NEIGHBOUR_MAC, NAME, 1, 12, RELEASE)), Some(RELEASE));
    assert_eq!(input_heard(&presence, &counters, &inputs, &input_from(NEIGHBOUR_MAC, NAME, 1, 11, PRESS)), None);
    assert_eq!(dropped(&counters), 1);
}

#[test]
fn an_input_event_from_another_cluster_an_unheard_board_or_this_node_does_nothing() {
    let (presence, counters) = present();
    let inputs = ClusterInputs::new(TIMING);
    deliver(&presence, &counters, &pulse("garage", BARN, 1, 10), presence.link(), START_MS);
    assert_eq!(input_heard(&presence, &counters, &inputs, &input_from(NEIGHBOUR_MAC, "garage", 1, 11, PRESS)), None);
    assert_eq!(input_heard(&presence, &counters, &inputs, &input_from([2, 0, 0, 0, 0, 7], NAME, 2, 1, PRESS)), None);
    assert_eq!(input_heard(&presence, &counters, &inputs, &input_from(OWN_MAC, NAME, 0, 1, PRESS)), None);
    assert_eq!(dropped(&counters), 0);
}
