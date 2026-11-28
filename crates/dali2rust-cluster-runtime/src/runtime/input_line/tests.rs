use core::mem::MaybeUninit;

use super::*;

const START_MS: u32 = 70_000;
const GAP_MS: u32 = 20;
const PRESS: u16 = 1;
const RELEASE: u16 = 0;
const LUX_A: u16 = 300;
const LUX_B: u16 = 310;
const LUX_C: u16 = 320;
const LUX_D: u16 = 330;

fn line() -> InputLine {
    let mut place = MaybeUninit::<InputLine>::uninit();
    // SAFETY: `init` writes every field of the place, so it is initialised when read.
    unsafe {
        InputLine::init(place.as_mut_ptr());
        place.assume_init()
    }
}

fn event(instance: u8, kind: InputEventKind, event_info: u16, value: u16) -> InputEvent {
    InputEvent {
        address: InputAddress { adapter: 0, device: 3, instance: Some(instance) },
        instance_type: Some(1),
        groups: [None; 3],
        scheme: 1,
        kind,
        event_info,
        value,
    }
}

fn button(event_info: u16) -> InputEvent {
    event(0, InputEventKind::Button, event_info, 0)
}

fn lux(instance: u8, value: u16) -> InputEvent {
    event(instance, InputEventKind::Illuminance, 0, value)
}

fn repeat(event: InputEvent, seq: u32) -> InputRepeat {
    let header = Header { cluster_tag: 1, sender_mac: [2, 0, 0, 0, 0, 1], boot_epoch: 3, node_no: 0, seq };
    InputRepeat { header, event }
}

#[test]
fn a_press_goes_now_and_its_release_waits_for_the_copy_never_folded() {
    let mut line = line();
    assert_eq!(line.admit(button(PRESS), false), InputAdmission::Now(button(PRESS)));
    line.sent(repeat(button(PRESS), 7), START_MS, GAP_MS);
    assert_eq!(line.admit(button(RELEASE), line.copying()), InputAdmission::Later);
    assert_eq!(line.admit(button(PRESS), line.copying()), InputAdmission::Later);
    assert_eq!(line.take_repeat(START_MS + GAP_MS - 1), None, "the copy waits its gap");
    assert_eq!(line.take_repeat(START_MS + GAP_MS), Some(repeat(button(PRESS), 7)), "the copy keeps the first's sequence");
    assert_eq!(line.pop(), Some(button(RELEASE)));
    assert_eq!(line.pop(), Some(button(PRESS)), "a second press after the release is its own event");
    assert_eq!(line.pop(), None);
}

#[test]
fn an_event_queued_behind_another_never_overtakes_it() {
    let mut line = line();
    assert_eq!(line.admit(button(PRESS), true), InputAdmission::Later);
    assert_eq!(line.admit(button(RELEASE), false), InputAdmission::Later, "the line is not empty, so the release waits too");
    assert_eq!((line.pop(), line.pop()), (Some(button(PRESS)), Some(button(RELEASE))));
}

#[test]
fn a_full_line_refuses_the_next_event_and_keeps_the_earlier_ones() {
    let mut line = line();
    for _ in 0..INPUT_LINE {
        assert_eq!(line.admit(button(PRESS), true), InputAdmission::Later);
    }
    assert_eq!(line.admit(button(RELEASE), true), InputAdmission::Dropped);
    assert_eq!(core::iter::from_fn(|| line.pop()).count(), INPUT_LINE);
}

#[test]
fn illuminance_goes_at_most_once_a_period_and_the_last_value_trails() {
    let mut line = line();
    let period = period_ms(InputEventKind::Illuminance).expect("illuminance is measured");
    assert_eq!(period, ILLUMINANCE_PERIOD_MS);
    assert_eq!(line.measure(lux(0, LUX_A), period, START_MS), Some(lux(0, LUX_A)));
    assert_eq!(line.measure(lux(0, LUX_B), period, START_MS + 100), None);
    assert_eq!(line.measure(lux(0, LUX_C), period, START_MS + 200), None);
    assert_eq!(line.trailing_due().collect::<Vec<_>>(), vec![START_MS + period]);
    assert_eq!(line.take_trailing(START_MS + period - 1), None);
    assert_eq!(line.take_trailing(START_MS + period), Some(lux(0, LUX_C)), "the newest value, not the first held");
    assert_eq!(line.trailing_due().count(), 0);
    assert_eq!(line.measure(lux(0, LUX_D), period, START_MS + period + 1), None, "the trailing send opened a new period");
    assert_eq!(line.take_trailing(START_MS + 2 * period), Some(lux(0, LUX_D)));
}

#[test]
fn a_value_after_a_quiet_period_goes_now_and_drops_nothing_newer() {
    let mut line = line();
    let period = ILLUMINANCE_PERIOD_MS;
    assert_eq!(line.measure(lux(0, LUX_A), period, START_MS), Some(lux(0, LUX_A)));
    assert_eq!(line.measure(lux(0, LUX_B), period, START_MS + period + 5), Some(lux(0, LUX_B)));
    assert_eq!(line.take_trailing(START_MS + 3 * period), None);
}

#[test]
fn a_held_value_older_than_a_fresh_one_is_replaced_not_sent_after_it() {
    let mut line = line();
    let period = ILLUMINANCE_PERIOD_MS;
    line.measure(lux(0, LUX_A), period, START_MS);
    assert_eq!(line.measure(lux(0, LUX_B), period, START_MS + 10), None);
    assert_eq!(line.measure(lux(0, LUX_C), period, START_MS + period), Some(lux(0, LUX_C)));
    assert_eq!(line.take_trailing(START_MS + 2 * period), None, "the held 310 is older than the 320 sent");
}

#[test]
fn position_has_its_own_window_and_occupancy_and_buttons_none() {
    assert_eq!(period_ms(InputEventKind::Position), Some(POSITION_WINDOW_MS));
    assert_eq!(period_ms(InputEventKind::Occupancy), None);
    assert_eq!(period_ms(InputEventKind::Button), None);
    assert_eq!(period_ms(InputEventKind::Generic), None);
    let mut line = line();
    let moved = |value| event(1, InputEventKind::Position, 0, value);
    assert_eq!(line.measure(moved(10), POSITION_WINDOW_MS, START_MS), Some(moved(10)));
    assert_eq!(line.measure(moved(20), POSITION_WINDOW_MS, START_MS + POSITION_WINDOW_MS - 1), None);
    assert_eq!(line.take_trailing(START_MS + POSITION_WINDOW_MS), Some(moved(20)));
}

#[test]
fn instances_are_measured_apart() {
    let mut line = line();
    let period = ILLUMINANCE_PERIOD_MS;
    assert_eq!(line.measure(lux(0, LUX_A), period, START_MS), Some(lux(0, LUX_A)));
    assert_eq!(line.measure(lux(1, LUX_B), period, START_MS + 1), Some(lux(1, LUX_B)), "another instance has its own period");
}

#[test]
fn a_full_measure_table_sends_untracked_and_loses_no_held_value() {
    let mut line = line();
    let period = ILLUMINANCE_PERIOD_MS;
    let instances = u8::try_from(MEASURED_INSTANCES).expect("a table of instances");
    for instance in 0..instances {
        line.measure(lux(instance, LUX_A), period, START_MS);
        assert_eq!(line.measure(lux(instance, LUX_B), period, START_MS + 1), None);
    }
    assert_eq!(line.measure(lux(instances, LUX_C), period, START_MS + 2), Some(lux(instances, LUX_C)));
    assert_eq!(line.measure(lux(instances, LUX_D), period, START_MS + 3), Some(lux(instances, LUX_D)), "untracked: every value goes");
    let trailing: Vec<_> = core::iter::from_fn(|| line.take_trailing(START_MS + period)).collect();
    assert_eq!(trailing.len(), MEASURED_INSTANCES, "every held value still trails");
    assert!(trailing.iter().all(|sent| sent.value == LUX_B));
}

#[test]
fn an_idle_measure_is_pruned_after_its_window_and_a_held_one_never() {
    let mut line = line();
    let period = ILLUMINANCE_PERIOD_MS;
    line.measure(lux(0, LUX_A), period, START_MS);
    line.measure(lux(1, LUX_A), period, START_MS);
    line.measure(lux(1, LUX_B), period, START_MS + 1);
    line.prune(START_MS + period);
    assert_eq!(line.measure(lux(0, LUX_C), period, START_MS + period + 1), Some(lux(0, LUX_C)));
    assert_eq!(line.take_trailing(START_MS + period), Some(lux(1, LUX_B)));
}

#[test]
fn abandoning_counts_what_was_queued_and_held_and_clears_the_copy() {
    let mut line = line();
    line.sent(repeat(button(PRESS), 1), START_MS, GAP_MS);
    line.admit(button(RELEASE), true);
    line.measure(lux(0, LUX_A), ILLUMINANCE_PERIOD_MS, START_MS);
    line.measure(lux(0, LUX_B), ILLUMINANCE_PERIOD_MS, START_MS + 1);
    assert_eq!(line.abandon(), 2);
    assert!(!line.copying());
    assert_eq!((line.pop(), line.take_trailing(START_MS + ILLUMINANCE_PERIOD_MS)), (None, None));
}
