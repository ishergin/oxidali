use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{cluster_tag, encode, DatagramBuffer, Header, InfoPageWriter, NodeClaim, Pulse};
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
    presence.take_datagram(bytes, Delivery { source: SOURCE, link, now_ms }, counters, &ClusterInfo::new());
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
    let info = ClusterInfo::new();
    let take = |bytes: &[u8]| {
        presence.take_datagram(bytes, Delivery { source: SOURCE, link: 0, now_ms: START_MS }, &counters, &info);
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
