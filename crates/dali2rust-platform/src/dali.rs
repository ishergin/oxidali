use core::sync::atomic::Ordering::Relaxed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOutcome {
    Answer(u8),
    NoAnswer,
    Collision,
    BusBusy,
    ForeignInWindow,
    // IEC 62386-101 §8.2.5
    CorruptedInWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOutcome {
    Completed(TransferOutcome),
    Aborted(TransferOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedRawFrameKind {
    Forward16,
    Forward24,
    Backward8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedRawFrame {
    pub bytes: [u8; 3],
    pub kind: ObservedRawFrameKind,
    pub observed_at_ms: u64,
    pub observed_at_mono_ms: u32,
}

pub type ObservedFrameSender = std::sync::mpsc::SyncSender<ObservedRawFrame>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDirection {
    Tx,
    ForeignRx,
    Reply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnifferRecord {
    pub at_ms: u64,
    pub direction: FrameDirection,
    pub kind: ObservedRawFrameKind,
    pub bytes: [u8; 3],
    pub adapter_id: u8,
    pub attempt: u8,
}

#[derive(Debug)]
pub struct SnifferTap {
    enabled: core::sync::atomic::AtomicBool,
    tx: std::sync::mpsc::SyncSender<SnifferRecord>,
    dropped: core::sync::atomic::AtomicU32,
}

impl SnifferTap {
    pub fn new(
        capacity: usize,
    ) -> (
        std::sync::Arc<Self>,
        std::sync::mpsc::Receiver<SnifferRecord>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(capacity);
        let tap = std::sync::Arc::new(Self {
            enabled: core::sync::atomic::AtomicBool::new(false),
            tx,
            dropped: core::sync::atomic::AtomicU32::new(0),
        });
        (tap, rx)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled
            .store(on, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_frame(
        &self,
        direction: FrameDirection,
        kind: ObservedRawFrameKind,
        bytes: [u8; 3],
        adapter_id: u8,
        attempt: u8,
    ) {
        if !self.is_enabled() {
            return;
        }
        self.record(SnifferRecord {
            at_ms: wall_clock_millis(),
            direction,
            kind,
            bytes,
            adapter_id,
            attempt,
        });
    }

    pub fn record(&self, record: SnifferRecord) {
        if !self.is_enabled() {
            return;
        }
        if self.tx.try_send(record).is_err() {
            self.dropped
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn dropped_total(&self) -> u32 {
        self.dropped.load(core::sync::atomic::Ordering::Relaxed)
    }
}

pub fn wall_clock_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct DaliWireCounters {
    pub transactions_started: core::sync::atomic::AtomicU32,
    pub transactions_completed: core::sync::atomic::AtomicU32,
    pub transactions_by_class: [core::sync::atomic::AtomicU32; 4],
    pub transaction_reopened: core::sync::atomic::AtomicU32,
    pub transaction_should_exceedances: core::sync::atomic::AtomicU32,
    pub transaction_budget_exceeded: core::sync::atomic::AtomicU32,
    pub transaction_leaks: core::sync::atomic::AtomicU32,
    pub session_poll_spins: core::sync::atomic::AtomicU32,

    pub frames_sent_by_priority: [core::sync::atomic::AtomicU32; 5],
    pub p1_window_late: core::sync::atomic::AtomicU32,
    pub bus_releases: core::sync::atomic::AtomicU32,

    pub bus_acquire_timeout: core::sync::atomic::AtomicU32,
    pub collisions: core::sync::atomic::AtomicU32,
    pub foreign_in_window: core::sync::atomic::AtomicU32,
    pub corrupted_in_window: core::sync::atomic::AtomicU32,

    pub exchange_retries: core::sync::atomic::AtomicU32,
    pub retry_exhausted: core::sync::atomic::AtomicU32,
    // IEC 62386-101 Table 17, Table 20
    pub send_twice_over_transmitter_max: core::sync::atomic::AtomicU32,
    pub send_twice_split: core::sync::atomic::AtomicU32,

    pub bus_power_down_active: core::sync::atomic::AtomicU32,
    pub bus_power_down_entries: core::sync::atomic::AtomicU32,
    pub system_failure_active: core::sync::atomic::AtomicU32,
    pub system_failure_entries: core::sync::atomic::AtomicU32,

    pub wire_ticks_total: core::sync::atomic::AtomicU32,
    pub wire_ticks_active: core::sync::atomic::AtomicU32,
    pub wire_ticks_tx: core::sync::atomic::AtomicU32,
    pub load_permille: core::sync::atomic::AtomicU32,
    pub load_own_permille: core::sync::atomic::AtomicU32,
}

pub const WIRE_LOAD_FULL_PERMILLE: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireLoad {
    pub permille: u16,
    pub own_permille: u16,
}

#[derive(Debug, Clone)]
pub struct WireLoadWindow {
    last_total: u32,
    last_active: u32,
    last_tx: u32,
    primed: bool,
    min_ticks: u32,
}

impl WireLoadWindow {
    pub fn new(min_ticks: u32) -> Self {
        Self {
            last_total: 0,
            last_active: 0,
            last_tx: 0,
            primed: false,
            min_ticks,
        }
    }

    pub fn sample(&mut self, total: u32, active: u32, tx: u32) -> Option<WireLoad> {
        if !self.primed {
            self.rebase(total, active, tx);
            self.primed = true;
            return None;
        }
        let d_total = total.wrapping_sub(self.last_total);
        if d_total < self.min_ticks.max(1) {
            return None;
        }
        let d_active = active.wrapping_sub(self.last_active);
        let d_tx = tx.wrapping_sub(self.last_tx);
        self.rebase(total, active, tx);
        let permille = permille_of(d_active, d_total);
        let own_permille = permille_of(d_tx, d_total).min(permille);
        Some(WireLoad {
            permille,
            own_permille,
        })
    }

    fn rebase(&mut self, total: u32, active: u32, tx: u32) {
        self.last_total = total;
        self.last_active = active;
        self.last_tx = tx;
    }
}

fn permille_of(part: u32, whole: u32) -> u16 {
    let scaled =
        u64::from(part) * u64::from(WIRE_LOAD_FULL_PERMILLE) / u64::from(whole.max(1));
    scaled.min(u64::from(WIRE_LOAD_FULL_PERMILLE)) as u16
}

#[cfg(test)]
mod wire_load_tests {
    use super::{WireLoad, WireLoadWindow};

    #[test]
    fn the_first_sample_only_sets_the_base() {
        let mut w = WireLoadWindow::new(100);
        assert_eq!(w.sample(5_000, 2_500, 1_000), None);
    }

    #[test]
    fn a_window_shorter_than_the_minimum_yields_nothing_and_keeps_its_base() {
        let mut w = WireLoadWindow::new(100);
        assert_eq!(w.sample(0, 0, 0), None);
        assert_eq!(w.sample(99, 50, 10), None);
        assert_eq!(
            w.sample(100, 50, 10),
            Some(WireLoad { permille: 500, own_permille: 100 })
        );
    }

    #[test]
    fn consecutive_windows_are_independent() {
        let mut w = WireLoadWindow::new(1);
        w.sample(0, 0, 0);
        assert_eq!(
            w.sample(1_000, 250, 100),
            Some(WireLoad { permille: 250, own_permille: 100 })
        );
        assert_eq!(
            w.sample(2_000, 250, 100),
            Some(WireLoad { permille: 0, own_permille: 0 })
        );
    }

    #[test]
    fn counters_wrapping_through_u32_max_read_as_a_plain_delta() {
        let mut w = WireLoadWindow::new(1);
        w.sample(u32::MAX - 9, u32::MAX - 4, u32::MAX - 1);
        assert_eq!(
            w.sample(10, 5, 1),
            Some(WireLoad { permille: 500, own_permille: 150 })
        );
    }

    #[test]
    fn a_torn_sample_clamps_instead_of_lying() {
        let mut w = WireLoadWindow::new(1);
        w.sample(0, 0, 0);
        assert_eq!(
            w.sample(10, 11, 12),
            Some(WireLoad { permille: 1000, own_permille: 1000 })
        );
    }

    #[test]
    fn own_never_exceeds_total_load() {
        let mut w = WireLoadWindow::new(1);
        w.sample(0, 0, 0);
        let reading = w.sample(100, 10, 20).expect("a full window");
        assert!(reading.own_permille <= reading.permille);
    }

    #[test]
    fn a_stalled_window_of_millions_of_ticks_does_not_overflow() {
        let mut w = WireLoadWindow::new(1);
        w.sample(0, 0, 0);
        assert_eq!(
            w.sample(4_000_000, 2_000_000, 0),
            Some(WireLoad { permille: 500, own_permille: 0 })
        );
    }
}

impl DaliWireCounters {
    pub const CLASS_COUNT: usize = 4;
    pub const PRIORITY_COUNT: usize = 5;

    pub fn note_frame_sent(&self, priority: u8) {
        if let Some(slot) = self
            .frames_sent_by_priority
            .get(usize::from(priority).wrapping_sub(1))
        {
            slot.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn note_transaction_started(&self, class: u8) {
        self.transactions_started
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if let Some(slot) = self
            .transactions_by_class
            .get(usize::from(class).wrapping_sub(2))
        {
            slot.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[must_use]
pub const fn sleep_parks_the_task(step_ms: u32, tick_ms: u32) -> bool {
    step_ms >= tick_ms
}

#[derive(Debug, Default)]
pub struct PhySnifferCounters {
    pub frames: core::sync::atomic::AtomicU32,
    pub backward8: core::sync::atomic::AtomicU32,
    pub forward16: core::sync::atomic::AtomicU32,
    pub forward24: core::sync::atomic::AtomicU32,
    pub decode_failed: core::sync::atomic::AtomicU32,
    pub unsupported_len: core::sync::atomic::AtomicU32,
    pub dropped: core::sync::atomic::AtomicU32,
    pub poll_fast: core::sync::atomic::AtomicU32,
    pub isr_ticks_lost: core::sync::atomic::AtomicU32,
    pub isr_ticks_extra: core::sync::atomic::AtomicU32,
    pub isr_late_ticks: core::sync::atomic::AtomicU32,
    pub isr_max_gap_us: core::sync::atomic::AtomicU32,

    pub backward_undecodable: core::sync::atomic::AtomicU32,
    pub backward_frame_size: core::sync::atomic::AtomicU32,
    pub backward_incomplete: core::sync::atomic::AtomicU32,
    pub backward_early_rejected: core::sync::atomic::AtomicU32,
    pub backward_late_rejected: core::sync::atomic::AtomicU32,
    pub backward_multi_answer: core::sync::atomic::AtomicU32,

    pub isr_ticks_deficit_raw: core::sync::atomic::AtomicU32,
    pub isr_ticks_surplus_raw: core::sync::atomic::AtomicU32,

    pub answer_staged: core::sync::atomic::AtomicU32,
    pub answer_stage_late: core::sync::atomic::AtomicU32,
    pub answer_stage_max_ticks: core::sync::atomic::AtomicU32,
    pub sniff_poll_late: core::sync::atomic::AtomicU32,
    pub sniff_poll_gap_max_us: core::sync::atomic::AtomicU32,
}

pub const SNIFFER_POLL_LATE_SLACK_US: u32 = 4_000;

pub fn sniffer_poll_is_late(gap_us: u32, poll_ms: u32) -> bool {
    gap_us > poll_ms.saturating_mul(1_000).saturating_add(SNIFFER_POLL_LATE_SLACK_US)
}

pub static PHY_FRAME_ACTIVE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

pub static PERSIST_COMMITS_TOTAL: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_COMMITS_DURING_FRAME: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

pub fn note_persist_commit() {
    use core::sync::atomic::Ordering::Relaxed;
    PERSIST_COMMITS_TOTAL.fetch_add(1, Relaxed);
    if PHY_FRAME_ACTIVE.load(Relaxed) {
        PERSIST_COMMITS_DURING_FRAME.fetch_add(1, Relaxed);
    }
}

pub fn persist_commit_overlap() -> (u32, u32) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        PERSIST_COMMITS_TOTAL.load(Relaxed),
        PERSIST_COMMITS_DURING_FRAME.load(Relaxed),
    )
}

pub static PHY_IDLE_TICKS: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

pub const PERSIST_QUIET_GAP_TICKS: u32 = 101;

pub const PERSIST_GATE_BUDGET_MS: u32 = 50;

pub static PERSIST_GATE_WAITS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_GATE_WAIT_MS: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_GATE_TIMEOUTS: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

pub fn wire_busy_for_persist() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    PHY_FRAME_ACTIVE.load(Relaxed) || PHY_IDLE_TICKS.load(Relaxed) < PERSIST_QUIET_GAP_TICKS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistGate {
    Open,
    Waited { ms: u32 },
    TimedOut { ms: u32 },
}

pub fn await_wire_gap(
    mut busy: impl FnMut() -> bool,
    mut sleep_ms: impl FnMut(u32),
    step_ms: u32,
    budget_ms: u32,
) -> PersistGate {
    if !busy() {
        return PersistGate::Open;
    }
    let mut waited = 0u32;
    while waited < budget_ms {
        sleep_ms(step_ms);
        waited = waited.saturating_add(step_ms);
        if !busy() {
            return PersistGate::Waited { ms: waited };
        }
    }
    PersistGate::TimedOut { ms: waited }
}

pub fn note_persist_gate(outcome: PersistGate) {
    use core::sync::atomic::Ordering::Relaxed;
    match outcome {
        PersistGate::Open => {}
        PersistGate::Waited { ms } => {
            PERSIST_GATE_WAITS.fetch_add(1, Relaxed);
            PERSIST_GATE_WAIT_MS.fetch_add(ms, Relaxed);
        }
        PersistGate::TimedOut { ms } => {
            PERSIST_GATE_WAITS.fetch_add(1, Relaxed);
            PERSIST_GATE_WAIT_MS.fetch_add(ms, Relaxed);
            PERSIST_GATE_TIMEOUTS.fetch_add(1, Relaxed);
        }
    }
}

pub fn persist_gate_stats() -> (u32, u32, u32) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        PERSIST_GATE_WAITS.load(Relaxed),
        PERSIST_GATE_WAIT_MS.load(Relaxed),
        PERSIST_GATE_TIMEOUTS.load(Relaxed),
    )
}

pub const PERSIST_FLUSH_SLOW_MS: u32 = 250;

pub static PERSIST_FLUSH_TOTAL: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_FLUSH_SLOW_TOTAL: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_FLUSH_MS_TOTAL: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub static PERSIST_FLUSH_MAX_MS: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

pub fn note_persist_flush(ms: u32) -> bool {
    PERSIST_FLUSH_TOTAL.fetch_add(1, Relaxed);
    PERSIST_FLUSH_MS_TOTAL.fetch_add(ms, Relaxed);
    PERSIST_FLUSH_MAX_MS.fetch_max(ms, Relaxed);
    let slow = ms > PERSIST_FLUSH_SLOW_MS;
    if slow {
        PERSIST_FLUSH_SLOW_TOTAL.fetch_add(1, Relaxed);
    }
    slow
}

pub fn persist_flush_stats() -> (u32, u32, u32, u32) {
    (
        PERSIST_FLUSH_TOTAL.load(Relaxed),
        PERSIST_FLUSH_SLOW_TOTAL.load(Relaxed),
        PERSIST_FLUSH_MS_TOTAL.load(Relaxed),
        PERSIST_FLUSH_MAX_MS.load(Relaxed),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame24Error<E> {
    Unsupported,
    Transport(E),
}

pub trait DaliTransport {
    type Error: core::fmt::Debug;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error>;

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error>;

    fn is_bus_idle(&self) -> Result<bool, Self::Error>;

    fn exchange_frame(
        &mut self,
        frame: u16,
        expects_backward: bool,
    ) -> Result<TransferOutcome, Self::Error> {
        self.send_forward_frame(frame)?;
        if !expects_backward {
            return Ok(TransferOutcome::NoAnswer);
        }
        Ok(match self.receive_backward_frame()? {
            Some(backward) => TransferOutcome::Answer(backward),
            None => TransferOutcome::NoAnswer,
        })
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let _ = min_idle_us;
        self.exchange_frame(frame, expects_backward)
    }

    fn exchange_transaction(
        &mut self,
        frames: &[(u16, bool, u32)],
    ) -> Result<BatchOutcome, Self::Error> {
        let mut last = TransferOutcome::NoAnswer;
        for &(frame, expects_backward, min_idle_us) in frames {
            last = self.exchange_frame_with_settle(frame, expects_backward, min_idle_us)?;
        }
        Ok(BatchOutcome::Completed(last))
    }

    fn exchange_frame24(
        &mut self,
        _frame: [u8; 3],
        _expects_backward: bool,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        Err(Frame24Error::Unsupported)
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        let _ = min_idle_us;
        self.exchange_frame24(frame, expects_backward)
    }

    fn supports_frame24(&self) -> bool {
        false
    }

    fn supports_tx_batching(&self) -> bool {
        false
    }

    fn tx_batch_capacity(&self) -> usize {
        1
    }

    fn foreign_activity(&self) -> u32 {
        0
    }

    fn honours_settle(&self) -> bool {
        false
    }

    fn last_tx_settle_ticks(&self) -> Option<u16> {
        None
    }

    fn set_observed_frame_sender(&mut self, _sender: ObservedFrameSender) {}

    fn set_sniffer_counters(&mut self, _sink: std::sync::Arc<PhySnifferCounters>) {}

    fn set_wire_counters(&mut self, _sink: std::sync::Arc<DaliWireCounters>) {}

    fn set_arbitration_reflex(
        &mut self,
        _reflex: std::sync::Arc<crate::arbitration::ArbitrationReflex>,
    ) {
    }
}

#[derive(Debug)]
pub struct WireActivity {
    start: std::time::Instant,
    last_ms: core::sync::atomic::AtomicU32,
    arrivals: [core::sync::atomic::AtomicU32; YIELDABLE_PRIORITIES],
}

const YIELDABLE_PRIORITIES: usize = 2;

impl Default for WireActivity {
    fn default() -> Self {
        Self::new()
    }
}

impl WireActivity {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            last_ms: core::sync::atomic::AtomicU32::new(0),
            arrivals: [
                core::sync::atomic::AtomicU32::new(0),
                core::sync::atomic::AtomicU32::new(0),
            ],
        }
    }

    fn now_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    pub fn note_activity(&self) {
        self.last_ms.store(
            self.now_ms().saturating_add(1),
            core::sync::atomic::Ordering::Relaxed,
        );
    }

    pub fn quiet_for(&self, quiet: core::time::Duration) -> bool {
        let last = self.last_ms.load(core::sync::atomic::Ordering::Relaxed);
        if last == 0 {
            return true;
        }
        u64::from(self.now_ms().wrapping_sub(last.saturating_sub(1)))
            >= quiet.as_millis() as u64
    }

    pub fn note_arrival(&self, priority: WirePriority) {
        let Some(slot) = self.arrivals.get(priority as usize) else {
            return;
        };
        self.note_activity();
        slot.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    fn arrival_tickets(&self) -> [u32; YIELDABLE_PRIORITIES] {
        core::array::from_fn(|index| {
            self.arrivals[index].load(core::sync::atomic::Ordering::Relaxed)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum WirePriority {
    Interactive = 0,
    Attended = 1,
    Unattended = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YieldGranularity {
    Frame,
    Step,
    Never,
}

#[derive(Debug, Clone)]
pub struct WireLease {
    gate: std::sync::Arc<WireActivity>,
    granularity: YieldGranularity,
    tickets: [u32; YIELDABLE_PRIORITIES],
    watched: usize,
}

impl WireLease {
    pub fn new(
        gate: std::sync::Arc<WireActivity>,
        priority: WirePriority,
        granularity: YieldGranularity,
    ) -> Self {
        let tickets = gate.arrival_tickets();
        Self {
            gate,
            granularity,
            tickets,
            watched: (priority as usize).min(YIELDABLE_PRIORITIES),
        }
    }

    pub fn granularity(&self) -> YieldGranularity {
        self.granularity
    }

    pub fn should_yield(&self) -> bool {
        let now = self.gate.arrival_tickets();
        now[..self.watched] != self.tickets[..self.watched]
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::sleep_parks_the_task;

    #[test]
    fn a_sleep_parks_only_at_or_above_one_tick() {
        const TICK_100HZ: u32 = 10;
        const TICK_1KHZ: u32 = 1;
        for burning in [1, 2, 3, 5, 6, 9] {
            assert!(
                !sleep_parks_the_task(burning, TICK_100HZ),
                "{burning} ms is below a 10 ms tick and busy-waits"
            );
        }
        for parking in [10, 11, 16, 22, 25, 100] {
            assert!(
                sleep_parks_the_task(parking, TICK_100HZ),
                "{parking} ms is at least one 10 ms tick and parks"
            );
        }
        assert!(!sleep_parks_the_task(0, TICK_1KHZ));
        for parking in [1, 2, 10] {
            assert!(sleep_parks_the_task(parking, TICK_1KHZ));
        }
    }
}

#[cfg(test)]
mod wire_activity_tests {
    use super::{WireActivity, WireLease, WirePriority, YieldGranularity};
    use core::time::Duration;
    use std::sync::Arc;

    fn lease(gate: &Arc<WireActivity>, priority: WirePriority) -> WireLease {
        WireLease::new(Arc::clone(gate), priority, YieldGranularity::Frame)
    }

    #[test]
    fn a_fresh_signal_is_quiet_so_the_first_poll_cycle_is_not_delayed() {
        assert!(WireActivity::new().quiet_for(Duration::from_millis(500)));
    }

    #[test]
    fn an_interactive_command_makes_it_busy() {
        let a = WireActivity::new();
        a.note_activity();
        assert!(!a.quiet_for(Duration::from_secs(5)));
    }

    #[test]
    fn a_zero_window_is_always_quiet_which_is_how_the_gate_is_disabled() {
        let a = WireActivity::new();
        a.note_activity();
        assert!(a.quiet_for(Duration::ZERO));
    }

    #[test]
    fn a_lease_taken_on_a_calm_bus_does_not_ask_to_yield() {
        let gate = Arc::new(WireActivity::new());
        assert!(!lease(&gate, WirePriority::Attended).should_yield());
        assert!(!lease(&gate, WirePriority::Unattended).should_yield());
    }

    #[test]
    fn an_interactive_arrival_revokes_every_lease_below_it() {
        let gate = Arc::new(WireActivity::new());
        let attended = lease(&gate, WirePriority::Attended);
        let unattended = lease(&gate, WirePriority::Unattended);
        gate.note_arrival(WirePriority::Interactive);
        assert!(attended.should_yield());
        assert!(unattended.should_yield());
    }

    #[test]
    fn an_attended_arrival_revokes_the_poller_but_not_another_attended_run() {
        let gate = Arc::new(WireActivity::new());
        let attended = lease(&gate, WirePriority::Attended);
        let unattended = lease(&gate, WirePriority::Unattended);
        gate.note_arrival(WirePriority::Attended);
        assert!(!attended.should_yield(), "same priority must not preempt");
        assert!(
            unattended.should_yield(),
            "the poller yields to attended work"
        );
    }

    #[test]
    fn the_poller_revokes_nothing_and_does_not_close_its_own_quiet_window() {
        let gate = Arc::new(WireActivity::new());
        let unattended = lease(&gate, WirePriority::Unattended);
        gate.note_arrival(WirePriority::Unattended);
        assert!(!unattended.should_yield());
        assert!(gate.quiet_for(Duration::from_secs(5)));
    }

    #[test]
    fn an_arrival_before_the_lease_was_taken_does_not_revoke_it() {
        let gate = Arc::new(WireActivity::new());
        gate.note_arrival(WirePriority::Interactive);
        assert!(!lease(&gate, WirePriority::Unattended).should_yield());
    }

    #[test]
    fn arrivals_need_no_completion_so_the_signal_cannot_leak() {
        let gate = Arc::new(WireActivity::new());
        for _ in 0..5 {
            gate.note_arrival(WirePriority::Interactive);
        }
        assert!(!lease(&gate, WirePriority::Unattended).should_yield());
    }

    #[test]
    fn an_attended_arrival_also_closes_the_quiet_window() {
        let gate = WireActivity::new();
        gate.note_arrival(WirePriority::Attended);
        assert!(!gate.quiet_for(Duration::from_secs(5)));
    }
}

#[cfg(test)]
mod sniffer_poll_tests {
    use super::sniffer_poll_is_late;

    #[test]
    fn a_poll_is_late_only_past_its_own_pace_plus_the_slack() {
        assert!(!sniffer_poll_is_late(5_000, 1));
        assert!(sniffer_poll_is_late(5_001, 1));
        assert!(!sniffer_poll_is_late(14_000, 10), "the idle pace is not late at 10 ms");
        assert!(sniffer_poll_is_late(14_001, 10));
    }
}

#[cfg(test)]
mod persist_flush_tests {
    use super::{note_persist_flush, persist_flush_stats, PERSIST_FLUSH_SLOW_MS};

    #[test]
    fn a_flush_is_booked_and_only_a_slow_one_is_called_slow() {
        let (n0, slow0, ms0, _) = persist_flush_stats();
        assert!(!note_persist_flush(PERSIST_FLUSH_SLOW_MS));
        assert!(note_persist_flush(PERSIST_FLUSH_SLOW_MS + 1));
        let (n1, slow1, ms1, max1) = persist_flush_stats();
        assert!(n1.wrapping_sub(n0) >= 2);
        assert!(slow1.wrapping_sub(slow0) >= 1);
        assert!(ms1.wrapping_sub(ms0) >= 2 * PERSIST_FLUSH_SLOW_MS + 1);
        assert!(max1 > PERSIST_FLUSH_SLOW_MS);
    }
}

#[cfg(test)]
mod persist_gate_tests {
    use super::{await_wire_gap, PersistGate};

    #[test]
    fn a_quiet_wire_opens_the_gate_without_waiting() {
        let mut slept = 0u32;
        let outcome = await_wire_gap(|| false, |ms| slept += ms, 1, 50);
        assert_eq!(outcome, PersistGate::Open);
        assert_eq!(slept, 0);
    }

    #[test]
    fn a_busy_wire_is_waited_out_step_by_step() {
        let mut polls = 0u32;
        let mut slept = 0u32;
        let outcome = await_wire_gap(
            || {
                polls += 1;
                polls <= 7
            },
            |ms| slept += ms,
            1,
            50,
        );
        assert_eq!(outcome, PersistGate::Waited { ms: 7 });
        assert_eq!(slept, 7);
    }

    #[test]
    fn a_saturated_wire_times_the_gate_out_at_its_budget() {
        let mut slept = 0u32;
        let outcome = await_wire_gap(|| true, |ms| slept += ms, 1, 50);
        assert_eq!(outcome, PersistGate::TimedOut { ms: 50 });
        assert_eq!(slept, 50);
    }

    #[test]
    fn the_quiet_gap_outlasts_the_backward_window() {
        const TABLE_20_MAX_US: u32 = 10_500;
        const PHY_TICK_US: u32 = 104;
        assert!(
            super::PERSIST_QUIET_GAP_TICKS * PHY_TICK_US > TABLE_20_MAX_US,
            "a {} tick gap is {} us, inside Table 20's {TABLE_20_MAX_US} us",
            super::PERSIST_QUIET_GAP_TICKS,
            super::PERSIST_QUIET_GAP_TICKS * PHY_TICK_US
        );
    }
}
