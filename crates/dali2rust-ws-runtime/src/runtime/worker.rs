use core::ops::ControlFlow;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime::coalesce::BurstCoalescer;
use dali2rust_api::ws::{
    diagnostics_snapshot_frame, event_channels, log_batch_frame, log_batch_prefix,
    project_event_where, sniffer_batch_frame, stats_snapshot_frame, Channel,
    SnifferDecoderState,
};
use dali2rust_api::http::diagnostics_state::DiagnosticsHttpState;
use dali2rust_api::http::stats_state::StatsHttpState;
use dali2rust_bus::{BusFrame, BusSubscriberRx, EventInboxProbe};
use dali2rust_bsp::log_ring::LogRing;
use dali2rust_platform::dali::{SnifferRecord, SnifferTap};
use dali2rust_platform::logs::{LogLevel, LogLine};

use crate::counters::WsCounters;
use crate::runtime::hub::WsHub;

const IDLE_RECV_TIMEOUT: Duration = Duration::from_millis(250);

const MIN_RECV_TIMEOUT: Duration = Duration::from_millis(10);

const NO_CLIENT_PARK: Duration = Duration::from_secs(5);

const SNIFFER_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

const SNIFFER_BATCH_MAX: usize = 64;

const LOGS_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

const LOGS_BATCH_MAX: usize = 16;

const STATS_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);
const DIAGNOSTICS_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(2);

pub struct WsWorkerPorts {
    pub stats: Arc<dyn StatsHttpState>,
    pub diagnostics: Arc<dyn DiagnosticsHttpState>,
}

struct Timers {
    sniffer_flush: Instant,
    logs_flush: Instant,
    stats: Instant,
    diagnostics: Instant,
}

impl Timers {
    fn new(now: Instant) -> Self {
        Self {
            sniffer_flush: now,
            logs_flush: now,
            stats: now,
            diagnostics: now,
        }
    }

    fn due(slot: &mut Instant, now: Instant, every: Duration) -> bool {
        if now.duration_since(*slot) < every {
            return false;
        }
        *slot = now;
        true
    }

    fn next_wait(&self, now: Instant) -> Duration {
        let earliest = [
            (self.sniffer_flush, SNIFFER_FLUSH_INTERVAL),
            (self.logs_flush, LOGS_FLUSH_INTERVAL),
            (self.stats, STATS_SNAPSHOT_INTERVAL),
            (self.diagnostics, DIAGNOSTICS_SNAPSHOT_INTERVAL),
        ]
        .into_iter()
        .map(|(slot, every)| every.saturating_sub(now.duration_since(slot)))
        .min()
        .unwrap_or(IDLE_RECV_TIMEOUT);
        earliest.min(IDLE_RECV_TIMEOUT).max(MIN_RECV_TIMEOUT)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one param per wired dependency; a struct here would only move the list"
)]
pub fn spawn_ws_worker(
    ev_rx: BusSubscriberRx,
    inbox: EventInboxProbe,
    sniffer_rx: Receiver<SnifferRecord>,
    tap: Arc<SnifferTap>,
    hub: Arc<WsHub>,
    ports: WsWorkerPorts,
    clock_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"ws_worker",
        dali2rust_bsp::std_thread_stack::EVENT_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || run(ev_rx, inbox, sniffer_rx, tap, hub, ports, clock_ms),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors spawn_ws_worker's wired dependency list"
)]
fn run(
    ev_rx: BusSubscriberRx,
    inbox: EventInboxProbe,
    sniffer_rx: Receiver<SnifferRecord>,
    tap: Arc<SnifferTap>,
    hub: Arc<WsHub>,
    ports: WsWorkerPorts,
    clock_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
) {
    let mut decoder = SnifferDecoderState::default();
    let mut burst = BurstCoalescer::new();
    let mut timers = Timers::new(Instant::now());
    let mut drains = Drains::new(&hub, &inbox);
    let backlog = Backlog {
        ev_rx: &ev_rx,
        sniffer_rx: &sniffer_rx,
        inbox: &inbox,
        tap: &tap,
    };
    loop {
        if !hub.has_clients() {
            if backlog.park_until_watched(&hub, &mut drains).is_break() {
                break;
            }
            continue;
        }
        match ev_rx.recv_timeout(timers.next_wait(Instant::now())) {
            Ok(frame) => drain_events(&hub, &ev_rx, frame, &mut burst),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if hub.has_clients() {
            report_inbox_overflow(&hub, &inbox, &mut drains.last_inbox_dropped);
            tick(
                &hub,
                &tap,
                &sniffer_rx,
                &mut decoder,
                &mut timers,
                &ports,
                clock_ms.as_ref(),
                &mut drains,
            );
        }
    }
}

struct Backlog<'a> {
    ev_rx: &'a BusSubscriberRx,
    sniffer_rx: &'a Receiver<SnifferRecord>,
    inbox: &'a EventInboxProbe,
    tap: &'a SnifferTap,
}

impl Backlog<'_> {
    fn park_until_watched(&self, hub: &WsHub, drains: &mut Drains) -> ControlFlow<()> {
        self.park_unwatched(hub, &mut drains.last_inbox_dropped, &mut drains.last_tap_dropped)?;
        drains.logs.rebase();
        ControlFlow::Continue(())
    }

    fn park_unwatched(
        &self,
        hub: &WsHub,
        last_inbox_dropped: &mut u32,
        last_tap_dropped: &mut u32,
    ) -> std::ops::ControlFlow<()> {
        while self.sniffer_rx.try_recv().is_ok() {}
        match self.ev_rx.try_recv() {
            Ok(_) => {
                while self.ev_rx.try_recv().is_ok() {}
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return std::ops::ControlFlow::Break(())
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        *last_inbox_dropped = self.inbox.dropped_total();
        *last_tap_dropped = self.tap.dropped_total();
        hub.wait_for_clients(NO_CLIENT_PARK);
        std::ops::ControlFlow::Continue(())
    }
}

fn report_inbox_overflow(hub: &WsHub, inbox: &EventInboxProbe, last: &mut u32) {
    let total = inbox.dropped_total();
    let delta = total.wrapping_sub(*last);
    if delta == 0 {
        return;
    }
    *last = total;
    hub.note_inbox_overflow(delta);
}

fn drain_events(hub: &WsHub, ev_rx: &BusSubscriberRx, first: BusFrame, burst: &mut BurstCoalescer) {
    push_event(burst, first);
    while let Ok(frame) = ev_rx.try_recv() {
        push_event(burst, frame);
    }
    WsCounters::bump(
        &hub.counters().events_coalesced_total,
        burst.take_superseded(),
    );
    for envelope in burst.drain() {
        fan_out_envelope(hub, &envelope);
    }
    burst.release_burst();
}

fn push_event(burst: &mut BurstCoalescer, frame: BusFrame) {
    if let BusFrame::Event(envelope) = frame {
        crate::runtime::coalesce::push_envelope(burst, envelope);
    }
}

fn fan_out_envelope(hub: &WsHub, envelope: &dali2rust_contracts::msg::EventEnvelope) {
    if !hub.has_clients() {
        return;
    }
    if !event_channels(envelope)
        .iter()
        .any(|channel| hub.any_subscriber(*channel))
    {
        return;
    }
    for (channel, text) in project_event_where(envelope, &|channel| hub.any_subscriber(channel)) {
        hub.broadcast(channel, &text);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one periodic sweep over independent timers; splitting the argument list would only move the same state behind a struct that exists for the argument count"
)]
fn tick(
    hub: &WsHub,
    tap: &SnifferTap,
    sniffer_rx: &Receiver<SnifferRecord>,
    decoder: &mut SnifferDecoderState,
    timers: &mut Timers,
    ports: &WsWorkerPorts,
    clock_ms: &(dyn Fn() -> u64 + Send + Sync),
    drains: &mut Drains,
) {
    let now = Instant::now();
    hub.reap_closed();
    if Timers::due(&mut timers.sniffer_flush, now, SNIFFER_FLUSH_INTERVAL) {
        flush_sniffer(hub, tap, sniffer_rx, decoder, clock_ms, &mut drains.last_tap_dropped);
    }
    if Timers::due(&mut timers.logs_flush, now, LOGS_FLUSH_INTERVAL) {
        flush_logs(hub, &mut drains.logs, clock_ms);
    }
    if Timers::due(&mut timers.stats, now, STATS_SNAPSHOT_INTERVAL)
        && hub.any_subscriber(Channel::Stats)
    {
        let frame = stats_snapshot_frame(ports.stats.as_ref(), clock_ms());
        hub.broadcast(Channel::Stats, &frame);
    }
    if Timers::due(&mut timers.diagnostics, now, DIAGNOSTICS_SNAPSHOT_INTERVAL)
        && hub.any_subscriber(Channel::Diagnostics)
    {
        let frame = diagnostics_snapshot_frame(ports.diagnostics.as_ref(), clock_ms());
        hub.broadcast(Channel::Diagnostics, &frame);
    }
}

struct LogDrain {
    ring: &'static LogRing,
    cursor: u32,
    buffer: Vec<LogLine>,
    last_ring_dropped: u32,
}

impl LogDrain {
    fn new(ring: &'static LogRing) -> Self {
        Self {
            cursor: ring.head(),
            ring,
            buffer: Vec::with_capacity(LOGS_BATCH_MAX),
            last_ring_dropped: 0,
        }
    }

    fn rebase(&mut self) {
        self.cursor = self.ring.head();
        self.last_ring_dropped = self.ring.dropped_total();
    }
}

struct Drains {
    last_inbox_dropped: u32,
    last_tap_dropped: u32,
    logs: LogDrain,
}

impl Drains {
    fn new(hub: &WsHub, inbox: &EventInboxProbe) -> Self {
        Self {
            last_inbox_dropped: inbox.dropped_total(),
            last_tap_dropped: 0,
            logs: LogDrain::new(hub.log_ring()),
        }
    }
}

fn flush_logs(hub: &WsHub, drain: &mut LogDrain, clock_ms: &(dyn Fn() -> u64 + Send + Sync)) {
    let counters = hub.counters();
    let ring_dropped = drain.ring.dropped_total();
    let refused = ring_dropped.wrapping_sub(drain.last_ring_dropped);
    drain.last_ring_dropped = ring_dropped;
    if let Some(rewind) = drain.ring.take_replay() {
        drain.cursor = drain.cursor.min(rewind);
    }
    drain.buffer.clear();
    let outcome = drain
        .ring
        .drain_from(drain.cursor, LOGS_BATCH_MAX, &mut drain.buffer);
    let carried = log_batch_prefix(&drain.buffer).max(1).min(drain.buffer.len());
    drain.buffer.truncate(carried);
    drain.cursor = match drain.buffer.last() {
        Some(last) => last.seq.wrapping_add(1),
        None => outcome.next_cursor,
    };
    let dropped_since = refused.saturating_add(outcome.missed);
    WsCounters::bump(&counters.logs_dropped_total, dropped_since);
    if drain.buffer.is_empty() && dropped_since == 0 {
        return;
    }
    for level in hub.log_level_cohorts() {
        send_cohort(hub, drain, level, dropped_since, clock_ms());
    }
}

fn send_cohort(
    hub: &WsHub,
    drain: &LogDrain,
    level: LogLevel,
    dropped_since: u32,
    ts_ms: u64,
) {
    let visible: Vec<LogLine> = drain
        .buffer
        .iter()
        .copied()
        .filter(|line| line.level <= level)
        .collect();
    if visible.is_empty() && dropped_since == 0 {
        return;
    }
    let counters = hub.counters();
    let lines = visible.len() as u32;
    WsCounters::bump(&counters.logs_lines_total, lines);
    let (frame, shed) = log_batch_frame(&visible, dropped_since, ts_ms);
    WsCounters::bump(&counters.logs_dropped_total, shed);
    hub.broadcast_to_log_level(level, &frame, lines.saturating_add(dropped_since));
}

fn flush_sniffer(
    hub: &WsHub,
    tap: &SnifferTap,
    sniffer_rx: &Receiver<SnifferRecord>,
    decoder: &mut SnifferDecoderState,
    clock_ms: &(dyn Fn() -> u64 + Send + Sync),
    last_tap_dropped: &mut u32,
) {
    let (mut batch, over_cap) = drain_newest(sniffer_rx);
    let tap_dropped = tap.dropped_total();
    let shed_by_tap = tap_dropped.wrapping_sub(*last_tap_dropped);
    *last_tap_dropped = tap_dropped;
    let dropped_since = shed_by_tap.saturating_add(over_cap);
    let counters = hub.counters();
    WsCounters::bump(&counters.sniffer_dropped_total, dropped_since);
    if batch.is_empty() && dropped_since == 0 {
        return;
    }
    if !hub.any_subscriber(Channel::Sniffer) {
        return;
    }
    let records = batch.len() as u32;
    WsCounters::bump(&counters.sniffer_records_total, records);
    let (frame, shed_by_budget) =
        sniffer_batch_frame(decoder, batch.make_contiguous(), dropped_since, clock_ms());
    WsCounters::bump(&counters.sniffer_dropped_total, shed_by_budget);
    hub.broadcast_counting(
        Channel::Sniffer,
        &frame,
        records.saturating_add(dropped_since),
    );
}

fn drain_newest(sniffer_rx: &Receiver<SnifferRecord>) -> (VecDeque<SnifferRecord>, u32) {
    let mut batch: VecDeque<SnifferRecord> = VecDeque::with_capacity(SNIFFER_BATCH_MAX);
    let mut over_cap = 0u32;
    while let Ok(record) = sniffer_rx.try_recv() {
        if batch.len() == SNIFFER_BATCH_MAX {
            batch.pop_front();
            over_cap = over_cap.saturating_add(1);
        }
        batch.push_back(record);
    }
    (batch, over_cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::hub::WsHubConfig;
    use crate::runtime::sink::{WsSink, WsSinkError};
    use dali2rust_platform::dali::{FrameDirection, ObservedRawFrameKind};
    use dali2rust_test_support::wait_until;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Condvar, Mutex};

    #[test]
    fn every_timer_slot_is_reachable_through_the_primary_wait() {
        let now = Instant::now();
        let mut timers = Timers::new(now);
        timers.sniffer_flush = now + Duration::from_secs(60);
        timers.stats = now + Duration::from_secs(60);
        timers.diagnostics = now + Duration::from_secs(60);
        assert!(
            timers.next_wait(now) <= LOGS_FLUSH_INTERVAL,
            "the logs timer is not in `next_wait`"
        );
    }

    #[test]
    fn a_log_flush_with_no_subscriber_sends_nothing() {
        let ring: &'static LogRing = crate::runtime::hub::leaked_test_ring();
        ring.set_min_level(dali2rust_platform::logs::LogLevel::Info);
        for index in 0..4u32 {
            ring.record(dali2rust_platform::logs::LogLevel::Info, b"t", b"line", index.into());
        }
        let (tap, _rx) = SnifferTap::new(4);
        let counters = Arc::new(WsCounters::default());
        let hub = WsHub::new(WsHubConfig::default(), Arc::clone(&counters), tap, ring);
        let mut drain = LogDrain::new(ring);
        drain.cursor = 0;

        flush_logs(&hub, &mut drain, &|| 0);

        assert_eq!(WsCounters::load(&counters.logs_lines_total), 0);
        assert_eq!(WsCounters::load(&counters.events_sent_total), 0);
    }

    #[test]
    fn a_ring_that_wrapped_before_the_drain_reports_the_gap_and_counts_it() {
        use dali2rust_platform::logs::LogLevel;

        let ring: &'static LogRing = crate::runtime::hub::leaked_test_ring();
        ring.set_min_level(LogLevel::Info);
        let (tap, _rx) = SnifferTap::new(4);
        let counters = Arc::new(WsCounters::default());
        let hub = WsHub::new(WsHubConfig::default(), Arc::clone(&counters), tap, ring);
        let mut drain = LogDrain::new(ring);

        for index in 0..(dali2rust_bsp::log_ring::LOG_RING_LINES + 40) {
            ring.record(LogLevel::Info, b"t", b"line", index as u64);
        }
        flush_logs(&hub, &mut drain, &|| 0);

        assert!(
            WsCounters::load(&counters.logs_dropped_total) >= 40,
            "the wrap must be counted, not silently skipped"
        );
    }

    #[test]
    fn a_quiet_subscriber_is_not_handed_the_loud_lines_the_ring_kept() {
        use dali2rust_platform::logs::LogLevel;

        let ring: &'static LogRing = crate::runtime::hub::leaked_test_ring();
        ring.set_min_level(LogLevel::Info);
        ring.record(LogLevel::Warn, b"t", b"noisy", 1);
        ring.record(LogLevel::Error, b"t", b"real", 2);

        let mut drain = LogDrain::new(ring);
        drain.cursor = 0;
        drain.buffer.clear();
        let outcome = ring.drain_from(0, LOGS_BATCH_MAX, &mut drain.buffer);
        assert_eq!(outcome.missed, 0);

        let visible: Vec<_> = drain
            .buffer
            .iter()
            .filter(|line| line.level <= LogLevel::Error)
            .map(|line| line.text_str().to_owned())
            .collect();
        assert_eq!(
            visible,
            vec!["real".to_owned()],
            "an `error` subscriber must not receive the ring's `warn` history"
        );
    }

    #[test]
    fn a_replay_is_streamed_one_batch_per_tick() {
        let ring: &'static LogRing = crate::runtime::hub::leaked_test_ring();
        ring.set_min_level(dali2rust_platform::logs::LogLevel::Info);
        let total = LOGS_BATCH_MAX * 3;
        for index in 0..total {
            ring.record(dali2rust_platform::logs::LogLevel::Info, b"t", b"x", index as u64);
        }
        let mut drain = LogDrain::new(ring);
        ring.request_replay(total as u32);

        let mut seen = 0usize;
        for _ in 0..3 {
            let mut buffer = Vec::new();
            if let Some(rewind) = ring.take_replay() {
                drain.cursor = drain.cursor.min(rewind);
            }
            let outcome = ring.drain_from(drain.cursor, LOGS_BATCH_MAX, &mut buffer);
            drain.cursor = outcome.next_cursor;
            assert!(buffer.len() <= LOGS_BATCH_MAX, "a tick may not exceed the cap");
            seen += buffer.len();
        }
        assert_eq!(seen, total, "three ticks cover the ring the replay asked for");
    }

    const BURST: u64 = SNIFFER_BATCH_MAX as u64 * 2;

    fn record(at_ms: u64) -> SnifferRecord {
        SnifferRecord {
            at_ms,
            direction: FrameDirection::Tx,
            kind: ObservedRawFrameKind::Forward16,
            bytes: [0xFE, 0x00, 0x00],
            adapter_id: 0,
            attempt: 0,
        }
    }

    #[test]
    fn an_over_cap_drain_keeps_the_newest_records_and_counts_the_rest() {
        let (tx, rx) = std::sync::mpsc::sync_channel(BURST as usize);
        for at_ms in 0..BURST {
            tx.send(record(at_ms)).expect("queue");
        }
        let (batch, over_cap) = drain_newest(&rx);
        assert_eq!(batch.len(), SNIFFER_BATCH_MAX);
        assert_eq!(over_cap as u64, BURST - SNIFFER_BATCH_MAX as u64);
        assert_eq!(
            batch.front().map(|r| r.at_ms),
            Some(BURST - SNIFFER_BATCH_MAX as u64),
            "the oldest survivor must be the first record inside the cap"
        );
        assert_eq!(
            batch.back().map(|r| r.at_ms),
            Some(BURST - 1),
            "the newest record on the wire must always be in the batch"
        );
    }

    #[test]
    fn a_drain_within_the_cap_keeps_everything_in_order() {
        let (tx, rx) = std::sync::mpsc::sync_channel(SNIFFER_BATCH_MAX);
        for at_ms in 0..3 {
            tx.send(record(at_ms)).expect("queue");
        }
        let (batch, over_cap) = drain_newest(&rx);
        assert_eq!(over_cap, 0);
        assert_eq!(
            batch.iter().map(|r| r.at_ms).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    struct PausedSink {
        frames: Arc<Mutex<Vec<String>>>,
        hold: Arc<(Mutex<bool>, Condvar)>,
        entered: Arc<AtomicU32>,
    }

    impl WsSink for PausedSink {
        fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let (lock, released) = &*self.hold;
            let mut held = lock.lock().unwrap();
            while *held {
                held = released.wait(held).unwrap();
            }
            drop(held);
            self.frames.lock().unwrap().push(text.to_string());
            Ok(())
        }
        fn close(&self) {}
    }

    fn find(frames: &Arc<Mutex<Vec<String>>>, needle: &str) -> Option<String> {
        frames
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.contains(needle))
            .cloned()
    }

    fn dropped_since_of(frame: &str) -> u32 {
        frame
            .split("\"dropped_since\":")
            .nth(1)
            .expect("a SnifferBatch frame carries dropped_since")
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .expect("digits follow the key")
    }

    #[test]
    fn byte_budget_shed_is_counted_not_just_reported_on_the_wire() {
        let (tap, _unused_tap_rx) = SnifferTap::new(8);
        let counters = Arc::new(WsCounters::default());
        let hub = WsHub::new(
            WsHubConfig::default(),
            Arc::clone(&counters),
            tap.clone(),
            crate::runtime::hub::leaked_test_ring(),
        );
        let frames = Arc::new(Mutex::new(Vec::new()));
        let id = hub
            .register(Box::new(PausedSink {
                frames: Arc::clone(&frames),
                hold: Arc::new((Mutex::new(false), Condvar::new())),
                entered: Arc::new(AtomicU32::new(0)),
            }))
            .expect("register");
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["sniffer"]}"#);

        let (tx, rx) = std::sync::mpsc::sync_channel(SNIFFER_BATCH_MAX);
        for at_ms in 0..SNIFFER_BATCH_MAX as u64 {
            tx.send(record(at_ms)).expect("queue");
        }
        let mut decoder = SnifferDecoderState::default();
        let mut last_tap_dropped = 0u32;
        flush_sniffer(&hub, &tap, &rx, &mut decoder, &|| 0, &mut last_tap_dropped);

        wait_until(
            || find(&frames, "SnifferBatch").is_some(),
            Duration::from_secs(2),
        );
        let batch = find(&frames, "SnifferBatch").expect("wait_until returned");
        let wire = dropped_since_of(&batch);
        assert!(wire > 0, "precondition: the byte budget must have shed");
        assert_eq!(
            WsCounters::load(&counters.sniffer_dropped_total),
            wire,
            "what the wire admits to losing, the counter must count"
        );
    }

    #[test]
    fn a_dropped_batch_frame_keeps_its_whole_record_debt() {
        const BATCH_RECORDS: u64 = 5;
        let (tap, _unused_tap_rx) = SnifferTap::new(8);
        let counters = Arc::new(WsCounters::default());
        let hub = WsHub::new(
            WsHubConfig {
                max_clients: 1,
                queue_depth: 1,
            },
            counters,
            tap.clone(),
            crate::runtime::hub::leaked_test_ring(),
        );
        let frames = Arc::new(Mutex::new(Vec::new()));
        let hold = Arc::new((Mutex::new(true), Condvar::new()));
        let entered = Arc::new(AtomicU32::new(0));
        let id = hub
            .register(Box::new(PausedSink {
                frames: Arc::clone(&frames),
                hold: Arc::clone(&hold),
                entered: Arc::clone(&entered),
            }))
            .expect("register");
        wait_until(
            || entered.load(Ordering::SeqCst) == 1,
            Duration::from_secs(2),
        );
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["sniffer"]}"#);

        let (tx, rx) = std::sync::mpsc::sync_channel(SNIFFER_BATCH_MAX);
        for at_ms in 0..BATCH_RECORDS {
            tx.send(record(at_ms)).expect("queue");
        }
        let mut decoder = SnifferDecoderState::default();
        let mut last_tap_dropped = 0u32;
        flush_sniffer(&hub, &tap, &rx, &mut decoder, &|| 0, &mut last_tap_dropped);

        *hold.0.lock().unwrap() = false;
        hold.1.notify_all();
        wait_until(
            || find(&frames, "\"op\":\"subscribed\"").is_some(),
            Duration::from_secs(2),
        );
        tx.send(record(99)).expect("queue");
        flush_sniffer(&hub, &tap, &rx, &mut decoder, &|| 0, &mut last_tap_dropped);
        wait_until(
            || find(&frames, "DropNotice").is_some(),
            Duration::from_secs(2),
        );
        let notice = find(&frames, "DropNotice").expect("wait_until returned");
        assert!(
            notice.contains(&format!("\"dropped_count\":{BATCH_RECORDS}")),
            "the debt is the batch's records, not the frame count: {notice}"
        );
    }

    #[test]
    fn the_primary_wait_never_overshoots_the_next_due_timer() {
        assert!(
            SNIFFER_FLUSH_INTERVAL < IDLE_RECV_TIMEOUT,
            "the ceiling is what used to be the whole wait"
        );
        let now = Instant::now();
        let timers = Timers::new(now);
        assert_eq!(timers.next_wait(now), SNIFFER_FLUSH_INTERVAL);
        let elapsed = Duration::from_millis(60);
        assert_eq!(
            timers.next_wait(now + elapsed),
            SNIFFER_FLUSH_INTERVAL - elapsed
        );
    }

    #[test]
    fn an_overdue_timer_still_leaves_the_busy_loop_floor() {
        let now = Instant::now();
        let timers = Timers::new(now);
        assert_eq!(
            timers.next_wait(now + STATS_SNAPSHOT_INTERVAL * 2),
            MIN_RECV_TIMEOUT
        );
    }
}
