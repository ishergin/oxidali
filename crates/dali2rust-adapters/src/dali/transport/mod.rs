pub mod mock;

#[cfg(not(target_os = "espidf"))]
pub mod sim;

#[cfg(target_os = "espidf")]
pub mod esp_idf;

#[cfg_attr(
    not(target_os = "espidf"),
    allow(dead_code, reason = "only the ESP sniffer thread runs it; host tests exercise it")
)]
mod sniffer_diagnostics;

#[cfg_attr(
    not(target_os = "espidf"),
    allow(dead_code, reason = "only the ESP sniffer thread runs it; host tests exercise it")
)]
mod answer_path;

pub mod bus_unit_wire;

#[cfg(target_os = "espidf")]
pub mod phy_interrupt;

use dali2rust_dali_codec::rx_decode::{RxDecode, SniffedFrame};
use dali2rust_dali_phy::isr::BUS_FAILURE_POWER_DOWN;
use dali2rust_dali_phy::{
    restart_gate_for, RxCompletedEvent, SessionEvent, BUS_POWER_DOWN_TICKS, LINE_HELD_TICKS,
    PHY_TICK_US,
};
use dali2rust_platform::dali::TransferOutcome;

pub const WIRE_LOAD_WINDOW_TICKS: u32 = 1_000_000 / PHY_TICK_US;

pub(crate) const MICROS_PER_MILLI: u32 = 1_000;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IsrTickLedger {
    origin_us: Option<i64>,
    last_ticks: u32,
    counted_ticks: u64,
    last_check_us: i64,
    deficit_accounted: u64,
    surplus_accounted: u64,
}

impl IsrTickLedger {
    pub(crate) fn account(&mut self, now_us: i64, ticks: u32) -> (u32, u32, u32, u32) {
        let Some(origin_us) = self.origin_us else {
            self.origin_us = Some(now_us);
            self.last_ticks = ticks;
            self.last_check_us = now_us;
            return (0, 0, 0, 0);
        };
        if now_us.saturating_sub(self.last_check_us) < 1_000_000 {
            return (0, 0, 0, 0);
        }
        self.last_check_us = now_us;
        self.counted_ticks = self
            .counted_ticks
            .saturating_add(u64::from(ticks.wrapping_sub(self.last_ticks)));
        self.last_ticks = ticks;
        let elapsed = now_us.saturating_sub(origin_us) as u64;
        let expected = elapsed / u64::from(PHY_TICK_US);
        let counted = self.counted_ticks;
        let deficit = expected.saturating_sub(counted);
        let surplus = counted.saturating_sub(expected);
        let raw_deficit = deficit.saturating_sub(self.deficit_accounted);
        let raw_surplus = surplus.saturating_sub(self.surplus_accounted);
        let missing = deficit
            .saturating_sub(1)
            .saturating_sub(self.deficit_accounted.saturating_sub(1));
        let extra = surplus
            .saturating_sub(1)
            .saturating_sub(self.surplus_accounted.saturating_sub(1));
        self.deficit_accounted = self.deficit_accounted.max(deficit);
        self.surplus_accounted = self.surplus_accounted.max(surplus);
        (
            u32::try_from(missing).unwrap_or(u32::MAX),
            u32::try_from(extra).unwrap_or(u32::MAX),
            u32::try_from(raw_deficit).unwrap_or(u32::MAX),
            u32::try_from(raw_surplus).unwrap_or(u32::MAX),
        )
    }
}

// IEC 62386-101 Table 20, §8.2.5
pub const BACKWARD_ACCEPTANCE_LIMIT_US: u32 = 13_400;

// IEC 62386-101 Table 20
pub const BACKWARD_ACCEPTANCE_FLOOR_US: u32 = 2_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackwardTiming {
    TooEarly,
    InWindow,
    TooLate,
}

pub fn backward_timing(pre_idle_ticks: u8) -> BackwardTiming {
    let settling_us = u32::from(pre_idle_ticks) * PHY_TICK_US;
    if settling_us < BACKWARD_ACCEPTANCE_FLOOR_US {
        BackwardTiming::TooEarly
    } else if settling_us >= BACKWARD_ACCEPTANCE_LIMIT_US {
        BackwardTiming::TooLate
    } else {
        BackwardTiming::InWindow
    }
}

pub fn arrived_too_late_for_a_backward_frame(ev: &RxCompletedEvent) -> bool {
    backward_timing(ev.pre_idle_ticks) == BackwardTiming::TooLate
}

pub fn settled_past_backward_acceptance(pre_idle_ticks: u8) -> bool {
    backward_timing(pre_idle_ticks) == BackwardTiming::TooLate
}

pub fn foreign_forward_outcome(pre_idle_ticks: u8) -> TransferOutcome {
    match backward_timing(pre_idle_ticks) {
        BackwardTiming::TooLate => TransferOutcome::NoAnswer,
        BackwardTiming::TooEarly | BackwardTiming::InWindow => TransferOutcome::ForeignInWindow,
    }
}

pub fn incomplete_reception_outcome(rx_pre_idle_ticks: u8) -> TransferOutcome {
    match backward_timing(rx_pre_idle_ticks) {
        BackwardTiming::TooEarly | BackwardTiming::InWindow => TransferOutcome::CorruptedInWindow,
        BackwardTiming::TooLate => TransferOutcome::NoAnswer,
    }
}

pub fn arrived_too_early_for_a_backward_frame(ev: &RxCompletedEvent) -> bool {
    backward_timing(ev.pre_idle_ticks) == BackwardTiming::TooEarly
}

// IEC 62386-101 §8.2.1, Table 18, Table 19, §8.2.5
pub fn held_capture_outcome(dominant_run_ticks: u16) -> TransferOutcome {
    if dominant_run_ticks >= BUS_POWER_DOWN_TICKS {
        TransferOutcome::BusBusy
    } else {
        TransferOutcome::CorruptedInWindow
    }
}

// IEC 62386-101 §8.2.1, Table 18, Table 19, §8.2.5
pub fn incomplete_reception_verdict(
    bus_failure_flags: u32,
    frame_active: bool,
) -> Option<TransferOutcome> {
    if bus_failure_flags & BUS_FAILURE_POWER_DOWN != 0 {
        Some(TransferOutcome::BusBusy)
    } else if frame_active {
        None
    } else {
        Some(TransferOutcome::CorruptedInWindow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxAttemptOutcome {
    Sent { pre_idle_ticks: u16 },
    Collision,
    BusBusy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxEventCause {
    Completed,
    Collision,
    Rejected,
    Voided,
    UnexpectedBackward,
}

pub fn classify_tx_event(event: &SessionEvent) -> (Option<TxAttemptOutcome>, TxEventCause) {
    match event {
        SessionEvent::TxComplete { pre_idle_ticks } => (
            Some(TxAttemptOutcome::Sent {
                pre_idle_ticks: *pre_idle_ticks,
            }),
            TxEventCause::Completed,
        ),
        SessionEvent::TxCollision => (Some(TxAttemptOutcome::Collision), TxEventCause::Collision),
        SessionEvent::TxRejected => (Some(TxAttemptOutcome::BusBusy), TxEventCause::Rejected),
        SessionEvent::TxVoided => (Some(TxAttemptOutcome::BusBusy), TxEventCause::Voided),
        SessionEvent::RxComplete(_) | SessionEvent::ForeignForward { .. } => {
            (None, TxEventCause::UnexpectedBackward)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackwardCause {
    Undecodable,
    FrameSize,
    Incomplete,
    EarlyRejected,
    LateRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTally {
    ForeignFrame,
    Backward(BackwardCause),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndecodableCause {
    ForeignForward16 { raw: u16 },
    ForeignForward24 { bytes: [u8; 3] },
    Backward8,
    FrameSize { bits: u8 },
    BusPowerDown { run_ticks: u16 },
    LineHeld { run_ticks: u16 },
    BitTiming { run_ticks: u16 },
}

impl UndecodableCause {
    pub fn tally(self) -> WindowTally {
        match self {
            Self::ForeignForward16 { .. } | Self::ForeignForward24 { .. } => {
                WindowTally::ForeignFrame
            }
            Self::Backward8
            | Self::BusPowerDown { .. }
            | Self::LineHeld { .. }
            | Self::BitTiming { .. } => WindowTally::Backward(BackwardCause::Undecodable),
            Self::FrameSize { .. } => WindowTally::Backward(BackwardCause::FrameSize),
        }
    }
}

pub fn classify_undecodable_in_window(
    ev: &RxCompletedEvent,
    frame: SniffedFrame,
) -> (TransferOutcome, UndecodableCause) {
    match frame {
        SniffedFrame::Forward16(raw) => (
            TransferOutcome::ForeignInWindow,
            UndecodableCause::ForeignForward16 { raw },
        ),
        SniffedFrame::Forward24(bytes) => (
            TransferOutcome::ForeignInWindow,
            UndecodableCause::ForeignForward24 { bytes },
        ),
        SniffedFrame::Backward8(_) => (
            TransferOutcome::CorruptedInWindow,
            UndecodableCause::Backward8,
        ),
        SniffedFrame::UnsupportedLength(bits) => (
            TransferOutcome::CorruptedInWindow,
            UndecodableCause::FrameSize { bits },
        ),
        SniffedFrame::DecodeFailed => classify_held_capture(ev.longest_dominant_run_ticks()),
    }
}

fn classify_held_capture(run_ticks: u16) -> (TransferOutcome, UndecodableCause) {
    let outcome = held_capture_outcome(run_ticks);
    let cause = if outcome == TransferOutcome::BusBusy {
        UndecodableCause::BusPowerDown { run_ticks }
    } else if run_ticks >= LINE_HELD_TICKS {
        UndecodableCause::LineHeld { run_ticks }
    } else {
        UndecodableCause::BitTiming { run_ticks }
    };
    (outcome, cause)
}

// IEC 62386-101 §9.1.4, Table 25
const RESTART_FIRST_EDGE_TICKS: u8 = 40;
const RESTART_SPREAD_TICKS: u32 = 4;

pub fn collision_restart_ticks(draw: u32) -> u8 {
    RESTART_FIRST_EDGE_TICKS + (draw % RESTART_SPREAD_TICKS) as u8
}

// IEC 62386-101 §9.1.4, §9.3
pub fn collision_restart_gate(frame_key: u32, previous_key: Option<u32>, draw: u32) -> u8 {
    if previous_key == Some(frame_key) {
        return 0;
    }
    restart_gate_for(collision_restart_ticks(draw))
}

#[cfg(test)]
mod collision_restart_tests {
    use super::*;

    // IEC 62386-101 Table 25
    const T_RECOVER_US: (u32, u32) = (4_000, 4_600);

    #[test]
    fn every_drawn_restart_lands_inside_t_recover_with_a_tick_to_spare() {
        let mut seen = std::collections::BTreeSet::new();
        for draw in 0..64u32 {
            let ticks = u32::from(collision_restart_ticks(draw));
            seen.insert(ticks);
            assert!((ticks - 1) * PHY_TICK_US >= T_RECOVER_US.0, "{ticks} ticks");
            assert!((ticks + 1) * PHY_TICK_US <= T_RECOVER_US.1, "{ticks} ticks");
        }
        assert_eq!(seen.len(), 4, "footnote a: the restart point is spread over the window");
    }

    #[test]
    fn a_frame_repeating_its_predecessor_is_never_restarted_alone() {
        assert_eq!(
            collision_restart_gate(0x01A5, Some(0x01A5), 7),
            0,
            "101 §9.3: a break splits a send-twice pair, so its second copy is not restarted"
        );
        assert_ne!(collision_restart_gate(0x01A5, Some(0x01A4), 7), 0);
        assert_ne!(collision_restart_gate(0x01A5, None, 7), 0);
    }
}

#[cfg(test)]
mod backward_window_tests {
    use super::*;
    use dali2rust_dali_phy::LINE_HELD_TICKS;

    fn arriving_after(ticks: u8) -> RxCompletedEvent {
        RxCompletedEvent {
            pre_idle_ticks: ticks,
            ..RxCompletedEvent::new()
        }
    }

    const FIRST_ACCEPTED_TICK: u8 = BACKWARD_ACCEPTANCE_FLOOR_US.div_ceil(PHY_TICK_US) as u8;

    #[test]
    fn every_answer_a_conforming_gear_can_send_is_accepted() {
        for us in [5_500u32, 8_000, 10_500, 12_400] {
            let ticks = (us / PHY_TICK_US) as u8;
            let ev = arriving_after(ticks);
            assert!(
                !arrived_too_late_for_a_backward_frame(&ev)
                    && !arrived_too_early_for_a_backward_frame(&ev),
                "{us} µs is inside the Table 20 acceptance window"
            );
        }
    }

    const SHARED_INTERFACE_CORRUPTION_US: [u32; 2] = [1_300, 2_000];

    #[test]
    fn an_active_state_short_of_45_ms_is_a_bit_timing_violation_and_so_an_answer() {
        let shared = SHARED_INTERFACE_CORRUPTION_US.map(|us| (us / PHY_TICK_US) as u16);
        for run in [shared[0], shared[1], LINE_HELD_TICKS, 200, BUS_POWER_DOWN_TICKS - 1] {
            assert_eq!(
                held_capture_outcome(run),
                TransferOutcome::CorruptedInWindow,
                "{run} ticks of active state: 101 Tables 18 and 19 call it a bit-timing \
                 violation, and §8.2.5 reads a violation in the window as a backward frame"
            );
        }
    }

    #[test]
    fn an_active_state_past_45_ms_is_bus_power_down_and_answers_nothing() {
        for run in [BUS_POWER_DOWN_TICKS, BUS_POWER_DOWN_TICKS + 1, u16::MAX] {
            assert_eq!(
                held_capture_outcome(run),
                TransferOutcome::BusBusy,
                "{run} ticks: footnote b of Tables 18 and 19 makes it bus power down, not a frame"
            );
        }
    }

    #[test]
    fn a_reception_still_active_at_the_deadline_waits_for_its_verdict() {
        assert_eq!(incomplete_reception_verdict(0, true), None);
        assert_eq!(
            incomplete_reception_verdict(0, false),
            Some(TransferOutcome::CorruptedInWindow),
            "released before 45 ms: the hold was a violation, so a backward frame"
        );
        assert_eq!(
            incomplete_reception_verdict(BUS_FAILURE_POWER_DOWN, false),
            Some(TransferOutcome::BusBusy),
            "still active at 45 ms: bus power down, not an answer"
        );
    }

    #[test]
    fn an_incomplete_reception_is_an_answer_only_while_table_20_says_it_could_be() {
        for ticks in [0, 16, FIRST_ACCEPTED_TICK - 1] {
            assert_eq!(
                incomplete_reception_outcome(ticks),
                TransferOutcome::CorruptedInWindow,
                "{ticks} ticks in: preserve the re-probe path until bench evidence decides it"
            );
        }
        for us in [5_500u32, 8_000, 10_500, 12_400] {
            let ticks = (us / PHY_TICK_US) as u8;
            assert_eq!(
                incomplete_reception_outcome(ticks),
                TransferOutcome::CorruptedInWindow,
                "{us} µs in: a gear could still have been answering us"
            );
        }
        const FIRST_LATE_TICK: u8 = BACKWARD_ACCEPTANCE_LIMIT_US.div_ceil(PHY_TICK_US) as u8;
        for ticks in [FIRST_LATE_TICK, 192, 209, 252] {
            assert_eq!(
                incomplete_reception_outcome(ticks),
                TransferOutcome::NoAnswer,
                "{ticks} ticks in: Table 22 gives that slot to another master, \
                 and our query's answer is silence — NO (102 §3.13)"
            );
        }
    }

    #[test]
    fn a_late_frame_means_the_same_whether_or_not_it_finished() {
        for ticks in [
            BACKWARD_ACCEPTANCE_LIMIT_US.div_ceil(PHY_TICK_US) as u8,
            192,
            252,
        ] {
            assert_eq!(
                incomplete_reception_outcome(ticks),
                foreign_forward_outcome(ticks),
                "{ticks} ticks in: completed or not, it is somebody else's frame"
            );
        }
    }

    #[test]
    fn a_frame_below_the_table_20_floor_is_not_our_answer() {
        assert!(arrived_too_early_for_a_backward_frame(&arriving_after(16)));
        assert!(
            arrived_too_early_for_a_backward_frame(&arriving_after(FIRST_ACCEPTED_TICK - 1)),
            "tick {} reads back as {} µs, below the {BACKWARD_ACCEPTANCE_FLOOR_US} µs floor",
            FIRST_ACCEPTED_TICK - 1,
            u32::from(FIRST_ACCEPTED_TICK - 1) * PHY_TICK_US,
        );
        assert!(
            !arrived_too_early_for_a_backward_frame(&arriving_after(FIRST_ACCEPTED_TICK)),
            "tick {FIRST_ACCEPTED_TICK} is the first at or above the floor and must be accepted"
        );
    }

    #[test]
    fn the_two_ends_of_table_20_leave_the_legal_window_open() {
        for us in [5_500u32, 8_000, 10_500, 12_400] {
            let ticks = (us / PHY_TICK_US) as u8;
            let ev = arriving_after(ticks);
            assert!(
                !arrived_too_early_for_a_backward_frame(&ev)
                    && !arrived_too_late_for_a_backward_frame(&ev),
                "{us} µs is what a conforming gear does — both ends must accept it"
            );
        }
        const { assert!(BACKWARD_ACCEPTANCE_FLOOR_US < BACKWARD_ACCEPTANCE_LIMIT_US) };
    }

    #[test]
    fn a_frame_at_or_past_the_limit_is_not_our_answer() {
        let at_limit = (BACKWARD_ACCEPTANCE_LIMIT_US / PHY_TICK_US) as u8;
        assert!(arrived_too_late_for_a_backward_frame(&arriving_after(
            at_limit + 1
        )));
        assert!(arrived_too_late_for_a_backward_frame(&arriving_after(240)));
    }

    #[test]
    fn waiting_stops_exactly_where_accepting_stops() {
        let closes_at_ticks = BACKWARD_ACCEPTANCE_LIMIT_US.div_ceil(PHY_TICK_US);
        let before = (closes_at_ticks - 1) as u8;
        assert!(
            !arrived_too_late_for_a_backward_frame(&arriving_after(before)),
            "the wait may not close while a legal answer can still arrive"
        );
        assert!(arrived_too_late_for_a_backward_frame(&arriving_after(
            closes_at_ticks as u8
        )));
    }

    #[test]
    fn a_foreign_forward_frame_inside_the_window_is_foreign_in_window() {
        for ticks in [0, 16, FIRST_ACCEPTED_TICK - 1] {
            assert_eq!(
                foreign_forward_outcome(ticks),
                TransferOutcome::ForeignInWindow,
                "{ticks} ticks in: the early marker preserves the existing contention path"
            );
        }
        assert_eq!(
            foreign_forward_outcome(60),
            TransferOutcome::ForeignInWindow
        );
        let last_inside = ((BACKWARD_ACCEPTANCE_LIMIT_US - 1) / PHY_TICK_US) as u8;
        assert_eq!(
            foreign_forward_outcome(last_inside),
            TransferOutcome::ForeignInWindow
        );
    }

    #[test]
    fn a_foreign_forward_frame_past_the_acceptance_limit_is_no_answer() {
        let at_limit = BACKWARD_ACCEPTANCE_LIMIT_US.div_ceil(PHY_TICK_US) as u8;
        assert_eq!(foreign_forward_outcome(at_limit), TransferOutcome::NoAnswer);
        assert_eq!(foreign_forward_outcome(u8::MAX), TransferOutcome::NoAnswer);
        assert!(arrived_too_late_for_a_backward_frame(&arriving_after(
            at_limit
        )));
        assert!(!arrived_too_late_for_a_backward_frame(&arriving_after(
            at_limit - 1
        )));
    }

    #[test]
    fn the_boundary_falls_between_two_adjacent_ticks() {
        let last_accepted = (BACKWARD_ACCEPTANCE_LIMIT_US - 1) / PHY_TICK_US;
        assert!(!arrived_too_late_for_a_backward_frame(&arriving_after(
            last_accepted as u8
        )));
        assert!(arrived_too_late_for_a_backward_frame(&arriving_after(
            last_accepted as u8 + 1
        )));
    }
}

#[cfg(test)]
mod outcome_classification_tests {
    use super::*;
    use dali2rust_dali_phy::DALI_RX_SAMPLE_BUF_LEN;

    fn classified_tx(event: SessionEvent) -> (Option<TxAttemptOutcome>, TxEventCause) {
        classify_tx_event(&event)
    }

    fn capture_with_dominant_run(run: u16) -> RxCompletedEvent {
        let mut ev = RxCompletedEvent::new();
        ev.samples = [0xFF; DALI_RX_SAMPLE_BUF_LEN];
        let full = usize::from(run / 8);
        ev.samples[..full].fill(0);
        let rest = run % 8;
        if rest > 0 {
            ev.samples[full] = 0xFF >> rest;
        }
        ev.sample_count = run.div_ceil(8);
        ev
    }

    fn classified_rx(
        ev: &RxCompletedEvent,
        frame: SniffedFrame,
    ) -> (TransferOutcome, UndecodableCause) {
        classify_undecodable_in_window(ev, frame)
    }

    #[test]
    fn a_completed_transmission_is_sent_and_keeps_its_settle_ticks() {
        for ticks in [0u16, 1, 40, u16::MAX] {
            assert_eq!(
                classified_tx(SessionEvent::TxComplete { pre_idle_ticks: ticks }),
                (
                    Some(TxAttemptOutcome::Sent { pre_idle_ticks: ticks }),
                    TxEventCause::Completed
                )
            );
        }
    }

    #[test]
    fn a_collision_is_a_collision() {
        assert_eq!(
            classified_tx(SessionEvent::TxCollision),
            (Some(TxAttemptOutcome::Collision), TxEventCause::Collision)
        );
    }

    #[test]
    fn a_rejected_start_and_a_voided_frame_are_both_bus_busy_for_different_reasons() {
        assert_eq!(
            classified_tx(SessionEvent::TxRejected),
            (Some(TxAttemptOutcome::BusBusy), TxEventCause::Rejected)
        );
        assert_eq!(
            classified_tx(SessionEvent::TxVoided),
            (Some(TxAttemptOutcome::BusBusy), TxEventCause::Voided)
        );
    }

    #[test]
    fn a_backward_event_while_waiting_for_transmit_completion_is_not_an_outcome() {
        assert_eq!(
            classified_tx(SessionEvent::RxComplete(RxCompletedEvent::new())),
            (None, TxEventCause::UnexpectedBackward)
        );
        for (pre_idle_ticks, sample_count) in [(0u8, 0u16), (60, 19), (u8::MAX, u16::MAX)] {
            assert_eq!(
                classified_tx(SessionEvent::ForeignForward { pre_idle_ticks, sample_count }),
                (None, TxEventCause::UnexpectedBackward)
            );
        }
    }

    #[test]
    fn a_decoded_foreign_forward16_in_the_window_is_foreign_and_counted_as_a_frame() {
        let (outcome, cause) = classified_rx(&RxCompletedEvent::new(), SniffedFrame::Forward16(0x82A9));
        assert_eq!(outcome, TransferOutcome::ForeignInWindow);
        assert_eq!(cause, UndecodableCause::ForeignForward16 { raw: 0x82A9 });
        assert_eq!(cause.tally(), WindowTally::ForeignFrame);
    }

    #[test]
    fn a_decoded_foreign_forward24_in_the_window_is_foreign_and_counted_as_a_frame() {
        let frame = SniffedFrame::Forward24([0xFF, 0xFE, 0x3D]);
        let (outcome, cause) = classified_rx(&RxCompletedEvent::new(), frame);
        assert_eq!(outcome, TransferOutcome::ForeignInWindow);
        assert_eq!(cause, UndecodableCause::ForeignForward24 { bytes: [0xFF, 0xFE, 0x3D] });
        assert_eq!(cause.tally(), WindowTally::ForeignFrame);
    }

    #[test]
    fn a_decoded_backward_byte_that_reached_this_path_is_corrupted_whatever_the_line_did() {
        for run in [0u16, LINE_HELD_TICKS, BUS_POWER_DOWN_TICKS, 512] {
            let ev = capture_with_dominant_run(run);
            let (outcome, cause) = classified_rx(&ev, SniffedFrame::Backward8(0x55));
            assert_eq!(outcome, TransferOutcome::CorruptedInWindow, "run {run}");
            assert_eq!(cause, UndecodableCause::Backward8, "run {run}");
            assert_eq!(
                cause.tally(),
                WindowTally::Backward(BackwardCause::Undecodable)
            );
        }
    }

    #[test]
    fn an_unsupported_frame_length_is_a_size_violation_whatever_the_line_did() {
        for run in [0u16, LINE_HELD_TICKS, BUS_POWER_DOWN_TICKS, 512] {
            let ev = capture_with_dominant_run(run);
            let (outcome, cause) = classified_rx(&ev, SniffedFrame::UnsupportedLength(12));
            assert_eq!(outcome, TransferOutcome::CorruptedInWindow, "run {run}");
            assert_eq!(cause, UndecodableCause::FrameSize { bits: 12 }, "run {run}");
            assert_eq!(
                cause.tally(),
                WindowTally::Backward(BackwardCause::FrameSize)
            );
        }
    }

    fn assert_held_capture(run: u16, outcome: TransferOutcome, cause: UndecodableCause) {
        let ev = capture_with_dominant_run(run);
        assert_eq!(ev.longest_dominant_run_ticks(), run, "the fixture measures its own run");
        assert_eq!(classified_rx(&ev, SniffedFrame::DecodeFailed), (outcome, cause), "run {run}");
        assert_eq!(
            cause.tally(),
            WindowTally::Backward(BackwardCause::Undecodable),
            "run {run}"
        );
    }

    #[test]
    fn a_dominant_run_short_of_a_hold_is_a_bit_timing_violation_and_so_an_answer() {
        for run in [0, 1, LINE_HELD_TICKS - 1] {
            let cause = UndecodableCause::BitTiming { run_ticks: run };
            assert_held_capture(run, TransferOutcome::CorruptedInWindow, cause);
        }
    }

    #[test]
    fn a_hold_short_of_45_ms_is_a_held_line_and_still_an_answer() {
        for run in [LINE_HELD_TICKS, 100, BUS_POWER_DOWN_TICKS - 1] {
            let cause = UndecodableCause::LineHeld { run_ticks: run };
            assert_held_capture(run, TransferOutcome::CorruptedInWindow, cause);
        }
    }

    #[test]
    fn a_dominant_run_of_45_ms_or_more_is_bus_power_down_and_no_answer() {
        for run in [BUS_POWER_DOWN_TICKS, BUS_POWER_DOWN_TICKS + 1, 512] {
            let cause = UndecodableCause::BusPowerDown { run_ticks: run };
            assert_held_capture(run, TransferOutcome::BusBusy, cause);
        }
    }

    #[test]
    fn a_held_line_the_sniffer_cannot_decode_is_classified_from_its_own_decode() {
        for (run, outcome) in [
            (LINE_HELD_TICKS, TransferOutcome::CorruptedInWindow),
            (100, TransferOutcome::CorruptedInWindow),
            (BUS_POWER_DOWN_TICKS, TransferOutcome::BusBusy),
        ] {
            let ev = capture_with_dominant_run(run);
            let decoded = ev.decode_sniffed_with_phase();
            assert_eq!(decoded.frame, SniffedFrame::DecodeFailed, "run {run}");
            assert_eq!(classified_rx(&ev, decoded.frame).0, outcome, "run {run}");
        }
    }

    #[test]
    fn the_hold_split_agrees_with_the_codec_and_the_outcome_with_the_held_capture_rule() {
        for run in 0..=512u16 {
            let ev = capture_with_dominant_run(run);
            let (outcome, cause) = classified_rx(&ev, SniffedFrame::DecodeFailed);
            assert_eq!(outcome, held_capture_outcome(run), "run {run}");
            let held = matches!(
                cause,
                UndecodableCause::LineHeld { .. } | UndecodableCause::BusPowerDown { .. }
            );
            assert_eq!(held, ev.is_line_held(), "run {run}");
        }
    }
}

#[cfg(test)]
mod isr_tick_ledger_tests {
    use super::IsrTickLedger;

    #[test]
    fn cumulative_accounting_does_not_rebook_an_open_deficit() {
        let mut ledger = IsrTickLedger::default();
        assert_eq!(ledger.account(0, 0), (0, 0, 0, 0));
        assert_eq!(ledger.account(1_040_000, 9_998), (1, 0, 2, 0));
        assert_eq!(
            ledger.account(2_080_000, 19_998),
            (0, 0, 0, 0),
            "the same cumulative deficit is not another lost tick"
        );
        assert_eq!(ledger.account(3_120_000, 29_997), (1, 0, 1, 0));
    }

    #[test]
    fn subsecond_samples_leave_the_ledger_unchanged() {
        let mut ledger = IsrTickLedger::default();
        assert_eq!(ledger.account(100, 7), (0, 0, 0, 0));
        assert_eq!(ledger.account(999_999, 1), (0, 0, 0, 0));
        assert_eq!(ledger.account(1_040_100, 10_007), (0, 0, 0, 0));
    }

    #[test]
    fn periodic_accounting_stays_correct_across_the_u32_tick_wrap() {
        let mut ledger = IsrTickLedger::default();
        let before_wrap = u32::MAX - 10_000;
        assert_eq!(ledger.account(0, 0), (0, 0, 0, 0));
        assert_eq!(
            ledger.account(i64::from(before_wrap) * 104, before_wrap),
            (0, 0, 0, 0)
        );
        assert_eq!(
            ledger.account(i64::from(u32::MAX) * 104 + 1_040_000, 9_999),
            (0, 0, 0, 0),
            "a normal u32 wrap is not a multi-billion-tick deficit"
        );
    }
}

#[cfg(test)]
mod phy_constant_pins {
    #[test]
    fn the_controller_restates_the_phy_tick_correctly() {
        assert_eq!(
            dali2rust_dali_phy::PHY_TICK_US,
            dali2rust_dali_runtime::runtime::controller::PHY_TICK_US,
            "the DALI controller's PHY_TICK_US copy has drifted from the PHY"
        );
    }

    #[test]
    fn the_controller_restates_the_arming_lead_correctly() {
        assert_eq!(
            dali2rust_dali_phy::TX_ARM_LEAD_TICKS,
            dali2rust_dali_runtime::runtime::controller::TX_ARM_LEAD_TICKS,
            "the DALI controller's TX_ARM_LEAD_TICKS copy has drifted from the PHY"
        );
    }
}

#[cfg_attr(not(target_os = "espidf"), allow(dead_code, reason = "only the ESP build calls this"))]
pub(crate) const LATE_REPORT_BYTES: usize = 768;

#[cfg_attr(not(target_os = "espidf"), allow(dead_code, reason = "only the ESP build calls this"))]
pub(crate) fn forward_addresses_single_gear(first_byte: u8) -> bool {
    first_byte & 0x80 == 0
}

#[cfg(test)]
mod forward_address_class_tests {
    use super::forward_addresses_single_gear;

    #[test]
    fn short_addresses_are_single_gear() {
        for first in [0x01u8, 0x7e, 0x09] {
            assert!(forward_addresses_single_gear(first), "{first:#04x}");
        }
    }

    #[test]
    fn group_special_and_broadcast_may_draw_several_answers() {
        for first in [0x81u8, 0xa9, 0xbb, 0xfd, 0xff] {
            assert!(!forward_addresses_single_gear(first), "{first:#04x}");
        }
    }
}

pub(crate) struct LateReportLine<'a> {
    bytes: &'a mut [u8],
    len: usize,
}

impl<'a> LateReportLine<'a> {
    pub(crate) fn over(bytes: &'a mut [u8]) -> Self {
        Self { bytes, len: 0 }
    }

    pub(crate) fn as_str(&self) -> &str {
        match core::str::from_utf8(&self.bytes[..self.len]) {
            Ok(text) => text,
            Err(error) => {
                let good = error.valid_up_to();
                // SAFETY: `valid_up_to` is the length of a prefix that already validated as UTF-8.
                unsafe { core::str::from_utf8_unchecked(&self.bytes[..good]) }
            }
        }
    }
}

impl core::fmt::Write for LateReportLine<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let take = (self.bytes.len() - self.len).min(text.len());
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

#[cfg(test)]
mod late_report_line_tests {
    use super::{LateReportLine, LATE_REPORT_BYTES};
    use core::fmt::Write;

    #[test]
    fn a_split_character_does_not_erase_everything_before_it() {
        let mut buffer = [0u8; LATE_REPORT_BYTES];
        let mut line = LateReportLine::over(&mut buffer);
        let filler = "a".repeat(766);
        let _ = line.write_str(&filler);
        let _ = write!(line, "€");
        assert_eq!(line.len, 768, "the buffer took everything it could hold");
        assert_eq!(line.as_str(), filler, "the whole prefix survives the split");
    }

    #[test]
    fn a_line_that_fits_is_returned_whole() {
        let mut buffer = [0u8; LATE_REPORT_BYTES];
        let mut line = LateReportLine::over(&mut buffer);
        let _ = write!(line, "census+isr щ");
        assert_eq!(line.as_str(), "census+isr щ");
    }
}
