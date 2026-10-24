use core::fmt::Write as _;
use core::time::Duration;
use std::collections::HashMap;
use std::time::Instant;

use dali2rust_dali_codec::rx_decode::{SniffedDecode, SniffedFrame};
use dali2rust_dali_phy::{
    LateTick, BACKWARD8_SAMPLE_COUNT, FORWARD16_SAMPLE_COUNT, FORWARD24_SAMPLE_COUNT,
    LINE_HELD_TICKS, PHY_TICK_US,
};

use super::{IsrTickLedger, LateReportLine, MICROS_PER_MILLI};

const SNIFFER_DIAG_INTERVAL_MS: u64 = 5_000;
const CALIBRATION_DUMP_EVERY: u32 = 256;

#[derive(Debug, Clone, Copy, Default)]
struct LateTally {
    count: u32,
    max_gap_us: u32,
    pc_at_max: u32,
    ra_at_max: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LateTaskKey {
    handle: usize,
    name: Option<&'static core::ffi::CStr>,
    nested: bool,
}

impl core::fmt::Display for LateTaskKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.handle == 0 {
            f.write_str("(none)")?;
        } else if let Some(name) = self.name {
            f.write_str(name.to_str().unwrap_or("<invalid-task-name>"))?;
        } else {
            write!(f, "0x{:08x} (not registered)", self.handle)?;
        }
        if self.nested {
            f.write_str("+isr")?;
        }
        Ok(())
    }
}

pub(crate) struct SnifferDiagnostics {
    last_report: Instant,
    line_held: (u32, u32),
    since_calibration_dump: u32,
    tick_ledger: IsrTickLedger,
    isr_ticks_lost: u32,
    isr_tick_windows_short: u32,
    isr_last_short_window: (u32, i64),
    isr_ticks_extra: u32,
    isr_ticks_deficit_raw: u32,
    isr_ticks_surplus_raw: u32,
    isr_ticks_raw_reported: (u32, u32),
    stage_max_ticks: u32,
    stage_over_budget: u32,
    stage_samples: u32,
    poll_gap_max_us: i64,
    poll_late: u32,
    last_poll_us: Option<i64>,
    probe_counts: (u32, u32),
    late_by_task: HashMap<LateTaskKey, LateTally>,
    frames: u32,
    forward16: u32,
    forward24: u32,
    backward8: u32,
    decode_failed: u32,
    decode_failed_forward16_len: u32,
    phase_adjusted: u32,
    unsupported_len: u32,
    dropped: u32,
    samples_backward8: u32,
    samples_forward16: u32,
    samples_forward24: u32,
    samples_other: u32,
    min_samples: u16,
    max_samples: u16,
    last_persist_sample: (u32, u32),
    tx_holds: (u32, u32),
    last_tx_holds: (u32, u32),
}

impl SnifferDiagnostics {
    pub(crate) fn new() -> Self {
        Self {
            last_report: Instant::now(),
            line_held: (0, 0),
            since_calibration_dump: 0,
            tick_ledger: IsrTickLedger::default(),
            isr_ticks_lost: 0,
            isr_tick_windows_short: 0,
            isr_last_short_window: (0, 0),
            isr_ticks_extra: 0,
            isr_ticks_deficit_raw: 0,
            isr_ticks_surplus_raw: 0,
            isr_ticks_raw_reported: (0, 0),
            stage_max_ticks: 0,
            stage_over_budget: 0,
            stage_samples: 0,
            poll_gap_max_us: 0,
            poll_late: 0,
            last_poll_us: None,
            probe_counts: (0, 0),
            late_by_task: HashMap::new(),
            last_persist_sample: (0, 0),
            tx_holds: (0, 0),
            last_tx_holds: (0, 0),
            frames: 0,
            forward16: 0,
            forward24: 0,
            backward8: 0,
            decode_failed: 0,
            decode_failed_forward16_len: 0,
            phase_adjusted: 0,
            unsupported_len: 0,
            dropped: 0,
            samples_backward8: 0,
            samples_forward16: 0,
            samples_forward24: 0,
            samples_other: 0,
            min_samples: u16::MAX,
            max_samples: 0,
        }
    }

    pub(crate) fn record_dropped(&mut self, dropped: u32) {
        self.dropped = self.dropped.saturating_add(dropped);
    }

    pub(crate) fn record_frame(&mut self, sample_count: u16, decoded: SniffedDecode) {
        self.frames = self.frames.saturating_add(1);
        self.min_samples = self.min_samples.min(sample_count);
        self.max_samples = self.max_samples.max(sample_count);

        if decoded.phase_ticks != 0 && decoded.frame != SniffedFrame::DecodeFailed {
            self.phase_adjusted = self.phase_adjusted.saturating_add(1);
        }

        self.record_sample_length(sample_count);
        self.record_decoded_frame(sample_count, decoded.frame);
    }

    fn record_sample_length(&mut self, sample_count: u16) {
        let slot = match sample_count {
            BACKWARD8_SAMPLE_COUNT => &mut self.samples_backward8,
            FORWARD16_SAMPLE_COUNT => &mut self.samples_forward16,
            FORWARD24_SAMPLE_COUNT => &mut self.samples_forward24,
            _ => &mut self.samples_other,
        };
        *slot = slot.saturating_add(1);
    }

    fn record_decoded_frame(&mut self, sample_count: u16, frame: SniffedFrame) {
        let slot = match frame {
            SniffedFrame::Forward16(_) => &mut self.forward16,
            SniffedFrame::Forward24(_) => &mut self.forward24,
            SniffedFrame::Backward8(_) => &mut self.backward8,
            SniffedFrame::UnsupportedLength(_) => &mut self.unsupported_len,
            SniffedFrame::DecodeFailed => &mut self.decode_failed,
        };
        *slot = slot.saturating_add(1);
        if frame == SniffedFrame::DecodeFailed && sample_count == FORWARD16_SAMPLE_COUNT {
            self.decode_failed_forward16_len = self.decode_failed_forward16_len.saturating_add(1);
        }
    }

    pub(crate) fn account_isr_ticks(&mut self, now_us: i64, ticks: u32) -> (u32, u32, u32, u32) {
        let (missing, extra, raw_deficit, raw_surplus) = self.tick_ledger.account(now_us, ticks);
        self.isr_ticks_deficit_raw = self.isr_ticks_deficit_raw.saturating_add(raw_deficit);
        self.isr_ticks_surplus_raw = self.isr_ticks_surplus_raw.saturating_add(raw_surplus);
        if missing > 0 {
            self.isr_ticks_lost = self.isr_ticks_lost.saturating_add(missing);
            self.isr_tick_windows_short = self.isr_tick_windows_short.saturating_add(1);
            self.isr_last_short_window = (missing, now_us / i64::from(MICROS_PER_MILLI));
        }
        if extra > 0 {
            self.isr_ticks_extra = self.isr_ticks_extra.saturating_add(extra);
        }
        (missing, extra, raw_deficit, raw_surplus)
    }

    pub(crate) fn note_stage_delay(&mut self, ticks: u32, budget: u32) {
        self.stage_samples = self.stage_samples.saturating_add(1);
        self.stage_max_ticks = self.stage_max_ticks.max(ticks);
        if ticks > budget {
            self.stage_over_budget = self.stage_over_budget.saturating_add(1);
        }
    }

    pub(crate) fn note_poll(&mut self, now_us: i64, poll_ms: u32) -> Option<i64> {
        let gap = self.last_poll_us.map(|last| now_us.saturating_sub(last));
        if let Some(gap) = gap {
            self.poll_gap_max_us = self.poll_gap_max_us.max(gap);
            if dali2rust_platform::dali::sniffer_poll_is_late(gap_to_u32(gap), poll_ms) {
                self.poll_late = self.poll_late.saturating_add(1);
            }
        }
        self.last_poll_us = Some(now_us);
        gap
    }

    pub(crate) fn note_late_tick(&mut self, late: LateTick) {
        let task = late_task_key(late.interrupted_task, late.nesting_depth > 1);
        let entry = self.late_by_task.entry(task).or_default();
        entry.count = entry.count.saturating_add(1);
        if late.gap_us > entry.max_gap_us {
            entry.max_gap_us = late.gap_us;
            entry.pc_at_max = late.interrupted_pc;
            entry.ra_at_max = late.interrupted_ra;
        }
    }

    fn report_late_ticks(&mut self, scratch: &mut [u8]) {
        if self.late_by_task.is_empty() {
            return;
        }
        let mut text = LateReportLine::over(scratch);
        for (index, (task, tally)) in self.late_by_task.drain().enumerate() {
            if index > 0 {
                let _ = text.write_str(", ");
            }
            let _ = write!(
                text,
                "{task}×{} (max {} us @{:#010x}<{:#010x})",
                tally.count, tally.max_gap_us, tally.pc_at_max, tally.ra_at_max
            );
        }
        log::warn!(
            "DALI ISR late entries (delayed; see raw deficit for losses): {}",
            text.as_str()
        );
    }

    fn timing_is_notable(&self) -> bool {
        self.stage_over_budget > 0
            || self.poll_late > 0
            || self.isr_tick_windows_short > 0
            || self.isr_ticks_extra > 0
            || self.isr_ticks_raw_reported
                != (self.isr_ticks_deficit_raw, self.isr_ticks_surplus_raw)
    }

    fn report_timing(&mut self, late_ticks: u32, max_gap_us: u32) {
        if self.timing_is_notable() {
            self.isr_ticks_raw_reported =
                (self.isr_ticks_deficit_raw, self.isr_ticks_surplus_raw);
            let (missing, at_ms) = self.isr_last_short_window;
            log::warn!(
                "DALI sniff timing: isr ticks lost={} extra={} raw deficit={} surplus={} \
                 (short windows={}, last {} at {} ms) \
                 probe late={} max gap={} us stage max={} ticks over budget={} of {} poll gap max={} us",
                self.isr_ticks_lost,
                self.isr_ticks_extra,
                self.isr_ticks_deficit_raw,
                self.isr_ticks_surplus_raw,
                self.isr_tick_windows_short,
                missing,
                at_ms,
                late_ticks,
                max_gap_us,
                self.stage_max_ticks,
                self.stage_over_budget,
                self.stage_samples,
                self.poll_gap_max_us
            );
        }
        self.stage_max_ticks = 0;
        self.stage_over_budget = 0;
        self.stage_samples = 0;
        self.poll_gap_max_us = 0;
        self.poll_late = 0;
    }

    pub(crate) fn note_tx_holds(&mut self, yields: u32, voided: u32) {
        self.tx_holds = (yields, voided);
    }

    pub(crate) fn note_probe_counts(&mut self, counts: (u32, u32)) {
        self.probe_counts = counts;
    }

    pub(crate) fn note_line_held(&mut self, runs: u32, max_ticks: u32) {
        self.line_held.0 = self.line_held.0.saturating_add(runs);
        self.line_held.1 = self.line_held.1.max(max_ticks);
    }

    pub(crate) fn calibration_dump_due(&mut self, frame: SniffedFrame) -> bool {
        if !matches!(
            frame,
            SniffedFrame::Forward16(_) | SniffedFrame::Forward24(_) | SniffedFrame::Backward8(_)
        ) {
            return false;
        }
        self.since_calibration_dump = self.since_calibration_dump.saturating_add(1);
        if self.since_calibration_dump < CALIBRATION_DUMP_EVERY {
            return false;
        }
        self.since_calibration_dump = 0;
        true
    }

    fn report_line_held(&mut self) {
        let (runs, max_ticks) = self.line_held;
        self.line_held = (0, 0);
        if runs == 0 {
            return;
        }
        log::warn!(
            "DALI line held: {runs} run(s) longer than {} ticks, longest {max_ticks} ticks ({} ms)",
            LINE_HELD_TICKS,
            PHY_TICK_US.saturating_mul(max_ticks) / MICROS_PER_MILLI
        );
    }

    fn report_tx_holds(&mut self) {
        if self.tx_holds == self.last_tx_holds {
            return;
        }
        self.last_tx_holds = self.tx_holds;
        let (yields, voided) = self.tx_holds;
        log::info!("DALI PHY TX: lead yields={yields} held frames voided={voided}");
    }

    fn report_persist_overlap(&mut self) {
        let sample = dali2rust_platform::dali::persist_commit_overlap();
        if sample == self.last_persist_sample {
            return;
        }
        self.last_persist_sample = sample;
        let (total, during) = sample;
        let dali2rust_platform::dali::PersistGateStats { waits, wait_ms, timeouts } =
            dali2rust_platform::dali::persist_gate_stats();
        log::info!(
            "DALI persist overlap: {during} of {total} commit(s) landed while a frame was on the wire; \
             gate waits={waits} ({wait_ms} ms) timeouts={timeouts}"
        );
    }

    #[inline(never)]
    fn report_side_channels(&mut self, scratch: &mut [u8]) {
        self.report_line_held();
        self.report_persist_overlap();
        self.report_tx_holds();
        let (late_ticks, max_gap_us) = self.probe_counts;
        self.report_timing(late_ticks, max_gap_us);
        self.report_late_ticks(scratch);
    }

    pub(crate) fn maybe_report(&mut self, now: Instant, scratch: &mut [u8]) {
        if now.duration_since(self.last_report) < Duration::from_millis(SNIFFER_DIAG_INTERVAL_MS) {
            return;
        }
        self.report_interval(now, scratch);
    }

    #[inline(never)]
    fn report_interval(&mut self, now: Instant, scratch: &mut [u8]) {
        self.report_side_channels(scratch);
        if self.frames == 0 && self.dropped == 0 {
            self.last_report = now;
            return;
        }
        self.log_interval();
        self.start_interval(now);
    }

    #[inline(never)]
    fn log_interval(&self) {
        let min_samples = if self.frames == 0 {
            0
        } else {
            self.min_samples
        };
        log::info!(
            "DALI sniff diag: frames={} ok={{b8:{}, f16:{}, f24:{}}} failed={} (samples19={}) phase_adjusted={} unsupported={} dropped={} samples={{11:{}, 19:{}, 27:{}, other:{}, min:{}, max:{}}}",
            self.frames,
            self.backward8,
            self.forward16,
            self.forward24,
            self.decode_failed,
            self.decode_failed_forward16_len,
            self.phase_adjusted,
            self.unsupported_len,
            self.dropped,
            self.samples_backward8,
            self.samples_forward16,
            self.samples_forward24,
            self.samples_other,
            min_samples,
            self.max_samples
        );
    }

    #[inline(never)]
    fn start_interval(&mut self, now: Instant) {
        *self = Self {
            last_report: now,
            last_persist_sample: self.last_persist_sample,
            tick_ledger: self.tick_ledger,
            since_calibration_dump: self.since_calibration_dump,
            last_poll_us: self.last_poll_us,
            ..Self::new()
        };
    }
}

pub(crate) fn gap_to_u32(gap_us: i64) -> u32 {
    u32::try_from(gap_us.max(0)).unwrap_or(u32::MAX)
}

fn late_task_key(handle: usize, nested: bool) -> LateTaskKey {
    LateTaskKey {
        handle,
        name: (handle != 0)
            .then(|| dali2rust_bsp::task_registry::name_of(handle))
            .flatten(),
        nested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLL_MS: u32 = 10;
    const POLL_SLACK_LIMIT_US: i64 = 14_000;
    const FIVE_SECONDS: Duration = Duration::from_millis(SNIFFER_DIAG_INTERVAL_MS);

    type Trigger = fn(&mut SnifferDiagnostics);

    fn decode(frame: SniffedFrame, phase_ticks: i8) -> SniffedDecode {
        SniffedDecode { frame, phase_ticks, score: 0 }
    }

    fn late(task: usize, depth: u32, gap_us: u32, pc: u32, ra: u32) -> LateTick {
        LateTick {
            gap_us,
            at_us: 0,
            interrupted_task: task,
            interrupted_pc: pc,
            interrupted_ra: ra,
            nesting_depth: depth,
        }
    }

    fn key(handle: usize, nested: bool) -> LateTaskKey {
        LateTaskKey { handle, name: None, nested }
    }

    fn text_of(scratch: &[u8]) -> &str {
        let end = scratch.iter().position(|&b| b == 0).unwrap_or(scratch.len());
        core::str::from_utf8(&scratch[..end]).expect("the report text is UTF-8")
    }

    #[test]
    fn a_new_window_starts_empty() {
        let d = SnifferDiagnostics::new();
        assert_eq!((d.frames, d.dropped, d.max_samples), (0, 0, 0));
        assert_eq!(d.min_samples, u16::MAX);
        assert_eq!((d.line_held, d.tx_holds, d.last_tx_holds), ((0, 0), (0, 0), (0, 0)));
        assert_eq!((d.last_persist_sample, d.probe_counts), ((0, 0), (0, 0)));
        assert_eq!((d.last_poll_us, d.since_calibration_dump), (None, 0));
        assert!(d.late_by_task.is_empty());
    }

    #[test]
    fn dropped_frames_add_up_and_stop_at_the_ceiling() {
        let mut d = SnifferDiagnostics::new();
        d.record_dropped(2);
        d.record_dropped(3);
        assert_eq!(d.dropped, 5);
        d.record_dropped(u32::MAX);
        assert_eq!(d.dropped, u32::MAX);
    }

    #[test]
    fn every_decoded_kind_lands_in_its_own_counter() {
        let mut d = SnifferDiagnostics::new();
        d.record_frame(19, decode(SniffedFrame::Forward16(0x82A9), 0));
        d.record_frame(27, decode(SniffedFrame::Forward24([1, 2, 3]), 0));
        d.record_frame(27, decode(SniffedFrame::Forward24([4, 5, 6]), 0));
        d.record_frame(11, decode(SniffedFrame::Backward8(0x55), 0));
        d.record_frame(23, decode(SniffedFrame::UnsupportedLength(12), 0));
        d.record_frame(19, decode(SniffedFrame::DecodeFailed, 0));
        assert_eq!(d.frames, 6);
        assert_eq!((d.forward16, d.forward24, d.backward8), (1, 2, 1));
        assert_eq!((d.unsupported_len, d.decode_failed), (1, 1));
    }

    #[test]
    fn the_sample_length_buckets_follow_the_frame_lengths() {
        let mut d = SnifferDiagnostics::new();
        for count in [BACKWARD8_SAMPLE_COUNT, FORWARD16_SAMPLE_COUNT, FORWARD16_SAMPLE_COUNT] {
            d.record_frame(count, decode(SniffedFrame::DecodeFailed, 0));
        }
        d.record_frame(FORWARD24_SAMPLE_COUNT, decode(SniffedFrame::DecodeFailed, 0));
        d.record_frame(BACKWARD8_SAMPLE_COUNT + 1, decode(SniffedFrame::DecodeFailed, 0));
        d.record_frame(0, decode(SniffedFrame::DecodeFailed, 0));
        assert_eq!(d.samples_backward8, 1);
        assert_eq!(d.samples_forward16, 2);
        assert_eq!(d.samples_forward24, 1);
        assert_eq!(d.samples_other, 2);
    }

    #[test]
    fn the_shortest_and_longest_captures_are_kept() {
        let mut d = SnifferDiagnostics::new();
        for count in [19u16, 11, 27, 19] {
            d.record_frame(count, decode(SniffedFrame::DecodeFailed, 0));
        }
        assert_eq!((d.min_samples, d.max_samples), (11, 27));
    }

    #[test]
    fn a_phase_shift_counts_as_an_adjustment_only_for_a_frame_that_decoded() {
        let cases = [
            (SniffedFrame::Forward16(1), 0i8, 0u32),
            (SniffedFrame::Forward16(1), 2, 1),
            (SniffedFrame::Backward8(1), -3, 1),
            (SniffedFrame::Forward24([1, 2, 3]), 1, 1),
            (SniffedFrame::UnsupportedLength(12), 1, 1),
            (SniffedFrame::DecodeFailed, 4, 0),
            (SniffedFrame::DecodeFailed, 0, 0),
        ];
        for (frame, phase, adjusted) in cases {
            let mut d = SnifferDiagnostics::new();
            d.record_frame(19, decode(frame, phase));
            assert_eq!(d.phase_adjusted, adjusted, "{frame:?} at phase {phase}");
        }
    }

    #[test]
    fn a_failed_decode_is_blamed_on_the_forward16_length_only_at_that_length() {
        let mut d = SnifferDiagnostics::new();
        d.record_frame(FORWARD16_SAMPLE_COUNT, decode(SniffedFrame::DecodeFailed, 0));
        assert_eq!(d.decode_failed_forward16_len, 1);
        d.record_frame(FORWARD16_SAMPLE_COUNT - 1, decode(SniffedFrame::DecodeFailed, 0));
        d.record_frame(FORWARD16_SAMPLE_COUNT + 1, decode(SniffedFrame::DecodeFailed, 0));
        d.record_frame(FORWARD16_SAMPLE_COUNT, decode(SniffedFrame::Forward16(7), 0));
        assert_eq!(d.decode_failed_forward16_len, 1);
        assert_eq!(d.decode_failed, 3);
    }

    #[test]
    fn counters_stop_at_the_ceiling() {
        let mut d = SnifferDiagnostics::new();
        d.frames = u32::MAX;
        d.forward16 = u32::MAX;
        d.samples_forward16 = u32::MAX;
        d.phase_adjusted = u32::MAX;
        d.record_frame(FORWARD16_SAMPLE_COUNT, decode(SniffedFrame::Forward16(7), 1));
        assert_eq!(
            (d.frames, d.forward16, d.samples_forward16, d.phase_adjusted),
            (u32::MAX, u32::MAX, u32::MAX, u32::MAX)
        );
    }

    #[test]
    fn a_short_window_books_lost_ticks_and_remembers_when_it_happened() {
        let mut d = SnifferDiagnostics::new();
        assert_eq!(d.account_isr_ticks(0, 0), (0, 0, 0, 0));
        assert_eq!(d.account_isr_ticks(1_040_000, 9_998), (1, 0, 2, 0));
        assert_eq!((d.isr_ticks_lost, d.isr_tick_windows_short), (1, 1));
        assert_eq!(d.isr_last_short_window, (1, 1_040));
        assert_eq!((d.isr_ticks_deficit_raw, d.isr_ticks_surplus_raw), (2, 0));
        assert_eq!(d.isr_ticks_extra, 0);
    }

    #[test]
    fn a_surplus_window_books_extra_ticks_and_no_loss() {
        let mut d = SnifferDiagnostics::new();
        d.account_isr_ticks(0, 0);
        assert_eq!(d.account_isr_ticks(1_040_000, 10_003), (0, 2, 0, 3));
        assert_eq!((d.isr_ticks_extra, d.isr_ticks_surplus_raw), (2, 3));
        assert_eq!((d.isr_ticks_lost, d.isr_tick_windows_short), (0, 0));
        assert_eq!(d.isr_last_short_window, (0, 0));
    }

    #[test]
    fn a_window_that_ticked_on_time_books_nothing() {
        let mut d = SnifferDiagnostics::new();
        d.account_isr_ticks(0, 0);
        assert_eq!(d.account_isr_ticks(1_040_000, 10_000), (0, 0, 0, 0));
        assert_eq!(d.account_isr_ticks(2_080_000, 20_000), (0, 0, 0, 0));
        assert_eq!((d.isr_ticks_lost, d.isr_ticks_extra), (0, 0));
        assert_eq!((d.isr_ticks_deficit_raw, d.isr_ticks_surplus_raw), (0, 0));
    }

    #[test]
    fn raw_totals_and_lost_ticks_add_up_across_windows() {
        let mut d = SnifferDiagnostics::new();
        d.account_isr_ticks(0, 0);
        d.account_isr_ticks(1_040_000, 9_998);
        d.account_isr_ticks(3_120_000, 29_997);
        assert_eq!((d.isr_ticks_lost, d.isr_tick_windows_short), (2, 2));
        assert_eq!((d.isr_ticks_deficit_raw, d.isr_ticks_surplus_raw), (3, 0));
        assert_eq!(d.isr_last_short_window, (1, 3_120));
    }

    #[test]
    fn stage_delay_is_over_budget_only_when_strictly_over() {
        let mut d = SnifferDiagnostics::new();
        d.note_stage_delay(5, 5);
        assert_eq!((d.stage_samples, d.stage_max_ticks, d.stage_over_budget), (1, 5, 0));
        d.note_stage_delay(6, 5);
        assert_eq!((d.stage_samples, d.stage_max_ticks, d.stage_over_budget), (2, 6, 1));
        d.note_stage_delay(3, 5);
        assert_eq!((d.stage_samples, d.stage_max_ticks, d.stage_over_budget), (3, 6, 1));
    }

    #[test]
    fn the_first_poll_has_no_gap() {
        let mut d = SnifferDiagnostics::new();
        assert_eq!(d.note_poll(5_000, POLL_MS), None);
        assert_eq!(d.last_poll_us, Some(5_000));
        assert_eq!((d.poll_gap_max_us, d.poll_late), (0, 0));
    }

    #[test]
    fn a_later_poll_reports_its_gap_and_the_longest_gap_is_kept() {
        let mut d = SnifferDiagnostics::new();
        d.note_poll(1_000, POLL_MS);
        assert_eq!(d.note_poll(11_000, POLL_MS), Some(10_000));
        assert_eq!(d.note_poll(14_000, POLL_MS), Some(3_000));
        assert_eq!(d.poll_gap_max_us, 10_000);
        assert_eq!(d.last_poll_us, Some(14_000));
    }

    #[test]
    fn a_poll_is_late_only_beyond_the_pace_and_the_slack() {
        for (gap, late_polls) in [(POLL_SLACK_LIMIT_US, 0u32), (POLL_SLACK_LIMIT_US + 1, 1)] {
            let mut d = SnifferDiagnostics::new();
            d.note_poll(0, POLL_MS);
            d.note_poll(gap, POLL_MS);
            assert_eq!(d.poll_late, late_polls, "gap {gap} us at {POLL_MS} ms");
        }
        let mut fast = SnifferDiagnostics::new();
        fast.note_poll(0, 1);
        fast.note_poll(5_001, 1);
        assert_eq!(fast.poll_late, 1, "the slack is added to the pace in force");
    }

    #[test]
    fn a_clock_that_ran_backwards_is_a_negative_gap_and_never_late() {
        let mut d = SnifferDiagnostics::new();
        d.note_poll(100_000, POLL_MS);
        assert_eq!(d.note_poll(90_000, POLL_MS), Some(-10_000));
        assert_eq!((d.poll_gap_max_us, d.poll_late), (0, 0));
        assert_eq!(d.last_poll_us, Some(90_000));
    }

    #[test]
    fn a_gap_is_clamped_into_u32() {
        assert_eq!(gap_to_u32(-5), 0);
        assert_eq!(gap_to_u32(0), 0);
        assert_eq!(gap_to_u32(14_000), 14_000);
        assert_eq!(gap_to_u32(i64::from(u32::MAX)), u32::MAX);
        assert_eq!(gap_to_u32(i64::from(u32::MAX) + 1), u32::MAX);
        assert_eq!(gap_to_u32(i64::MAX), u32::MAX);
    }

    #[test]
    fn late_ticks_are_tallied_per_interrupted_task_with_the_worst_site_kept() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0x10, 1, 500, 0xA, 0xB));
        d.note_late_tick(late(0x10, 1, 300, 0xC, 0xD));
        d.note_late_tick(late(0x20, 1, 100, 0xE, 0xF));
        let first = d.late_by_task[&key(0x10, false)];
        assert_eq!((first.count, first.max_gap_us, first.pc_at_max, first.ra_at_max), (2, 500, 0xA, 0xB));
        let second = d.late_by_task[&key(0x20, false)];
        assert_eq!((second.count, second.max_gap_us, second.pc_at_max), (1, 100, 0xE));
        assert_eq!(d.late_by_task.len(), 2);
    }

    #[test]
    fn only_a_strictly_longer_gap_moves_the_recorded_site() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0x10, 1, 500, 0xA, 0xB));
        d.note_late_tick(late(0x10, 1, 500, 0xC, 0xD));
        assert_eq!(d.late_by_task[&key(0x10, false)].pc_at_max, 0xA);
        d.note_late_tick(late(0x10, 1, 501, 0xC, 0xD));
        let tally = d.late_by_task[&key(0x10, false)];
        assert_eq!((tally.count, tally.max_gap_us, tally.pc_at_max, tally.ra_at_max), (3, 501, 0xC, 0xD));
    }

    #[test]
    fn a_tick_that_interrupted_an_interrupt_is_tallied_apart_from_one_that_did_not() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0x10, 0, 100, 1, 1));
        d.note_late_tick(late(0x10, 1, 100, 1, 1));
        assert_eq!(d.late_by_task.len(), 1, "depth 0 and 1 are both task level");
        d.note_late_tick(late(0x10, 2, 100, 1, 1));
        assert_eq!(d.late_by_task.len(), 2);
        assert_eq!(d.late_by_task[&key(0x10, false)].count, 2);
        assert_eq!(d.late_by_task[&key(0x10, true)].count, 1);
    }

    #[test]
    fn a_task_key_names_the_task_the_handle_or_nothing_and_marks_nesting() {
        let invalid = c"\xff";
        let cases = [
            (key(0, false), "(none)"),
            (key(0, true), "(none)+isr"),
            (key(0x3FC1_2345, false), "0x3fc12345 (not registered)"),
            (key(0x3FC1_2345, true), "0x3fc12345 (not registered)+isr"),
            (LateTaskKey { handle: 7, name: Some(c"dali_worker"), nested: false }, "dali_worker"),
            (LateTaskKey { handle: 7, name: Some(c"dali_worker"), nested: true }, "dali_worker+isr"),
            (LateTaskKey { handle: 7, name: Some(invalid), nested: false }, "<invalid-task-name>"),
        ];
        for (task, text) in cases {
            assert_eq!(task.to_string(), text);
        }
    }

    #[test]
    fn a_task_key_asks_the_registry_only_for_a_real_handle() {
        assert_eq!(late_task_key(0, false), key(0, false));
        assert_eq!(late_task_key(0x42, true), key(0x42, true));
    }

    #[test]
    fn one_late_task_is_written_to_the_scratch_and_the_tally_is_drained() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0, 1, 700, 0x10, 0x20));
        d.note_late_tick(late(0, 1, 200, 0x30, 0x40));
        let mut scratch = [0u8; 128];
        d.report_late_ticks(&mut scratch);
        assert_eq!(text_of(&scratch), "(none)×2 (max 700 us @0x00000010<0x00000020)");
        assert!(d.late_by_task.is_empty());
    }

    #[test]
    fn several_late_tasks_share_one_line_separated_by_commas() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0, 1, 700, 0x10, 0x20));
        d.note_late_tick(late(0x77, 2, 90, 0x50, 0x60));
        let mut scratch = [0u8; 256];
        d.report_late_ticks(&mut scratch);
        let text = text_of(&scratch);
        assert_eq!(text.matches(", ").count(), 1, "{text}");
        assert!(text.contains("(none)×1 (max 700 us @0x00000010<0x00000020)"), "{text}");
        assert!(text.contains("0x00000077 (not registered)+isr×1 (max 90 us @0x00000050<0x00000060)"), "{text}");
    }

    #[test]
    fn a_scratch_that_is_too_small_is_filled_to_the_brim_and_no_further() {
        let mut d = SnifferDiagnostics::new();
        d.note_late_tick(late(0, 1, 700, 0x10, 0x20));
        let mut scratch = [0u8; 9];
        d.report_late_ticks(&mut scratch);
        assert_eq!(&scratch, &"(none)×1 (max 700 us @0x00000010<0x00000020)".as_bytes()[..9]);
    }

    #[test]
    fn nothing_late_leaves_the_scratch_alone() {
        let mut d = SnifferDiagnostics::new();
        let mut scratch = [0xAAu8; 16];
        d.report_late_ticks(&mut scratch);
        assert_eq!(scratch, [0xAAu8; 16]);
    }

    #[test]
    fn each_condition_alone_makes_the_timing_worth_reporting() {
        let triggers: [(&str, Trigger); 6] = [
            ("stage over budget", |d| d.stage_over_budget = 1),
            ("late poll", |d| d.poll_late = 1),
            ("short window", |d| d.isr_tick_windows_short = 1),
            ("extra ticks", |d| d.isr_ticks_extra = 1),
            ("deficit moved", |d| d.isr_ticks_deficit_raw = 1),
            ("surplus moved", |d| d.isr_ticks_surplus_raw = 1),
        ];
        assert!(!SnifferDiagnostics::new().timing_is_notable());
        for (name, trigger) in triggers {
            let mut d = SnifferDiagnostics::new();
            trigger(&mut d);
            assert!(d.timing_is_notable(), "{name}");
        }
    }

    #[test]
    fn a_raw_total_already_reported_is_not_worth_reporting_again() {
        let mut d = SnifferDiagnostics::new();
        d.isr_ticks_deficit_raw = 4;
        d.isr_ticks_surplus_raw = 5;
        d.isr_ticks_raw_reported = (4, 5);
        assert!(!d.timing_is_notable());
        d.isr_ticks_raw_reported = (4, 6);
        assert!(d.timing_is_notable());
        d.isr_ticks_raw_reported = (3, 5);
        assert!(d.timing_is_notable());
    }

    #[test]
    fn a_notable_timing_report_marks_the_raw_totals_and_clears_the_window() {
        let mut d = SnifferDiagnostics::new();
        d.isr_ticks_deficit_raw = 4;
        d.isr_ticks_surplus_raw = 5;
        d.isr_ticks_lost = 8;
        d.stage_max_ticks = 9;
        d.stage_over_budget = 2;
        d.stage_samples = 7;
        d.poll_gap_max_us = 123;
        d.poll_late = 1;
        d.report_timing(3, 100);
        assert_eq!(d.isr_ticks_raw_reported, (4, 5));
        assert_eq!((d.stage_max_ticks, d.stage_over_budget, d.stage_samples), (0, 0, 0));
        assert_eq!((d.poll_gap_max_us, d.poll_late), (0, 0));
        assert_eq!((d.isr_ticks_lost, d.isr_ticks_deficit_raw), (8, 4));
        assert!(!d.timing_is_notable());
    }

    #[test]
    fn a_quiet_timing_report_still_clears_the_window_but_leaves_the_raw_mark() {
        let mut d = SnifferDiagnostics::new();
        d.isr_ticks_raw_reported = (2, 2);
        d.isr_ticks_deficit_raw = 2;
        d.isr_ticks_surplus_raw = 2;
        d.stage_max_ticks = 9;
        d.stage_samples = 7;
        d.poll_gap_max_us = 1_000;
        d.report_timing(0, 0);
        assert_eq!(d.isr_ticks_raw_reported, (2, 2));
        assert_eq!((d.stage_max_ticks, d.stage_samples, d.poll_gap_max_us), (0, 0, 0));
    }

    #[test]
    fn an_unchanged_tx_hold_sample_is_reported_once() {
        let mut d = SnifferDiagnostics::new();
        d.report_tx_holds();
        assert_eq!(d.last_tx_holds, (0, 0));
        d.note_tx_holds(3, 1);
        d.report_tx_holds();
        assert_eq!(d.last_tx_holds, (3, 1));
        d.note_tx_holds(3, 2);
        d.report_tx_holds();
        assert_eq!(d.last_tx_holds, (3, 2));
        d.report_tx_holds();
        assert_eq!((d.tx_holds, d.last_tx_holds), ((3, 2), (3, 2)));
    }

    #[test]
    fn line_holds_add_up_and_the_longest_is_kept_until_reported() {
        let mut d = SnifferDiagnostics::new();
        d.note_line_held(2, 30);
        d.note_line_held(1, 40);
        d.note_line_held(1, 10);
        assert_eq!(d.line_held, (4, 40));
        d.note_line_held(u32::MAX, 5);
        assert_eq!(d.line_held, (u32::MAX, 40));
        d.report_line_held();
        assert_eq!(d.line_held, (0, 0));
        d.report_line_held();
        assert_eq!(d.line_held, (0, 0));
        d.note_line_held(1, 7);
        d.report_line_held();
        assert_eq!(d.line_held, (0, 0), "a single run is reported too");
    }

    #[test]
    fn a_persist_sample_that_moved_is_remembered_and_one_that_did_not_is_left() {
        let mut d = SnifferDiagnostics::new();
        d.last_persist_sample = (u32::MAX, u32::MAX);
        d.report_persist_overlap();
        let now = dali2rust_platform::dali::persist_commit_overlap();
        assert_eq!(d.last_persist_sample, now);
        d.report_persist_overlap();
        assert_eq!(d.last_persist_sample, now);
    }

    #[test]
    fn only_decoded_frames_count_towards_the_calibration_dump() {
        let mut d = SnifferDiagnostics::new();
        assert!(!d.calibration_dump_due(SniffedFrame::DecodeFailed));
        assert!(!d.calibration_dump_due(SniffedFrame::UnsupportedLength(12)));
        assert_eq!(d.since_calibration_dump, 0);
        for frame in [SniffedFrame::Forward16(1), SniffedFrame::Forward24([1, 2, 3]), SniffedFrame::Backward8(1)] {
            assert!(!d.calibration_dump_due(frame));
        }
        assert_eq!(d.since_calibration_dump, 3);
    }

    #[test]
    fn every_256th_decoded_frame_asks_for_a_calibration_dump() {
        let mut d = SnifferDiagnostics::new();
        for round in 0..2 {
            for n in 1..CALIBRATION_DUMP_EVERY {
                assert!(!d.calibration_dump_due(SniffedFrame::Backward8(1)), "round {round} frame {n}");
            }
            assert!(d.calibration_dump_due(SniffedFrame::Forward16(1)), "round {round}");
            assert_eq!(d.since_calibration_dump, 0);
        }
    }

    fn a_window_with_everything_set() -> SnifferDiagnostics {
        let mut d = SnifferDiagnostics::new();
        d.record_frame(19, decode(SniffedFrame::Forward16(1), 1));
        d.record_dropped(2);
        d.line_held = (2, 30);
        d.isr_ticks_lost = 7;
        d.isr_tick_windows_short = 1;
        d.isr_last_short_window = (7, 99);
        d.isr_ticks_extra = 3;
        d.isr_ticks_deficit_raw = 4;
        d.isr_ticks_surplus_raw = 5;
        d.isr_ticks_raw_reported = (4, 5);
        d.stage_max_ticks = 9;
        d.poll_gap_max_us = 5_000;
        d.probe_counts = (6, 800);
        d.note_late_tick(late(0, 1, 100, 1, 2));
        d.tx_holds = (3, 1);
        d.last_tx_holds = (3, 1);
        d
    }

    #[test]
    fn a_new_window_keeps_the_persist_sample_the_ledger_the_dump_count_and_the_last_poll() {
        let mut d = SnifferDiagnostics::new();
        d.account_isr_ticks(0, 0);
        d.account_isr_ticks(1_040_000, 9_998);
        d.last_persist_sample = (11, 12);
        d.since_calibration_dump = 5;
        d.last_poll_us = Some(123);
        let ledger = format!("{:?}", d.tick_ledger);
        let now = d.last_report + FIVE_SECONDS;
        d.start_interval(now);
        assert_eq!(d.last_report, now);
        assert_eq!(d.last_persist_sample, (11, 12));
        assert_eq!(format!("{:?}", d.tick_ledger), ledger);
        assert_eq!((d.since_calibration_dump, d.last_poll_us), (5, Some(123)));
    }

    #[test]
    fn a_new_window_clears_every_other_counter() {
        let mut d = a_window_with_everything_set();
        d.start_interval(d.last_report + FIVE_SECONDS);
        assert_eq!((d.frames, d.forward16, d.phase_adjusted, d.dropped), (0, 0, 0, 0));
        assert_eq!((d.min_samples, d.max_samples), (u16::MAX, 0));
        assert_eq!((d.line_held, d.probe_counts), ((0, 0), (0, 0)));
        assert_eq!((d.isr_ticks_lost, d.isr_tick_windows_short, d.isr_ticks_extra), (0, 0, 0));
        assert_eq!((d.isr_ticks_deficit_raw, d.isr_ticks_surplus_raw), (0, 0));
        assert_eq!((d.isr_last_short_window, d.isr_ticks_raw_reported), ((0, 0), (0, 0)));
        assert_eq!((d.stage_max_ticks, d.poll_gap_max_us), (0, 0));
        assert_eq!((d.tx_holds, d.last_tx_holds), ((0, 0), (0, 0)));
        assert!(d.late_by_task.is_empty());
    }

    #[test]
    fn nothing_is_reported_until_the_interval_has_elapsed() {
        let mut d = a_window_with_everything_set();
        let before = d.last_report;
        d.maybe_report(before + FIVE_SECONDS - Duration::from_millis(1), &mut [0u8; 64]);
        assert_eq!(d.last_report, before);
        assert_eq!((d.frames, d.dropped, d.line_held), (1, 2, (2, 30)));
        assert_eq!(d.late_by_task.len(), 1);
        assert_eq!(d.stage_max_ticks, 9);
    }

    #[test]
    fn the_interval_is_reported_at_exactly_five_seconds() {
        let mut d = a_window_with_everything_set();
        let now = d.last_report + FIVE_SECONDS;
        d.maybe_report(now, &mut [0u8; 64]);
        assert_eq!(d.last_report, now);
        assert_eq!((d.frames, d.dropped, d.line_held), (0, 0, (0, 0)));
    }

    #[test]
    fn a_window_with_only_frames_or_only_drops_is_reported_and_restarted() {
        for (frames, dropped) in [(1u32, 0u32), (0, 2)] {
            let mut d = SnifferDiagnostics::new();
            d.frames = frames;
            d.dropped = dropped;
            d.isr_ticks_lost = 7;
            d.maybe_report(d.last_report + FIVE_SECONDS, &mut [0u8; 64]);
            assert_eq!((d.frames, d.dropped), (0, 0), "frames {frames} dropped {dropped}");
            assert_eq!(d.isr_ticks_lost, 0, "the window restarted");
        }
    }

    #[test]
    fn a_quiet_interval_reports_the_side_channels_and_keeps_the_running_totals() {
        let mut d = a_window_with_everything_set();
        d.frames = 0;
        d.dropped = 0;
        let now = d.last_report + FIVE_SECONDS;
        d.maybe_report(now, &mut [0u8; 64]);
        assert_eq!(d.last_report, now);
        assert_eq!(d.line_held, (0, 0));
        assert!(d.late_by_task.is_empty());
        assert_eq!(d.stage_max_ticks, 0);
        assert_eq!((d.isr_ticks_lost, d.isr_tick_windows_short, d.isr_ticks_extra), (7, 1, 3));
        assert_eq!((d.min_samples, d.last_tx_holds), (19, (3, 1)));
    }

    #[test]
    fn the_side_channels_each_run_when_an_interval_is_reported() {
        let mut d = SnifferDiagnostics::new();
        d.last_persist_sample = (u32::MAX, u32::MAX);
        d.line_held = (1, 25);
        d.tx_holds = (4, 2);
        d.isr_ticks_deficit_raw = 1;
        d.note_late_tick(late(0, 1, 100, 1, 2));
        d.report_side_channels(&mut [0u8; 64]);
        assert_eq!(d.last_persist_sample, dali2rust_platform::dali::persist_commit_overlap());
        assert_eq!((d.line_held, d.last_tx_holds), ((0, 0), (4, 2)));
        assert_eq!(d.isr_ticks_raw_reported, (1, 0));
        assert!(d.late_by_task.is_empty());
    }
}
