use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{
    cluster_block, ClusterCopyReadPort, ClusterPartKeys, ClusterPartReadPort, ClusterPartView, ClusterRecordNumbers,
    ClusterRecordReadPort, ClusterRecordView,
};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{cluster_tag, BirthMark};

const TIMING: PresenceTiming = PresenceTiming::every_ms(160);
const QUICK: PresenceTiming = PresenceTiming::every_ms(16);
const LONG_NAME: &str = "A channel whose name fills most of the sixty-four bytes it may";
const MARK: BirthMark = BirthMark { generation: 1, nonce: 0xbeef };
const ALL_NUMBERS: u16 = u16::MAX;
const WATCH_MS: u32 = 10_000;

struct FullBlock {
    node: u8,
}

impl ClusterRecordReadPort for FullBlock {
    fn cluster_record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let home_node = self.node;
        Some(ClusterRecordView { kind, number, home_node, mark: MARK, name: LONG_NAME.to_owned(), revision: 1 })
    }

    fn cluster_record_numbers(&self, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        ClusterRecordNumbers { first: *cluster_block(self.node).start(), live: ALL_NUMBERS }
    }

    fn cluster_generation(&self, _kind: ClusterRecordKind, _number: u8) -> u32 {
        MARK.generation
    }
}

impl ClusterPartReadPort for FullBlock {
    fn cluster_part(&self, _kind: ClusterRecordKind, _adapter: u8, _local: u8) -> Option<ClusterPartView> {
        None
    }

    fn cluster_part_keys(&self, _kind: ClusterRecordKind) -> ClusterPartKeys {
        ClusterPartKeys::default()
    }
}

impl ClusterCopyReadPort for FullBlock {
    fn cluster_copy(&self, _kind: ClusterRecordKind, _record: crate::frame::RecordRef) -> Option<ClusterRecordView> {
        None
    }

    fn cluster_copy_at(&self, _kind: ClusterRecordKind, _index: usize) -> Option<ClusterRecordView> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sent {
    at_ms: u32,
    closes_set: bool,
}

struct Rig {
    timing: PresenceTiming,
    announcer: Announcer,
    block: FullBlock,
    buffer: DatagramBuffer,
    sent: Vec<Sent>,
}

impl Rig {
    fn new(node: u8) -> Self {
        Self::timed(node, TIMING)
    }

    fn timed(node: u8, timing: PresenceTiming) -> Self {
        let buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        Self { timing, announcer: Announcer::new(), block: FullBlock { node }, buffer, sent: Vec::new() }
    }

    fn turn(&mut self, now_ms: u32, running: bool, hello: bool) {
        let node = self.block.node;
        let turn = Turn { running, node, now_ms, hello, timing: self.timing };
        let own = OwnInfo { records: &self.block, parts: &self.block, copies: &self.block };
        self.announcer.plan(turn, own);
        if self.announcer.page_due(now_ms) {
            let tag = cluster_tag("hall");
            let header = Header { cluster_tag: tag, sender_mac: [2; 6], boot_epoch: 1, node_no: node, seq: 0 };
            self.announcer.write_page(&mut self.buffer, &header, now_ms).expect("a page is written");
            self.sent.push(Sent { at_ms: now_ms, closes_set: !self.announcer.paging() });
        }
    }

    fn run(&mut self, from_ms: u32, to_ms: u32, hellos: &[u32]) {
        for now_ms in from_ms..to_ms {
            self.turn(now_ms, true, hellos.contains(&now_ms));
        }
    }

    fn set_starts(&self) -> Vec<u32> {
        let mut opens = true;
        let mut starts = Vec::new();
        for sent in &self.sent {
            if opens {
                starts.push(sent.at_ms);
            }
            opens = sent.closes_set;
        }
        starts
    }
}

#[test]
fn the_first_set_goes_out_once_the_coalescing_window_has_passed() {
    let mut rig = Rig::new(0);
    rig.run(0, TIMING.info_coalesce_ms(), &[]);
    assert!(rig.sent.is_empty());
    rig.run(TIMING.info_coalesce_ms(), TIMING.info_coalesce_ms() + 1, &[]);
    assert_eq!(rig.set_starts(), vec![TIMING.info_coalesce_ms()]);
}

#[test]
fn the_pages_of_one_set_go_out_a_page_gap_apart() {
    let mut rig = Rig::new(0);
    rig.run(0, 200, &[]);
    let set: Vec<u32> = rig.sent.iter().map(|sent| sent.at_ms).collect();
    assert!(set.len() > 1, "the full block takes more than one page: {set:?}");
    assert!(set.windows(2).all(|pair| pair[1] - pair[0] == PAGE_GAP_MS), "{set:?}");
    assert_eq!(rig.sent.iter().filter(|sent| sent.closes_set).count(), 1);
}

#[test]
fn while_a_set_goes_out_the_next_deadline_is_its_next_page_not_a_due_set() {
    let mut rig = Rig::timed(0, QUICK);
    let first_ms = QUICK.info_coalesce_ms();
    rig.run(0, first_ms + 1, &[]);
    assert!(rig.announcer.paging());
    rig.turn(first_ms + 1, true, true);
    let past_the_hello_ms = first_ms + 1 + QUICK.info_coalesce_ms() + 1;
    assert!(past_the_hello_ms < first_ms + PAGE_GAP_MS);
    rig.turn(past_the_hello_ms, true, false);
    assert_eq!(rig.announcer.deadline(), Some(first_ms + PAGE_GAP_MS));
}

#[test]
fn any_number_of_hellos_get_one_answer_after_the_node_s_own_delay() {
    let node = 3;
    let mut rig = Rig::new(node);
    rig.run(0, 400, &[]);
    let before = rig.set_starts().len();
    rig.run(400, 1_000, &[400, 401, 405, 450]);
    let answers: Vec<u32> = rig.set_starts().into_iter().skip(before).collect();
    assert_eq!(answers, vec![400 + hello_delay_ms_for(node)]);
}

#[test]
fn the_periodic_refresh_is_shifted_by_the_node_number() {
    let period = TIMING.info_refresh_ms();
    let shift = period / u32::from(NODE_COUNT);
    for node in [0_u8, 3] {
        let mut rig = Rig::new(node);
        rig.run(0, period * 2 + shift, &[]);
        let refreshes: Vec<u32> = rig.set_starts().into_iter().skip(1).collect();
        let first = if node == 0 { period } else { shift * u32::from(node) };
        assert_eq!(refreshes, vec![first, first + period], "node {node}");
    }
}

#[test]
fn a_node_that_is_not_running_announces_nothing() {
    let mut rig = Rig::new(0);
    for now_ms in 0..WATCH_MS {
        rig.turn(now_ms, false, true);
    }
    assert!(rig.sent.is_empty());
    assert_eq!(rig.announcer.deadline(), None);
}

const fn hello_delay_ms_for(node: u8) -> u32 {
    TIMING.info_coalesce_ms() * (1 + node as u32)
}
