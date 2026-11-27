use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64};

use super::*;
use crate::frame::{cluster_tag, Header, Hello, NodeClaim, Pulse};

const PERIOD_MS: u32 = 1_000;
const TIMING: PresenceTiming = PresenceTiming::every_ms(PERIOD_MS);
const NAME: &str = "home";
const HOUSE: &str = "dali-house";
const BARN: &str = "dali-barn";
const SOURCE: [u8; 4] = [10, 0, 0, 9];
const OWN_MAC: [u8; 6] = [0, 0, 0, 0, 0, 5];
const LOW_MAC: [u8; 6] = [0, 0, 0, 0, 0, 1];
const HIGH_MAC: [u8; 6] = [0, 0, 0, 0, 0, 9];
const OTHER_MAC: [u8; 6] = [0, 0, 0, 0, 0, 7];
const EPOCH: u32 = 41;
const START_MS: u32 = 100_000;

struct Board {
    mac: [u8; 6],
    node: u8,
    wire: u8,
    installation: &'static str,
    name: &'static str,
    claim: NodeClaim,
    epoch: u32,
    seq: u32,
}

impl Board {
    fn holding(mac: [u8; 6], node: u8, installation: &'static str) -> Self {
        Self { mac, node, wire: node, installation, name: NAME, claim: NodeClaim::Holding, epoch: EPOCH, seq: 0 }
    }

    fn header(&self) -> Header {
        Header {
            cluster_tag: cluster_tag(self.name),
            sender_mac: self.mac,
            boot_epoch: self.epoch,
            node_no: self.node,
            seq: self.seq,
        }
    }

    fn pulse(&self) -> Pulse<'static> {
        Pulse {
            cluster_name: self.name,
            installation: self.installation,
            claim: self.claim,
            wire: self.wire,
            wire_protection: true,
        }
    }

    fn pulse_into(&mut self, table: &mut BoardTable, now_ms: u32) -> Taken {
        self.seq += 1;
        let header = self.header();
        table.take_pulse(Arriving { header: &header, source: SOURCE, now_ms, timing: TIMING }, &self.pulse())
    }
}

fn own(node: u8, running_since_ms: Option<u32>) -> OwnFacts {
    OwnFacts {
        cluster_name: fixed_text_64(NAME),
        cluster_tag: cluster_tag(NAME),
        installation: fixed_text_32(HOUSE),
        mac: OWN_MAC,
        node_no: node,
        wire: node,
        running_since_ms,
    }
}

fn roster<'a>(table: &'a BoardTable, own: &'a OwnFacts, now_ms: u32) -> Roster<'a> {
    Roster::new(table, own, now_ms, TIMING)
}

fn only(table: &BoardTable) -> &HeardBoard {
    table.iter().next().expect("a board")
}

#[test]
fn a_first_pulse_makes_a_board_and_its_copy_is_repeated() {
    let mut table = BoardTable::default();
    let mut board = Board::holding(OTHER_MAC, 1, BARN);
    assert_eq!(board.pulse_into(&mut table, START_MS), Taken::First { evicted: false });
    let header = board.header();
    let again = Arriving { header: &header, source: SOURCE, now_ms: START_MS, timing: TIMING };
    assert_eq!(table.take_pulse(again, &board.pulse()), Taken::Repeated);
    assert_eq!(table.iter().count(), 1);
}

#[test]
fn a_skipped_sequence_number_is_counted_on_the_board() {
    let mut table = BoardTable::default();
    let mut board = Board::holding(OTHER_MAC, 1, BARN);
    board.pulse_into(&mut table, START_MS);
    board.seq += 2;
    assert_eq!(board.pulse_into(&mut table, START_MS + PERIOD_MS), Taken::Fresh { skipped: 2 });
    assert_eq!(only(&table).sequence_gaps, 2);
}

#[test]
fn a_new_boot_epoch_starts_a_new_run_even_within_the_liveness_window() {
    let mut table = BoardTable::default();
    let mut board = Board::holding(OTHER_MAC, 1, BARN);
    board.pulse_into(&mut table, START_MS);
    board.pulse_into(&mut table, START_MS + PERIOD_MS);
    assert_eq!(only(&table).holding_since_ms, Some(START_MS));
    board.epoch += 1;
    board.seq = 0;
    assert_eq!(board.pulse_into(&mut table, START_MS + 2 * PERIOD_MS), Taken::Restarted);
    assert_eq!(only(&table).holding_since_ms, Some(START_MS + 2 * PERIOD_MS));
}

#[test]
fn a_hello_reaches_only_a_board_already_heard() {
    let mut table = BoardTable::default();
    let mut board = Board::holding(OTHER_MAC, 1, BARN);
    let hello = Hello { ha_face: true };
    assert_eq!(table.take_hello(&board.header(), &hello), Taken::Unknown);
    board.pulse_into(&mut table, START_MS);
    board.seq += 1;
    assert_eq!(table.take_hello(&board.header(), &hello), Taken::Fresh { skipped: 0 });
    assert_eq!(only(&table).ha_face, Some(true));
}

#[test]
fn a_full_table_makes_room_by_dropping_the_board_heard_longest_ago() {
    let mut table = BoardTable::default();
    for index in 0..MAX_HEARD_BOARDS {
        let mac = [1, 0, 0, 0, 0, index as u8];
        Board::holding(mac, 1, BARN).pulse_into(&mut table, START_MS + index as u32);
    }
    let taken = Board::holding(OTHER_MAC, 2, BARN).pulse_into(&mut table, START_MS + PERIOD_MS);
    assert_eq!(taken, Taken::First { evicted: true });
    assert!(table.iter().all(|board| board.mac != [1, 0, 0, 0, 0, 0]));
    assert!(table.iter().any(|board| board.mac == OTHER_MAC));
}

#[test]
fn liveness_holds_across_the_wrap_of_the_millisecond_clock() {
    let mut table = BoardTable::default();
    let heard_at = u32::MAX - PERIOD_MS / 2;
    Board::holding(OTHER_MAC, 1, BARN).pulse_into(&mut table, heard_at);
    let after_wrap = heard_at.wrapping_add(TIMING.liveness_ms());
    assert!(only(&table).is_live(after_wrap, TIMING));
    assert!(!only(&table).is_live(after_wrap.wrapping_add(1), TIMING));
}

#[test]
fn a_board_of_another_cluster_name_or_still_waiting_is_ignored_with_its_reason() {
    let mut table = BoardTable::default();
    let mut stranger = Board::holding(OTHER_MAC, 1, BARN);
    stranger.name = "hoem";
    stranger.pulse_into(&mut table, START_MS);
    let mut shy = Board::holding(HIGH_MAC, 2, BARN);
    shy.claim = NodeClaim::Waiting;
    shy.pulse_into(&mut table, START_MS);
    let facts = own(0, None);
    let reasons: Vec<_> = roster(&table, &facts, START_MS).ignored().map(|(b, r)| (b.mac, r)).collect();
    assert_eq!(
        reasons,
        [(OTHER_MAC, IgnoredReason::OtherCluster), (HIGH_MAC, IgnoredReason::Waiting)]
    );
}

#[test]
fn of_two_installations_holding_one_number_the_lower_mac_is_the_node() {
    let mut table = BoardTable::default();
    Board::holding(HIGH_MAC, 1, BARN).pulse_into(&mut table, START_MS);
    Board::holding(LOW_MAC, 1, "dali-shed").pulse_into(&mut table, START_MS);
    let facts = own(0, None);
    let view = roster(&table, &facts, START_MS);
    assert_eq!(view.displayed(1).map(|board| board.mac), Some(LOW_MAC));
    let ignored: Vec<_> = view.ignored().map(|(b, r)| (b.mac, r)).collect();
    assert_eq!(ignored, [(HIGH_MAC, IgnoredReason::NumberHeld)]);
}

#[test]
fn this_running_node_takes_part_in_who_holds_its_number() {
    let mut table = BoardTable::default();
    Board::holding(HIGH_MAC, 3, BARN).pulse_into(&mut table, START_MS);
    let facts = own(3, Some(START_MS - PERIOD_MS));
    let view = roster(&table, &facts, START_MS);
    assert_eq!(view.standing(only(&table)), Standing::Ignored(IgnoredReason::NumberHeld));
    let waiting = own(3, None);
    assert_eq!(roster(&table, &waiting, START_MS).standing(only(&table)), Standing::Holder);
}

#[test]
fn two_boards_of_one_installation_transmitting_together_are_a_split_pair() {
    let mut table = BoardTable::default();
    let (mut first, mut second) = (Board::holding(LOW_MAC, 2, BARN), Board::holding(HIGH_MAC, 2, BARN));
    first.pulse_into(&mut table, START_MS);
    second.pulse_into(&mut table, START_MS + 10);
    let facts = own(0, None);
    assert!(!roster(&table, &facts, START_MS + 10).split(2));
    first.pulse_into(&mut table, START_MS + PERIOD_MS);
    assert!(roster(&table, &facts, START_MS + PERIOD_MS).split(2));
}

#[test]
fn a_switchover_shows_the_board_that_started_last_and_is_no_split() {
    let mut table = BoardTable::default();
    let (mut old, mut new) = (Board::holding(LOW_MAC, 2, BARN), Board::holding(HIGH_MAC, 2, BARN));
    old.pulse_into(&mut table, START_MS);
    new.pulse_into(&mut table, START_MS + PERIOD_MS);
    new.pulse_into(&mut table, START_MS + 2 * PERIOD_MS);
    let facts = own(0, None);
    let view = roster(&table, &facts, START_MS + 2 * PERIOD_MS);
    assert_eq!(view.displayed(2).map(|board| board.mac), Some(HIGH_MAC));
    assert!(!view.split(2));
}

#[test]
fn a_board_of_this_installation_still_pulsing_after_this_node_started_is_a_split_pair() {
    let mut table = BoardTable::default();
    let mut partner = Board::holding(HIGH_MAC, 4, HOUSE);
    partner.pulse_into(&mut table, START_MS);
    let facts = own(4, Some(START_MS + 10));
    assert!(!roster(&table, &facts, START_MS + 10).faults().split_pair);
    partner.pulse_into(&mut table, START_MS + PERIOD_MS);
    assert!(roster(&table, &facts, START_MS + PERIOD_MS).faults().split_pair);
}

#[test]
fn the_claim_listens_first_then_starts_on_a_free_number() {
    let table = BoardTable::default();
    let facts = own(1, None);
    let view = roster(&table, &facts, START_MS);
    let listen = TIMING.listen_ms();
    assert_eq!(claim_mode(&view, OwnMode::Listening, listen - 1), OwnMode::Listening);
    assert_eq!(claim_mode(&view, OwnMode::Listening, listen), OwnMode::Running);
}

#[test]
fn a_number_held_by_another_installation_keeps_this_node_waiting_until_the_holder_is_silent() {
    let mut table = BoardTable::default();
    Board::holding(HIGH_MAC, 1, BARN).pulse_into(&mut table, START_MS);
    let facts = own(1, None);
    let listen = TIMING.listen_ms();
    let held = roster(&table, &facts, START_MS);
    assert_eq!(claim_mode(&held, OwnMode::Listening, listen), OwnMode::Waiting);
    let freed = roster(&table, &facts, START_MS + TIMING.liveness_ms() + 1);
    assert_eq!(claim_mode(&freed, OwnMode::Waiting, listen), OwnMode::Running);
}

#[test]
fn when_both_installations_already_run_the_lower_mac_stays() {
    let mut table = BoardTable::default();
    Board::holding(HIGH_MAC, 1, BARN).pulse_into(&mut table, START_MS);
    let facts = own(1, Some(START_MS - PERIOD_MS));
    let listen = TIMING.listen_ms();
    assert_eq!(claim_mode(&roster(&table, &facts, START_MS), OwnMode::Running, listen), OwnMode::Running);
    let mut lower = BoardTable::default();
    Board::holding(LOW_MAC, 1, BARN).pulse_into(&mut lower, START_MS);
    assert_eq!(claim_mode(&roster(&lower, &facts, START_MS), OwnMode::Running, listen), OwnMode::Waiting);
}

#[test]
fn a_board_of_this_installation_never_keeps_this_node_from_starting() {
    let mut table = BoardTable::default();
    Board::holding(LOW_MAC, 1, HOUSE).pulse_into(&mut table, START_MS);
    let facts = own(1, None);
    let view = roster(&table, &facts, START_MS);
    assert_eq!(claim_mode(&view, OwnMode::Listening, TIMING.listen_ms()), OwnMode::Running);
}

#[test]
fn the_wire_owner_is_the_live_member_with_the_lowest_number_then_mac() {
    let mut table = BoardTable::default();
    let mut neighbour = Board::holding(HIGH_MAC, 1, BARN);
    neighbour.wire = 2;
    neighbour.pulse_into(&mut table, START_MS);
    let running = own(2, Some(START_MS));
    assert_eq!(
        roster(&table, &running, START_MS).wire_owner(),
        WireOwner::Board { node: 1, mac: HIGH_MAC }
    );
    let silent = START_MS + TIMING.liveness_ms() + 1;
    assert_eq!(roster(&table, &running, silent).wire_owner(), WireOwner::ThisNode);
    assert_eq!(roster(&table, &own(2, None), silent).wire_owner(), WireOwner::Nobody);
}

#[test]
fn a_node_naming_a_wire_whose_node_points_elsewhere_refuses_to_own_it() {
    let mut table = BoardTable::default();
    let mut middle = Board::holding(LOW_MAC, 1, BARN);
    middle.wire = 6;
    middle.pulse_into(&mut table, START_MS);
    let mut facts = own(3, Some(START_MS));
    facts.wire = 1;
    let view = roster(&table, &facts, START_MS);
    assert!(view.faults().wire_chain);
    assert_eq!(view.wire_owner(), WireOwner::Nobody);
}

#[test]
fn a_node_named_for_a_wire_while_it_points_elsewhere_reports_the_chain() {
    let mut table = BoardTable::default();
    let mut follower = Board::holding(LOW_MAC, 0, BARN);
    follower.wire = 3;
    follower.pulse_into(&mut table, START_MS);
    let mut facts = own(3, Some(START_MS));
    assert!(!roster(&table, &facts, START_MS).faults().wire_chain);
    facts.wire = 5;
    assert!(roster(&table, &facts, START_MS).faults().wire_chain);
}

#[test]
fn a_silent_holder_stays_only_while_its_number_has_no_live_holder() {
    let mut table = BoardTable::default();
    Board::holding(LOW_MAC, 2, BARN).pulse_into(&mut table, START_MS);
    let silent = START_MS + TIMING.liveness_ms() + 1;
    let facts = own(0, None);
    assert!(roster(&table, &facts, silent).keeps(only(&table)));
    Board::holding(HIGH_MAC, 2, BARN).pulse_into(&mut table, silent);
    let view = roster(&table, &facts, silent);
    let kept: Vec<_> = table.iter().filter(|board| view.keeps(board)).map(|board| board.mac).collect();
    assert_eq!(kept, [HIGH_MAC]);
}

#[test]
fn a_silent_ignored_board_is_not_kept() {
    let mut table = BoardTable::default();
    let mut stranger = Board::holding(LOW_MAC, 2, BARN);
    stranger.name = "elsewhere";
    stranger.pulse_into(&mut table, START_MS);
    let facts = own(0, None);
    assert!(roster(&table, &facts, START_MS).keeps(only(&table)));
    assert!(!roster(&table, &facts, START_MS + TIMING.liveness_ms() + 1).keeps(only(&table)));
}
