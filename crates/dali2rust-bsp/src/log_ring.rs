use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use dali2rust_platform::logs::{LogLevel, LogLine};

use crate::psram::PsramBox;

pub const LOG_RING_LINES: usize = 256;

pub const LOG_REPLAY_LINES: u32 = LOG_RING_LINES as u32;

pub const ARMED_LEVEL: LogLevel = LogLevel::Warn;

const NO_REPLAY: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    pub next_cursor: u32,
    pub missed: u32,
}

struct RingInner {
    arena: PsramBox<[LogLine; LOG_RING_LINES]>,
    write_seq: u32,
}

pub struct LogRing {
    inner: Mutex<RingInner>,
    min_level: AtomicU8,
    dropped: AtomicU32,
    replay_from: AtomicU32,
}

impl LogRing {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RingInner {
                arena: empty_arena(),
                write_seq: 0,
            }),
            min_level: AtomicU8::new(ARMED_LEVEL as u8),
            dropped: AtomicU32::new(0),
            replay_from: AtomicU32::new(NO_REPLAY),
        }
    }

    pub fn accepts(&self, level: LogLevel) -> bool {
        (level as u8) <= self.min_level.load(Ordering::Relaxed)
    }

    pub fn min_level(&self) -> u8 {
        self.min_level.load(Ordering::Relaxed)
    }

    pub fn set_min_level(&self, level: LogLevel) {
        self.min_level.store(level as u8, Ordering::Relaxed);
    }

    pub fn record(&self, level: LogLevel, target: &[u8], text: &[u8], at_ms: u64) {
        if !self.accepts(level) {
            return;
        }
        let Ok(mut inner) = self.inner.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let seq = inner.write_seq;
        let slot = (seq as usize) % LOG_RING_LINES;
        inner.arena[slot] = LogLine::new(at_ms, seq, level, target, text);
        inner.write_seq = seq.wrapping_add(1);
    }

    pub fn dropped_total(&self) -> u32 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn request_replay(&self, depth: u32) {
        let write_seq = self.inner.lock().map_or(0, |inner| inner.write_seq);
        let from = write_seq.saturating_sub(depth);
        self.replay_from.fetch_min(from, Ordering::Relaxed);
    }

    pub fn take_replay(&self) -> Option<u32> {
        match self.replay_from.swap(NO_REPLAY, Ordering::Relaxed) {
            NO_REPLAY => None,
            from => Some(from),
        }
    }

    pub fn drain_from(&self, cursor: u32, max: usize, out: &mut Vec<LogLine>) -> DrainOutcome {
        let Ok(inner) = self.inner.lock() else {
            return DrainOutcome { next_cursor: cursor, missed: 0 };
        };
        let oldest = inner.write_seq.saturating_sub(LOG_RING_LINES as u32);
        let missed = oldest.saturating_sub(cursor);
        let mut seq = cursor.max(oldest);
        while seq < inner.write_seq && out.len() < max {
            out.push(inner.arena[(seq as usize) % LOG_RING_LINES]);
            seq = seq.wrapping_add(1);
        }
        DrainOutcome { next_cursor: seq, missed }
    }

    pub fn head(&self) -> u32 {
        self.inner.lock().map_or(0, |inner| inner.write_seq)
    }
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

fn empty_arena() -> PsramBox<[LogLine; LOG_RING_LINES]> {
    // SAFETY: each of the `LOG_RING_LINES` elements is written exactly once below, so the place is initialised.
    unsafe {
        PsramBox::new_with(|place: &mut core::mem::MaybeUninit<[LogLine; LOG_RING_LINES]>| {
            let first = place.as_mut_ptr().cast::<LogLine>();
            let blank = LogLine::new(0, 0, ARMED_LEVEL, b"", b"");
            for index in 0..LOG_RING_LINES {
                first.add(index).write(blank);
            }
        })
    }
}

static GLOBAL: OnceLock<LogRing> = OnceLock::new();

pub fn global() -> &'static LogRing {
    GLOBAL.get_or_init(LogRing::new)
}

pub fn try_global() -> Option<&'static LogRing> {
    GLOBAL.get()
}

pub fn install_capture_source() {
    #[cfg(not(target_os = "espidf"))]
    host_logger::install();
    #[cfg(target_os = "espidf")]
    {
        let _ = global();
    }
}

struct FixedBuf {
    bytes: [u8; dali2rust_platform::logs::LOG_TEXT_BYTES],
    len: usize,
}

impl FixedBuf {
    const fn new() -> Self {
        Self { bytes: [0; dali2rust_platform::logs::LOG_TEXT_BYTES], len: 0 }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl core::fmt::Write for FixedBuf {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let room = self.bytes.len() - self.len;
        let take = room.min(text.len());
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

pub const fn facade_level(level: log::Level) -> LogLevel {
    match level {
        log::Level::Error => LogLevel::Error,
        log::Level::Warn => LogLevel::Warn,
        log::Level::Info => LogLevel::Info,
        log::Level::Debug => LogLevel::Debug,
        log::Level::Trace => LogLevel::Trace,
    }
}

pub fn facade_accepts(level: log::Level) -> bool {
    try_global().is_some_and(|ring| ring.accepts(facade_level(level)))
}

pub fn record_facade(record: &log::Record) {
    use core::fmt::Write;

    let Some(ring) = try_global() else {
        return;
    };
    let level = facade_level(record.level());
    if !ring.accepts(level) {
        return;
    }
    let mut message = FixedBuf::new();
    let _ = write!(message, "{}", record.args());
    ring.record(
        level,
        record.target().as_bytes(),
        message.as_bytes(),
        dali2rust_platform::dali::wall_clock_millis(),
    );
}

#[cfg(not(target_os = "espidf"))]
pub mod host_logger {
    use super::{facade_accepts, global, record_facade};

    struct RingLogger;

    impl log::Log for RingLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            facade_accepts(metadata.level())
        }

        fn log(&self, record: &log::Record) {
            record_facade(record);
        }

        fn flush(&self) {}
    }

    static LOGGER: RingLogger = RingLogger;

    pub fn install() {
        let _ = global();
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use dali2rust_platform::logs::LogLevel;

    use super::{LogRing, ARMED_LEVEL, LOG_RING_LINES, NO_REPLAY};

    fn record(ring: &LogRing, level: LogLevel, text: &str) {
        ring.record(level, b"test", text.as_bytes(), 0);
    }

    fn drained(ring: &LogRing, cursor: u32, max: usize) -> (Vec<String>, u32) {
        let mut out = Vec::new();
        let outcome = ring.drain_from(cursor, max, &mut out);
        (
            out.iter().map(|line| line.text_str().to_owned()).collect(),
            outcome.missed,
        )
    }

    #[test]
    fn an_unsubscribed_ring_still_records_at_the_armed_level() {
        let ring = LogRing::new();
        assert_eq!(ring.min_level(), ARMED_LEVEL as u8);

        record(&ring, LogLevel::Error, "boom");
        record(&ring, LogLevel::Info, "chatter");

        let (lines, _) = drained(&ring, 0, 16);
        assert_eq!(lines, vec!["boom".to_owned()], "info is below the armed level");
    }

    #[test]
    fn raising_the_level_admits_what_it_names_and_nothing_quieter() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);

        record(&ring, LogLevel::Info, "kept");
        record(&ring, LogLevel::Debug, "dropped");

        let (lines, _) = drained(&ring, 0, 16);
        assert_eq!(lines, vec!["kept".to_owned()]);
    }

    #[test]
    fn a_wrap_overwrites_the_oldest_and_reports_the_gap() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        for index in 0..(LOG_RING_LINES + 10) {
            record(&ring, LogLevel::Info, &format!("line{index}"));
        }

        let (lines, missed) = drained(&ring, 0, LOG_RING_LINES + 32);
        assert_eq!(missed, 10, "ten lines were overwritten before the drain");
        assert_eq!(lines.len(), LOG_RING_LINES);
        assert_eq!(lines.first().expect("oldest kept"), "line10");
        assert_eq!(lines.last().expect("newest"), &format!("line{}", LOG_RING_LINES + 9));
    }

    #[test]
    fn a_drain_resumes_where_the_last_one_stopped() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        for index in 0..5 {
            record(&ring, LogLevel::Info, &format!("line{index}"));
        }

        let mut out = Vec::new();
        let first = ring.drain_from(0, 2, &mut out);
        assert_eq!(first.next_cursor, 2);
        out.clear();
        let second = ring.drain_from(first.next_cursor, 99, &mut out);
        assert_eq!(second.next_cursor, 5);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text_str(), "line2");
    }

    #[test]
    fn a_contended_ring_counts_a_drop_instead_of_blocking() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        let held = ring.inner.lock().expect("hold the ring");

        record(&ring, LogLevel::Info, "lost");
        assert_eq!(ring.dropped_total(), 1);

        drop(held);
        record(&ring, LogLevel::Info, "kept");
        assert_eq!(ring.dropped_total(), 1);
        let (lines, _) = drained(&ring, 0, 16);
        assert_eq!(lines, vec!["kept".to_owned()]);
    }

    #[test]
    fn a_replay_request_rewinds_at_most_its_depth_and_is_one_shot() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        for index in 0..10 {
            record(&ring, LogLevel::Info, &format!("line{index}"));
        }

        ring.request_replay(4);
        assert_eq!(ring.take_replay(), Some(6));
        assert_eq!(ring.take_replay(), None, "a rewind is consumed once");
        assert_eq!(ring.replay_from.load(Ordering::Relaxed), NO_REPLAY);
    }

    #[test]
    fn the_deepest_pending_rewind_wins() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        for index in 0..20 {
            record(&ring, LogLevel::Info, &format!("line{index}"));
        }

        ring.request_replay(2);
        ring.request_replay(15);
        assert_eq!(ring.take_replay(), Some(5));
    }

    #[test]
    fn a_fresh_cursor_starts_at_the_head_and_sees_no_history() {
        let ring = LogRing::new();
        ring.set_min_level(LogLevel::Info);
        record(&ring, LogLevel::Info, "before");

        let head = ring.head();
        let (lines, missed) = drained(&ring, head, 16);
        assert!(lines.is_empty(), "history is asked for, not pushed");
        assert_eq!(missed, 0);
    }
}
