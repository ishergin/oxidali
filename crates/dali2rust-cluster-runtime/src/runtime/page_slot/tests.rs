use super::*;
use crate::presence::HALF_CLOCK_MS;

const SENT_MS: u32 = 1_000;

fn taken_at(now_ms: u32) -> PageSlot {
    let mut slot = PageSlot::default();
    slot.take(now_ms);
    slot
}

#[test]
fn a_page_holds_the_slot_for_one_page_gap() {
    let slot = taken_at(SENT_MS);
    assert!(slot.held(SENT_MS), "the page just sent holds the slot");
    assert!(slot.held(SENT_MS + PAGE_GAP_MS - 1), "the slot is held until the gap ends");
    assert!(!slot.held(SENT_MS + PAGE_GAP_MS), "the slot frees once the gap has passed");
    assert!(!PageSlot::default().held(SENT_MS), "a slot no page took is free");
}

#[test]
fn a_slot_last_taken_half_a_clock_ago_is_free() {
    let slot = taken_at(SENT_MS);
    for since_ms in [HALF_CLOCK_MS, HALF_CLOCK_MS + PAGE_GAP_MS + 5, u32::MAX - PAGE_GAP_MS] {
        let now_ms = SENT_MS.wrapping_add(since_ms);
        assert!(!slot.held(now_ms), "a page sent {since_ms} ms ago still holds the slot");
        assert_eq!(slot.clamp(now_ms, now_ms), now_ms, "a page due now waits {since_ms} ms after the last one");
    }
}

#[test]
fn a_page_due_inside_the_gap_waits_for_its_end() {
    let slot = taken_at(SENT_MS);
    let end_ms = SENT_MS + PAGE_GAP_MS;
    assert_eq!(slot.clamp(SENT_MS + 1, SENT_MS + 2), end_ms, "a page due inside the gap goes at its end");
    assert_eq!(slot.clamp(SENT_MS + 1, end_ms + 5), end_ms + 5, "a page due after the gap keeps its time");
}
