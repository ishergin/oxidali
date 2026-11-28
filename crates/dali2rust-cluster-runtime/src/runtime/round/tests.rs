use dali2rust_contracts::msg::{fixed_text_64, ClusterBirthMark};
use dali2rust_domain::registry::{ClusterRecordNumbers, ClusterRecordView};

use super::*;

const TIMING: PresenceTiming = PresenceTiming::every_ms(100);
const KIND: ClusterRecordKind = ClusterRecordKind::Channel;
const NUMBER: u8 = 83;

fn turn(now_ms: u32) -> RoundTurn {
    RoundTurn { running: true, own: 5, now_ms }
}

fn one_intent() -> Vec<ClusterRecordTakeBackCommand> {
    vec![ClusterRecordTakeBackCommand { kind: KIND, number: NUMBER, generation_floor: 1, record: None }]
}

#[test]
fn the_round_listens_until_every_member_was_heard_or_one_round_passed() {
    let mut round = BlockRound::new();
    assert!(round.step(turn(0), (false, TIMING), || panic!("nothing is offered while listening")).is_empty());
    assert!(!round.settled());
    assert_eq!(round.deadline(TIMING), Some(TIMING.block_round_ms()));
    assert!(round.step(turn(TIMING.block_round_ms() - 1), (false, TIMING), || panic!("still listening")).is_empty());
    assert!(round.step(turn(TIMING.block_round_ms()), (false, TIMING), Vec::new).is_empty());
    assert!(round.settled(), "a round with nothing to take back settles");
}

#[test]
fn a_round_whose_members_were_all_heard_settles_before_its_bound() {
    let mut round = BlockRound::new();
    round.step(turn(0), (false, TIMING), Vec::new);
    assert!(round.step(turn(10), (true, TIMING), Vec::new).is_empty());
    assert!(round.settled());
}

#[test]
fn take_backs_are_sent_again_until_none_is_wanted_and_at_most_three_times() {
    let mut round = BlockRound::new();
    round.step(turn(0), (false, TIMING), Vec::new);
    assert_eq!(round.step(turn(5), (true, TIMING), one_intent).len(), 1);
    assert!(!round.settled(), "nothing is issued before the registry holds what was taken back");
    let coalesce = TIMING.info_coalesce_ms();
    assert!(round.step(turn(5 + coalesce - 1), (true, TIMING), || panic!("not yet")).is_empty());
    assert_eq!(round.step(turn(5 + coalesce), (true, TIMING), one_intent).len(), 1);
    assert_eq!(round.step(turn(5 + 2 * coalesce), (true, TIMING), one_intent).len(), 1);
    assert!(round.step(turn(5 + 3 * coalesce), (true, TIMING), one_intent).is_empty());
    assert!(round.settled(), "a refused take-back stops after its attempts");
}

#[test]
fn leaving_running_starts_the_round_again() {
    let mut round = BlockRound::new();
    round.step(turn(0), (true, TIMING), Vec::new);
    assert!(round.settled());
    round.step(RoundTurn { running: false, ..turn(1) }, (true, TIMING), || panic!("not running"));
    assert!(!round.settled());
    assert!(round.step(turn(2), (false, TIMING), || panic!("listening again")).is_empty());
    assert!(!round.settled());
}

fn held(generation: u32, nonce: u32, revision: u32) -> HeldRecord {
    HeldRecord { mark: ClusterBirthMark { generation, nonce }, name: fixed_text_64("Hall"), revision }
}

struct Registry {
    live: Option<HeldRecord>,
    memory: u32,
}

impl ClusterRecordReadPort for Registry {
    fn cluster_record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let live = self.live.as_ref()?;
        let name = live.name.as_str().into();
        Some(ClusterRecordView { kind, number, home_node: 5, mark: live.mark, name, revision: live.revision })
    }

    fn cluster_record_numbers(&self, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        ClusterRecordNumbers::default()
    }

    fn cluster_generation(&self, _kind: ClusterRecordKind, _number: u8) -> u32 {
        self.memory
    }
}

fn wanted(registry: &Registry, best: Option<HeldRecord>, floor: u32) -> Option<(u32, Option<u32>)> {
    let command = intent(registry, KIND, NUMBER, Offer { best, floor })?;
    Some((command.generation_floor, command.record.map(|record| record.mark.generation)))
}

#[test]
fn an_empty_position_adopts_a_copy_newer_than_its_memory() {
    let empty = Registry { live: None, memory: 0 };
    assert_eq!(wanted(&empty, Some(held(2, 7, 3)), 2), Some((2, Some(2))));
}

#[test]
fn a_record_deleted_on_purpose_is_not_taken_back() {
    let deleted = Registry { live: None, memory: 2 };
    assert_eq!(wanted(&deleted, Some(held(2, 7, 3)), 2), None);
}

#[test]
fn a_part_naming_a_newer_generation_raises_the_memory_without_a_record() {
    let empty = Registry { live: None, memory: 0 };
    let raised = intent(&empty, KIND, NUMBER, Offer { best: Some(held(2, 7, 3)), floor: 4 }).expect("raised");
    assert_eq!(raised.generation_floor, 4);
    assert_eq!(wanted(&empty, None, 4), Some((4, None)));
    assert_eq!(wanted(&Registry { live: None, memory: 4 }, None, 4), None);
}

#[test]
fn a_live_record_is_only_refreshed_at_its_own_mark_with_a_higher_revision() {
    let live = Registry { live: Some(held(2, 7, 3)), memory: 2 };
    assert!(wanted(&live, Some(held(2, 7, 4)), 2).is_some());
    assert_eq!(wanted(&live, Some(held(2, 7, 3)), 2), None, "the same revision");
    assert_eq!(wanted(&live, Some(held(3, 1, 9)), 3), None, "another mark never replaces a live record");
    assert_eq!(wanted(&live, None, 5), None);
}

#[test]
fn an_offer_keeps_the_newest_generation_then_the_highest_revision_and_the_highest_floor() {
    let mut offer = Offer::default();
    offer.take(Some(&held(2, 7, 9)), [1].into_iter());
    offer.take(Some(&held(3, 1, 1)), [2].into_iter());
    offer.take(Some(&held(3, 1, 4)), core::iter::empty());
    offer.take(None, [5].into_iter());
    assert_eq!(offer.best.map(|best| (best.mark.generation, best.revision)), Some((3, 4)));
    assert_eq!(offer.floor, 5);
}
