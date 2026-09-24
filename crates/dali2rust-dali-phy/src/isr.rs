use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};

use crate::answer::AtomicAnswerCell;
use crate::backward_window::{
    answer_gate, AnswerGate, ANSWER_ARM_MAX_IDLE_TICKS, ANSWER_ARM_TARGET_IDLE_TICKS,
};
use crate::command::{AtomicCommandCell, ExchangeId};
use crate::cpu::{raw_core_id, ISR_CORE_ID};
use crate::frame_length::{frame_length_class, FrameLengthClass};
use crate::fsm::{BusState, DaliBitbangPhy, RxCompletedEvent, RxState, TxPollResult};
use crate::gpio::RegisterGpio;
use crate::halfbits::HalfBitBuffer;
use crate::ring::SpscRing;

pub const RX_RING_CAP: usize = 8;
pub const SESSION_RING_CAP: usize = 8;
pub const TX_RING_CAP: usize = 5;

pub enum SessionEvent {
    TxComplete {
        pre_idle_ticks: u16,
    },
    TxCollision,
    TxRejected,
    TxVoided,
    RxComplete(RxCompletedEvent),
    ForeignForward {
        pre_idle_ticks: u8,
        sample_count: u16,
    },
}

#[derive(Clone, Copy)]
pub struct TxRequest {
    exchange_id: ExchangeId,
    batched: bool,
    data: [u8; HalfBitBuffer::DATA_LEN],
    len: u8,
    expects_backward: bool,
    min_idle_ticks: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TryArmAnswer {
    NotStaged,
    Held,
    Armed,
}

struct ControlState {
    requested: AtomicU32,
    acknowledged: AtomicU32,
    active: AtomicBool,
    rx_max_gap_us: UnsafeCell<u32>,
}

impl ControlState {
    const fn new() -> Self {
        Self {
            requested: AtomicU32::new(0),
            acknowledged: AtomicU32::new(0),
            active: AtomicBool::new(false),
            rx_max_gap_us: UnsafeCell::new(0),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AnswerCounts {
    pub sent: u32,
    pub stale: u32,
    pub expired: u32,
    pub late: u32,
    pub rejected: u32,
    pub collided: u32,
}

pub struct PhyIsrCore {
    phy: UnsafeCell<DaliBitbangPhy<RegisterGpio>>,
    cmd: AtomicCommandCell,
    tx_queue: SpscRing<TxRequest, TX_RING_CAP>,
    pending: UnsafeCell<Option<TxRequest>>,
    batch_aborted: AtomicBool,
    control: ControlState,
    exchange_epoch: AtomicU32,
    active_epoch: AtomicU32,
    session_events: SpscRing<(ExchangeId, SessionEvent), SESSION_RING_CAP>,
    rx_ring: SpscRing<RxCompletedEvent, RX_RING_CAP>,
    sniff_dropped: AtomicU32,
    session_dropped: AtomicU32,
    stale_session_events: AtomicU32,
    idle_ticks: AtomicU32,
    active_expects_backward: AtomicBool,
    session_exchange: AtomicU32,
    session_rx_started: AtomicU32,
    bus_failure_flags: AtomicU32,
    bus_power_down_entries: AtomicU32,
    system_failure_entries: AtomicU32,
    line_held: AtomicU32,
    wire_ticks_total: AtomicU32,
    wire_ticks_active: AtomicU32,
    wire_ticks_tx: AtomicU32,
    answer: AtomicAnswerCell,
    bus_epoch: AtomicU8,
    active_is_answer: AtomicBool,
    last_answer_arm_idle_ticks: AtomicU32,
    answers_sent: AtomicU32,
    answers_stale: AtomicU32,
    answers_expired: AtomicU32,
    answers_late: AtomicU32,
    answers_rejected: AtomicU32,
    probe: TickProbe,
    tx_yields: AtomicU32,
    tx_voided: AtomicU32,
    answers_collided: AtomicU32,
}

pub const BUS_FAILURE_POWER_DOWN: u32 = 1;
pub const BUS_FAILURE_SYSTEM: u32 = 2;

pub const LATE_TICK_US: u32 = 2 * crate::fsm::PHY_TICK_US;

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
unsafe fn read_slot(addr: usize) -> usize {
    if addr == 0 {
        return 0;
    }
    // SAFETY: the caller names a word of process lifetime.
    unsafe { core::ptr::read_volatile(addr as *const usize) }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
unsafe fn read_counter(addr: usize) -> u32 {
    if addr == 0 {
        return 0;
    }
    // SAFETY: the caller names a word of process lifetime.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

const INTERRUPT_FRAME_RA_OFFSET: usize = 4;

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
unsafe fn interrupted_site(tcb: usize, depth: u32) -> (u32, u32) {
    if depth != 1 {
        return (0, 0);
    }
    // SAFETY: the TCB's first word is the interrupt frame the port saved on the current task's stack.
    let frame = unsafe { read_slot(tcb) };
    if frame == 0 {
        return (0, 0);
    }
    // SAFETY: the frame's first two words are `mepc` and `ra`.
    unsafe {
        (
            read_counter(frame),
            read_counter(frame.wrapping_add(INTERRUPT_FRAME_RA_OFFSET)),
        )
    }
}

struct TickProbe {
    clock: AtomicUsize,
    tcb: [AtomicUsize; 2],
    nesting: [AtomicUsize; 2],
    last_us: AtomicU32,
    late_ticks: AtomicU32,
    max_gap_us: AtomicU32,
    last_gap_us: AtomicU32,
    last_gap_at_us: AtomicU32,
    last_gap_tcb: AtomicUsize,
    last_gap_pc: AtomicU32,
    last_gap_ra: AtomicU32,
    last_gap_depth: AtomicU32,
}

impl TickProbe {
    const fn new() -> Self {
        Self {
            clock: AtomicUsize::new(0),
            tcb: [AtomicUsize::new(0), AtomicUsize::new(0)],
            nesting: [AtomicUsize::new(0), AtomicUsize::new(0)],
            last_us: AtomicU32::new(0),
            late_ticks: AtomicU32::new(0),
            max_gap_us: AtomicU32::new(0),
            last_gap_us: AtomicU32::new(0),
            last_gap_at_us: AtomicU32::new(0),
            last_gap_tcb: AtomicUsize::new(0),
            last_gap_pc: AtomicU32::new(0),
            last_gap_ra: AtomicU32::new(0),
            last_gap_depth: AtomicU32::new(0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LateTick {
    pub gap_us: u32,
    pub at_us: u32,
    pub interrupted_task: usize,
    pub interrupted_pc: u32,
    pub interrupted_ra: u32,
    pub nesting_depth: u32,
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
fn frame_active(bus_state: BusState, failure_flags: u32) -> bool {
    bus_state != BusState::Idle && failure_flags & BUS_FAILURE_POWER_DOWN == 0
}

// SAFETY: only the ISR touches the FSM; everything else is atomics and SPSC ends with one owner each.
unsafe impl Sync for PhyIsrCore {}

impl PhyIsrCore {
    pub fn new(gpio: RegisterGpio) -> Self {
        Self {
            phy: UnsafeCell::new(DaliBitbangPhy::new(gpio)),
            cmd: AtomicCommandCell::new(),
            tx_queue: SpscRing::new(),
            pending: UnsafeCell::new(None),
            batch_aborted: AtomicBool::new(false),
            control: ControlState::new(),
            exchange_epoch: AtomicU32::new(0),
            active_epoch: AtomicU32::new(0),
            session_events: SpscRing::new(),
            rx_ring: SpscRing::new(),
            sniff_dropped: AtomicU32::new(0),
            session_dropped: AtomicU32::new(0),
            stale_session_events: AtomicU32::new(0),
            idle_ticks: AtomicU32::new(0),
            active_expects_backward: AtomicBool::new(false),
            session_exchange: AtomicU32::new(0),
            session_rx_started: AtomicU32::new(0),
            bus_failure_flags: AtomicU32::new(0),
            bus_power_down_entries: AtomicU32::new(0),
            line_held: AtomicU32::new(0),
            system_failure_entries: AtomicU32::new(0),
            wire_ticks_total: AtomicU32::new(0),
            wire_ticks_active: AtomicU32::new(0),
            wire_ticks_tx: AtomicU32::new(0),
            answer: AtomicAnswerCell::new(),
            bus_epoch: AtomicU8::new(0),
            active_is_answer: AtomicBool::new(false),
            last_answer_arm_idle_ticks: AtomicU32::new(0),
            answers_sent: AtomicU32::new(0),
            answers_stale: AtomicU32::new(0),
            answers_expired: AtomicU32::new(0),
            answers_late: AtomicU32::new(0),
            answers_rejected: AtomicU32::new(0),
            probe: TickProbe::new(),
            tx_yields: AtomicU32::new(0),
            tx_voided: AtomicU32::new(0),
            answers_collided: AtomicU32::new(0),
        }
    }

    /// # Safety
    /// Only the GPTimer alarm callback may call this: it is the sole mutator of the PHY FSM.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub unsafe fn tick(&self) {
        let core = raw_core_id();
        ISR_CORE_ID.store(core, Ordering::Relaxed);
        // SAFETY: the caller guarantees this is the GPTimer ISR, the only context that touches the FSM.
        let phy = unsafe { &mut *self.phy.get() };
        let rx_before = phy.rx_state();
        // SAFETY: the probe's pointers were installed before the timer started.
        let sample_gap_us = unsafe { self.probe_tick(core) };

        // SAFETY: this is the interrupt, the sole owner of the pending slot.
        unsafe { self.arm_pending_tx(phy) };
        let was_tx_busy = phy.tx_busy.load(Ordering::Acquire);
        phy.tick();
        // SAFETY: this ISR exclusively owns the timing accumulator.
        unsafe { self.track_rx_timing(rx_before, phy.rx_state(), sample_gap_us) };
        self.publish_tick_state(phy);
        if was_tx_busy && !phy.tx_busy.load(Ordering::Acquire) {
            // SAFETY: this is the interrupt, the sole owner of `pending`.
            unsafe { self.report_tx_completion(phy) };
        }
        // SAFETY: same.
        unsafe { self.drain_completed_rx(phy) };
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn probe_tick(&self, core: u32) -> u32 {
        let clock = self.probe.clock.load(Ordering::Relaxed);
        if clock == 0 {
            return 0;
        }
        // SAFETY: installed as `unsafe extern "C" fn() -> i64` by `install_tick_probe`; only its address was stored.
        let clock: unsafe extern "C" fn() -> i64 = unsafe { core::mem::transmute(clock) };
        // SAFETY: an IRAM clock function installed before the timer started; it makes no kernel call.
        let now = unsafe { clock() } as u32;
        let last = self.probe.last_us.swap(now, Ordering::Relaxed);
        if last == 0 {
            return 0;
        }
        let gap = now.wrapping_sub(last);
        if gap > LATE_TICK_US {
            // SAFETY: same contract as this function.
            unsafe { self.book_late_tick(core, now, gap) };
        }
        gap
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn track_rx_timing(&self, before: RxState, after: RxState, gap_us: u32) {
        // SAFETY: the caller is `tick`, the sole owner of `rx_max_gap_us`.
        let slot = unsafe { &mut *self.control.rx_max_gap_us.get() };
        if before != RxState::Receiving && after == RxState::Receiving {
            *slot = 0;
        }
        if before == RxState::Receiving || after == RxState::Receiving {
            *slot = (*slot).max(gap_us);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn book_late_tick(&self, core: u32, now: u32, gap: u32) {
        self.probe.late_ticks.fetch_add(1, Ordering::Relaxed);
        if gap > self.probe.max_gap_us.load(Ordering::Relaxed) {
            self.probe.max_gap_us.store(gap, Ordering::Relaxed);
        }
        let (tcb_slot, depth_slot) = if core == 0 {
            (
                self.probe.tcb[0].load(Ordering::Relaxed),
                self.probe.nesting[0].load(Ordering::Relaxed),
            )
        } else {
            (
                self.probe.tcb[1].load(Ordering::Relaxed),
                self.probe.nesting[1].load(Ordering::Relaxed),
            )
        };
        // SAFETY: this core's current-task slot and nesting counter are process-lifetime DRAM words, read only.
        let tcb = unsafe { read_slot(tcb_slot) };
        // SAFETY: the port's nesting counter for this core, a DRAM word read only.
        let depth = unsafe { read_counter(depth_slot) };
        // SAFETY: `tcb` is the task running on this core at this instant.
        let (pc, ra) = unsafe { interrupted_site(tcb, depth) };
        self.probe.last_gap_us.store(gap, Ordering::Relaxed);
        self.probe.last_gap_at_us.store(now, Ordering::Relaxed);
        self.probe.last_gap_tcb.store(tcb, Ordering::Relaxed);
        self.probe.last_gap_pc.store(pc, Ordering::Relaxed);
        self.probe.last_gap_ra.store(ra, Ordering::Relaxed);
        self.probe.last_gap_depth.store(depth, Ordering::Relaxed);
    }

    pub fn install_tick_probe(
        &self,
        clock: unsafe extern "C" fn() -> i64,
        tcb_core0: *const usize,
        tcb_core1: *const usize,
        nesting_core0: *const u32,
        nesting_core1: *const u32,
    ) {
        self.probe.tcb[0].store(tcb_core0 as usize, Ordering::Relaxed);
        self.probe.tcb[1].store(tcb_core1 as usize, Ordering::Relaxed);
        self.probe.nesting[0].store(nesting_core0 as usize, Ordering::Relaxed);
        self.probe.nesting[1].store(nesting_core1 as usize, Ordering::Relaxed);
        self.probe.clock.store(clock as usize, Ordering::Release);
    }

    pub fn late_tick_counts(&self) -> (u32, u32) {
        (
            self.probe.late_ticks.load(Ordering::Relaxed),
            self.probe.max_gap_us.load(Ordering::Relaxed),
        )
    }

    pub fn take_late_tick(&self) -> Option<LateTick> {
        let gap_us = self.probe.last_gap_us.swap(0, Ordering::Relaxed);
        if gap_us == 0 {
            return None;
        }
        Some(LateTick {
            gap_us,
            at_us: self.probe.last_gap_at_us.load(Ordering::Relaxed),
            interrupted_task: self.probe.last_gap_tcb.load(Ordering::Relaxed),
            interrupted_pc: self.probe.last_gap_pc.load(Ordering::Relaxed),
            interrupted_ra: self.probe.last_gap_ra.load(Ordering::Relaxed),
            nesting_depth: self.probe.last_gap_depth.load(Ordering::Relaxed),
        })
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn arm_pending_tx(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) {
        // SAFETY: the caller is `tick`, which owns `pending` exclusively.
        unsafe { self.apply_requested_cancel(phy) };
        if phy.tx_busy.load(Ordering::Acquire) {
            return;
        }
        match self.try_arm_answer(phy) {
            TryArmAnswer::Armed | TryArmAnswer::Held => return,
            TryArmAnswer::NotStaged => {}
        }
        if self.session_exchange.load(Ordering::Acquire) != 0 {
            return;
        }
        if phy.bus_state() != BusState::Idle || phy.line_debouncing() {
            return;
        }
        // SAFETY: `tick` is the only caller and the only mutator of `pending`.
        unsafe { self.arm_pending_forward(phy) };
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn apply_requested_cancel(&self, phy: &DaliBitbangPhy<RegisterGpio>) {
        let raw = self.control.requested.load(Ordering::Acquire);
        if raw == 0 {
            return;
        }
        let active = phy.tx_busy.load(Ordering::Acquire)
            && !self.active_is_answer.load(Ordering::Acquire)
            && self.active_epoch.load(Ordering::Acquire) == raw;
        if active {
            self.control.active.store(true, Ordering::Release);
            while self.tx_queue.try_pop().is_some() {}
            self.cmd.take_command();
        } else {
            // SAFETY: called from `tick`, which owns `pending` and the queue consumer.
            unsafe { self.discard_queued_and_cell() };
            self.control.active.store(false, Ordering::Release);
        }
        if self.active_epoch.load(Ordering::Acquire) == raw {
            self.active_expects_backward.store(false, Ordering::Release);
        }
        if self.session_exchange.load(Ordering::Acquire) == raw {
            self.session_exchange.store(0, Ordering::Release);
            self.session_rx_started.store(0, Ordering::Release);
        }
        let _ = self
            .control
            .requested
            .compare_exchange(raw, 0, Ordering::AcqRel, Ordering::Relaxed);
        self.control.acknowledged.store(raw, Ordering::Release);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn arm_pending_forward(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) {
        // SAFETY: `tick` is the only caller and the only mutator of `pending`.
        let pending = unsafe { &mut *self.pending.get() };
        if pending.is_none() {
            *pending = self.next_request();
        }
        let Some(req) = *pending else {
            return;
        };
        if phy.idle_tick_count() < u16::from(req.min_idle_ticks) {
            return;
        }
        let hb = HalfBitBuffer {
            data: req.data,
            length: req.len,
        };
        self.active_epoch
            .store(req.exchange_id.0, Ordering::Release);
        self.active_expects_backward
            .store(req.expects_backward, Ordering::Release);
        if phy.start_tx(&hb) {
            self.bus_epoch.fetch_add(1, Ordering::Relaxed);
        } else {
            *pending = None;
            self.active_expects_backward.store(false, Ordering::Release);
            self.push_session_event(SessionEvent::TxRejected);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn clear_pending(&self) {
        // SAFETY: called from `tick`, the sole mutator of `pending`.
        unsafe { *self.pending.get() = None };
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn try_arm_answer(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) -> TryArmAnswer {
        let Some(epoch) = self.answer.staged_epoch() else {
            return TryArmAnswer::NotStaged;
        };
        if epoch != self.bus_epoch.load(Ordering::Relaxed) {
            self.answer.discard();
            self.answers_stale.fetch_add(1, Ordering::Relaxed);
            return TryArmAnswer::NotStaged;
        }
        if phy.bus_state() != BusState::Idle || phy.line_debouncing() {
            return TryArmAnswer::Held;
        }
        match answer_gate(
            phy.idle_tick_count(),
            ANSWER_ARM_TARGET_IDLE_TICKS,
            ANSWER_ARM_MAX_IDLE_TICKS,
        ) {
            AnswerGate::Hold => TryArmAnswer::Held,
            AnswerGate::Expired => {
                self.answer.discard();
                self.answers_expired.fetch_add(1, Ordering::Relaxed);
                TryArmAnswer::NotStaged
            }
            // SAFETY: caller guarantees the ISR context.
            AnswerGate::Arm { late } => unsafe { self.fire_staged_answer(phy, late) },
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn fire_staged_answer(
        &self,
        phy: &mut DaliBitbangPhy<RegisterGpio>,
        late: bool,
    ) -> TryArmAnswer {
        let Some((_epoch, hb)) = self.answer.take() else {
            return TryArmAnswer::NotStaged;
        };
        self.last_answer_arm_idle_ticks
            .store(u32::from(phy.idle_tick_count()), Ordering::Relaxed);
        if late {
            self.answers_late.fetch_add(1, Ordering::Relaxed);
        }
        self.active_is_answer.store(true, Ordering::Release);
        self.active_expects_backward.store(false, Ordering::Release);
        if !phy.start_tx(&hb) {
            self.active_is_answer.store(false, Ordering::Release);
            self.answers_rejected.fetch_add(1, Ordering::Relaxed);
        }
        TryArmAnswer::Armed
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn next_request(&self) -> Option<TxRequest> {
        if let Some(req) = self.tx_queue.try_pop() {
            return Some(req);
        }
        let cmd = self.cmd.take_command()?;
        Some(TxRequest {
            exchange_id: cmd.exchange_id,
            batched: false,
            data: cmd.data,
            len: cmd.len,
            expects_backward: cmd.expects_backward,
            min_idle_ticks: cmd.min_idle_ticks,
        })
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn abort_batch_tail(&self) {
        // SAFETY: called from `tick`, the sole mutator of `pending`.
        let held_request = unsafe { *self.pending.get() };
        // SAFETY: called from `tick`, the sole mutator of `pending`.
        let (held, queued) = unsafe { self.discard_queued() };
        if let (true, Some(request)) = (held, held_request) {
            self.tx_voided.fetch_add(1, Ordering::Relaxed);
            self.push_session_event_for(request.exchange_id, SessionEvent::TxVoided);
        }
        if queued || held_request.is_some_and(|request| request.batched) {
            self.batch_aborted.store(true, Ordering::Release);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn discard_queued(&self) -> (bool, bool) {
        // SAFETY: called from `tick`, the sole mutator of `pending`.
        let pending = unsafe { &mut *self.pending.get() };
        let held = pending.is_some();
        let queued = !self.tx_queue.is_empty();
        *pending = None;
        while self.tx_queue.try_pop().is_some() {}
        (held, queued)
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn discard_queued_and_cell(&self) {
        // SAFETY: called from `tick`, the sole mutator of `pending`.
        unsafe { self.discard_queued() };
        self.cmd.take_command();
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn publish_tick_state(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) {
        let idle = u32::from(phy.idle_tick_count());
        self.idle_ticks.store(idle, Ordering::Release);
        let low_run = phy.bus_low_run_ticks();
        let flags = u32::from(low_run >= crate::fsm::BUS_POWER_DOWN_TICKS)
            | (u32::from(low_run >= crate::fsm::SYSTEM_FAILURE_TICKS) << 1);
        let gap = if flags & BUS_FAILURE_POWER_DOWN != 0 { u32::MAX } else { idle };
        dali2rust_platform::dali::PHY_IDLE_TICKS.store(gap, Ordering::Relaxed);
        if low_run >= crate::fsm::LINE_HELD_TICKS {
            let packed = self.line_held.load(Ordering::Relaxed);
            let runs = packed >> 16;
            let longest = packed & 0xFFFF;
            let run = u32::from(low_run);
            let runs = if low_run == crate::fsm::LINE_HELD_TICKS {
                runs.saturating_add(1).min(0xFFFF)
            } else {
                runs
            };
            self.line_held
                .store((runs << 16) | longest.max(run), Ordering::Relaxed);
        }
        let prev = self.bus_failure_flags.swap(flags, Ordering::Relaxed);
        if flags & BUS_FAILURE_POWER_DOWN != 0 && prev & BUS_FAILURE_POWER_DOWN == 0 {
            self.bus_power_down_entries.fetch_add(1, Ordering::Relaxed);
        }
        if flags & BUS_FAILURE_SYSTEM != 0 && prev & BUS_FAILURE_SYSTEM == 0 {
            self.system_failure_entries.fetch_add(1, Ordering::Relaxed);
        }
        let bus_state = phy.bus_state();
        self.note_wire_ticks(bus_state);
        dali2rust_platform::dali::PHY_FRAME_ACTIVE
            .store(frame_active(bus_state, flags), Ordering::Relaxed);
        self.note_session_rx_started(phy);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn note_session_rx_started(&self, phy: &DaliBitbangPhy<RegisterGpio>) {
        if self.session_exchange.load(Ordering::Acquire) != 0
            && phy.rx_state() == RxState::Receiving
        {
            self.session_rx_started
                .store(1 + u32::from(phy.rx_pre_idle_ticks()), Ordering::Release);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn note_wire_ticks(&self, bus_state: BusState) {
        self.wire_ticks_total.fetch_add(1, Ordering::Relaxed);
        if bus_state != BusState::Idle {
            self.wire_ticks_active.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(bus_state, BusState::Tx | BusState::CollisionTx) {
            self.wire_ticks_tx.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn report_tx_completion(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) {
        let poll = phy.poll_tx_state();
        if self.active_is_answer.swap(false, Ordering::AcqRel) {
            self.report_answer_completion(poll);
            return;
        }
        // SAFETY: called from `tick`, the sole owner of `pending`.
        unsafe { self.report_forward_completion(phy, poll) };
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn report_answer_completion(&self, poll: TxPollResult) {
        if poll == TxPollResult::Collision {
            self.answers_collided.fetch_add(1, Ordering::Relaxed);
        } else if poll == TxPollResult::Yielded {
            self.answers_stale.fetch_add(1, Ordering::Relaxed);
        } else {
            self.answers_sent.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn report_forward_completion(
        &self,
        phy: &mut DaliBitbangPhy<RegisterGpio>,
        poll: TxPollResult,
    ) {
        if self.control.active.swap(false, Ordering::AcqRel) {
            // SAFETY: called from `tick`, the sole owner of `pending`.
            unsafe { self.clear_pending() };
            self.active_expects_backward.store(false, Ordering::Release);
            self.session_exchange.store(0, Ordering::Release);
            self.session_rx_started.store(0, Ordering::Release);
            return;
        }
        if poll == TxPollResult::Ok {
            // SAFETY: called from `tick`, the sole owner of `pending`.
            unsafe { self.clear_pending() };
            let expects = self.active_expects_backward.swap(false, Ordering::AcqRel);
            if expects {
                self.session_exchange
                    .store(self.active_epoch.load(Ordering::Acquire), Ordering::Release);
            }
            self.push_session_event(SessionEvent::TxComplete {
                pre_idle_ticks: phy.tx_pre_idle_ticks(),
            });
        } else if poll == TxPollResult::Collision {
            // SAFETY: called from `tick`, the sole owner of `pending`.
            unsafe { self.clear_pending() };
            self.active_expects_backward.store(false, Ordering::Release);
            self.session_exchange.store(0, Ordering::Release);
            // SAFETY: called from `tick`, the sole owner of `pending`.
            unsafe { self.abort_batch_tail() };
            self.push_session_event(SessionEvent::TxCollision);
        } else if poll == TxPollResult::Yielded {
            self.active_expects_backward.store(false, Ordering::Release);
            self.tx_yields.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn drain_completed_rx(&self, phy: &mut DaliBitbangPhy<RegisterGpio>) {
        let Some(mut ev) = phy.take_completed_rx_event() else {
            return;
        };
        let epoch = self
            .bus_epoch
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        ev.rx_epoch = epoch;
        ev.rx_tick = self.wire_ticks_total.load(Ordering::Relaxed);
        // SAFETY: `tick` is the sole owner, and the completed event closes this accumulator before the next capture.
        unsafe { self.stamp_rx_timing(&mut ev) };
        match frame_length_class(ev.sample_count) {
            // SAFETY: called from `tick`, which owns `pending` exclusively.
            FrameLengthClass::Backward => unsafe { self.route_backward_capture(ev) },
            // SAFETY: same.
            FrameLengthClass::Forward => unsafe { self.route_forward_capture(ev) },
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn stamp_rx_timing(&self, ev: &mut RxCompletedEvent) {
        // SAFETY: the caller is `tick`, the sole owner of `rx_max_gap_us`.
        ev.max_sample_gap_us = unsafe { *self.control.rx_max_gap_us.get() };
        ev.timing_degraded = ev.max_sample_gap_us >= LATE_TICK_US;
        // SAFETY: as above.
        unsafe { *self.control.rx_max_gap_us.get() = 0 };
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn route_backward_capture(&self, ev: RxCompletedEvent) {
        let exchange = self.session_exchange.swap(0, Ordering::AcqRel);
        self.session_rx_started.store(0, Ordering::Release);
        if exchange != 0 {
            self.push_session_event_for(ExchangeId(exchange), SessionEvent::RxComplete(ev));
            return;
        }
        self.push_sniffer_capture(ev);
    }

    // IEC 62386-102 §11.7.14
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    unsafe fn route_forward_capture(&self, ev: RxCompletedEvent) {
        // SAFETY: called from `tick`, which owns `pending` exclusively.
        unsafe { self.abort_batch_tail() };
        let exchange = self.session_exchange.swap(0, Ordering::AcqRel);
        self.session_rx_started.store(0, Ordering::Release);
        if exchange != 0 {
            self.push_session_event_for(
                ExchangeId(exchange),
                SessionEvent::ForeignForward {
                    pre_idle_ticks: ev.pre_idle_ticks,
                    sample_count: ev.sample_count,
                },
            );
        }
        self.push_sniffer_capture(ev);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn push_sniffer_capture(&self, ev: RxCompletedEvent) {
        if self.rx_ring.try_push(ev).is_err() {
            self.sniff_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn push_session_event(&self, event: SessionEvent) {
        let exchange_id = ExchangeId(self.active_epoch.load(Ordering::Relaxed));
        self.push_session_event_for(exchange_id, event);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn push_session_event_for(&self, exchange_id: ExchangeId, event: SessionEvent) {
        if self.session_events.try_push((exchange_id, event)).is_err() {
            self.session_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn submit_tx(
        &self,
        halfbit_data: &[u8; HalfBitBuffer::DATA_LEN],
        halfbit_len: u8,
        expects_backward: bool,
        min_idle_ticks: u8,
    ) -> bool {
        let exchange_id = match self.current_exchange() {
            ExchangeId(0) => self.begin_exchange(),
            id => id,
        };
        self.submit_tx_for(
            halfbit_data,
            halfbit_len,
            expects_backward,
            min_idle_ticks,
            exchange_id,
        )
    }

    pub fn submit_tx_for(
        &self,
        halfbit_data: &[u8; HalfBitBuffer::DATA_LEN],
        halfbit_len: u8,
        expects_backward: bool,
        min_idle_ticks: u8,
        exchange_id: ExchangeId,
    ) -> bool {
        if self.control.requested.load(Ordering::Acquire) != 0 {
            return false;
        }
        self.cmd.send_packed_tx(
            halfbit_data,
            halfbit_len,
            expects_backward,
            min_idle_ticks,
            exchange_id,
        )
    }

    pub fn submit_answer(&self, rx_epoch: u8, hb: &HalfBitBuffer) -> bool {
        self.answer.stage(rx_epoch, hb)
    }

    pub fn answer_counts(&self) -> AnswerCounts {
        AnswerCounts {
            sent: self.answers_sent.load(Ordering::Relaxed),
            stale: self.answers_stale.load(Ordering::Relaxed),
            expired: self.answers_expired.load(Ordering::Relaxed),
            late: self.answers_late.load(Ordering::Relaxed),
            rejected: self.answers_rejected.load(Ordering::Relaxed),
            collided: self.answers_collided.load(Ordering::Relaxed),
        }
    }

    pub fn tx_yields(&self) -> u32 {
        self.tx_yields.load(Ordering::Relaxed)
    }

    pub fn tx_voided(&self) -> u32 {
        self.tx_voided.load(Ordering::Relaxed)
    }

    pub fn last_answer_arm_idle_ticks(&self) -> u32 {
        self.last_answer_arm_idle_ticks.load(Ordering::Relaxed)
    }

    pub fn queue_tx(
        &self,
        halfbit_data: &[u8; HalfBitBuffer::DATA_LEN],
        halfbit_len: u8,
        expects_backward: bool,
        min_idle_ticks: u8,
    ) -> bool {
        let exchange_id = match self.current_exchange() {
            ExchangeId(0) => self.begin_exchange(),
            id => id,
        };
        self.queue_tx_for(
            halfbit_data,
            halfbit_len,
            expects_backward,
            min_idle_ticks,
            exchange_id,
        )
    }

    pub fn queue_tx_for(
        &self,
        halfbit_data: &[u8; HalfBitBuffer::DATA_LEN],
        halfbit_len: u8,
        expects_backward: bool,
        min_idle_ticks: u8,
        exchange_id: ExchangeId,
    ) -> bool {
        if self.control.requested.load(Ordering::Acquire) != 0 {
            return false;
        }
        if halfbit_len == 0 || halfbit_len > HalfBitBuffer::MAX_DATA_HALF_BITS {
            return false;
        }
        self.tx_queue
            .try_push(TxRequest {
                exchange_id,
                batched: true,
                data: *halfbit_data,
                len: halfbit_len,
                expects_backward,
                min_idle_ticks,
            })
            .is_ok()
    }

    pub const fn tx_batch_capacity() -> usize {
        TX_RING_CAP - 1
    }

    pub fn cancel_pending_tx(&self) {
        self.cancel_exchange(self.current_exchange());
    }

    pub fn cancel_exchange(&self, exchange_id: ExchangeId) {
        self.control
            .requested
            .store(exchange_id.0, Ordering::Release);
    }

    pub fn cancellation_acknowledged(&self, exchange_id: ExchangeId) -> bool {
        self.control.acknowledged.load(Ordering::Acquire) == exchange_id.0
    }

    pub fn take_batch_aborted(&self) -> bool {
        self.batch_aborted.swap(false, Ordering::AcqRel)
    }

    pub fn pop_session_event_for(&self, exchange_id: ExchangeId) -> Option<SessionEvent> {
        while let Some((stamped, event)) = self.session_events.try_pop() {
            if stamped == exchange_id {
                return Some(event);
            }
            self.stale_session_events.fetch_add(1, Ordering::Relaxed);
        }
        None
    }

    pub fn pop_any_session_event(&self) -> Option<SessionEvent> {
        self.session_events.try_pop().map(|(_, event)| event)
    }

    pub fn pop_session_event(&self) -> Option<SessionEvent> {
        self.pop_session_event_for(self.current_exchange())
    }

    pub fn begin_exchange(&self) -> ExchangeId {
        let id = self
            .exchange_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                Some(old.wrapping_add(1).max(1))
            })
            .unwrap_or_else(|old| old);
        ExchangeId(id.wrapping_add(1).max(1))
    }

    pub fn current_exchange(&self) -> ExchangeId {
        ExchangeId(self.exchange_epoch.load(Ordering::Acquire))
    }

    pub fn stale_session_events(&self) -> u32 {
        self.stale_session_events.load(Ordering::Relaxed)
    }

    pub fn pop_rx(&self) -> Option<RxCompletedEvent> {
        self.rx_ring.try_pop()
    }

    pub fn take_sniff_dropped(&self) -> u32 {
        self.sniff_dropped.swap(0, Ordering::AcqRel)
    }

    pub fn bus_failure_flags(&self) -> u32 {
        self.bus_failure_flags.load(Ordering::Relaxed)
    }

    pub fn take_line_held_counts(&self) -> (u32, u32) {
        let packed = self.line_held.swap(0, Ordering::Relaxed);
        (packed >> 16, packed & 0xFFFF)
    }

    pub fn bus_power_down_entries(&self) -> u32 {
        self.bus_power_down_entries.load(Ordering::Relaxed)
    }

    pub fn system_failure_entries(&self) -> u32 {
        self.system_failure_entries.load(Ordering::Relaxed)
    }

    pub fn wire_tick_counts(&self) -> (u32, u32, u32) {
        (
            self.wire_ticks_total.load(Ordering::Relaxed),
            self.wire_ticks_active.load(Ordering::Relaxed),
            self.wire_ticks_tx.load(Ordering::Relaxed),
        )
    }

    pub fn take_session_dropped(&self) -> u32 {
        self.session_dropped.swap(0, Ordering::AcqRel)
    }

    pub fn idle_ticks(&self) -> u32 {
        self.idle_ticks.load(Ordering::Acquire)
    }

    pub fn session_expects_backward(&self) -> bool {
        self.session_exchange.load(Ordering::Acquire) != 0
    }

    pub fn close_backward_window(&self) -> Option<u8> {
        let current = self.exchange_epoch.load(Ordering::Acquire);
        self.session_exchange
            .compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Self::unpack_rx_started(self.session_rx_started.swap(0, Ordering::AcqRel))
    }

    pub fn take_session_rx_started(&self) -> Option<u8> {
        let current = self.exchange_epoch.load(Ordering::Acquire);
        if self.session_exchange.load(Ordering::Acquire) != current {
            return None;
        }
        let packed = self.session_rx_started.swap(0, Ordering::AcqRel);
        Self::unpack_rx_started(packed)
    }

    fn unpack_rx_started(packed: u32) -> Option<u8> {
        packed
            .checked_sub(1)
            .map(|ticks| u8::try_from(ticks).unwrap_or(u8::MAX))
    }
}

/// # Safety
/// `user_ctx` is the `PhyIsrCore` registered with the timer, which outlives the timer.
#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub unsafe extern "C" fn dali_phy_alarm_isr(
    _timer: *mut core::ffi::c_void,
    _edata: *const core::ffi::c_void,
    user_ctx: *mut core::ffi::c_void,
) -> bool {
    // SAFETY: `user_ctx` is the `PhyIsrCore` registered with the driver, and this is the GPTimer alarm callback.
    unsafe { (*user_ctx.cast::<PhyIsrCore>()).tick() };
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_length::FORWARD_LENGTH_MIN_SAMPLES;
    use crate::fsm::{PHY_TICK_US, TX_ARM_LEAD_TICKS, TX_HALF_BIT_TICKS};

    fn test_core(set: &mut u32, clr: &mut u32, input: &u32) -> PhyIsrCore {
        // SAFETY: the three "registers" outlive the core within each test.
        let gpio = unsafe { RegisterGpio::new(set, clr, input, 0b0001, 0b0010) };
        PhyIsrCore::new(gpio)
    }

    #[test]
    fn a_submitted_frame_is_transmitted_and_reported_complete() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, false, 0));
        assert!(!core.submit_tx(&data, 16, false, 0));

        let mut completed = false;
        for _ in 0..128 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            if let Some(SessionEvent::TxComplete { .. }) = core.pop_session_event() {
                completed = true;
                break;
            }
        }
        assert!(completed, "the interrupt must report the frame complete");
    }

    #[test]
    fn a_gated_frame_waits_for_its_settling_and_then_goes_out_at_once() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        const GATE: u8 = 20;
        assert!(core.submit_tx(&data, 16, false, GATE));

        for _ in 0..u32::from(GATE) - 1 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            assert!(
                core.pop_session_event().is_none(),
                "a frame must not start before its settling gate"
            );
        }

        let mut armed_after = None;
        for _ in 0..128 {
            // SAFETY: see above.
            unsafe { core.tick() };
            if let Some(SessionEvent::TxComplete { pre_idle_ticks }) = core.pop_session_event() {
                armed_after = Some(pre_idle_ticks);
                break;
            }
        }
        let armed_after = armed_after.expect("the held frame must eventually go out");
        assert_eq!(
            armed_after,
            u16::from(GATE),
            "the interrupt must arm on the tick the gate opens, not ticks later — \
             an overshoot here is exactly the miss this design removes"
        );
    }

    unsafe fn tick_n(core: &PhyIsrCore, n: usize) {
        for _ in 0..n {
            // SAFETY: a single-threaded test stands in for the ISR context.
            unsafe { core.tick() };
        }
    }

    unsafe fn transmitted_within(core: &PhyIsrCore, ticks: usize) -> bool {
        let mut seen = false;
        for _ in 0..ticks {
            // SAFETY: a single-threaded test stands in for the ISR context.
            unsafe { core.tick() };
            while let Some(event) = core.pop_session_event() {
                if matches!(event, SessionEvent::TxComplete { .. }) {
                    seen = true;
                }
            }
        }
        seen
    }

    #[test]
    fn idle_ticks_count_towards_total_and_nothing_else() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 50) };
        assert_eq!(core.wire_tick_counts(), (50, 0, 0));
    }

    #[test]
    fn a_capture_carries_the_worst_interrupt_gap_seen_while_receiving() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let mut event = RxCompletedEvent::new();
        // SAFETY: test models consecutive calls from the sole ISR owner.
        unsafe {
            core.track_rx_timing(RxState::Empty, RxState::Receiving, PHY_TICK_US);
            core.track_rx_timing(RxState::Receiving, RxState::Receiving, LATE_TICK_US + 1);
            core.stamp_rx_timing(&mut event);
        }
        assert_eq!(event.max_sample_gap_us, LATE_TICK_US + 1);
        assert!(event.timing_degraded);
    }

    #[test]
    fn a_transmission_is_booked_as_active_and_ours() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, false, 0));
        const WINDOW: usize = 256;
        // SAFETY: single-threaded test standing in for the ISR context.
        assert!(unsafe { transmitted_within(&core, WINDOW) });
        let (total, active, tx) = core.wire_tick_counts();
        assert_eq!(total as usize, WINDOW);
        assert!(tx > 0, "a frame went out, so some ticks were ours");
        assert!(active >= tx, "every transmitting tick is an active tick");
        assert!(
            active < total,
            "a 16-bit frame is ~152 ticks; the window must contain idle ticks too"
        );
    }

    #[test]
    fn a_frame_cancelled_on_a_busy_bus_never_goes_out_when_the_bus_recovers() {
        const RECESSIVE: u32 = 0b0010;
        const DOMINANT: u32 = 0;
        let (mut set, mut clr) = (0u32, 0u32);
        let input = core::cell::UnsafeCell::new(DOMINANT);
        // SAFETY: the three "registers" outlive the core within this test; the cell lets the test move the bus.
        let gpio =
            unsafe { RegisterGpio::new(&raw mut set, &raw mut clr, input.get(), 0b0001, 0b0010) };
        let core = PhyIsrCore::new(gpio);

        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 20) };

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, false, 0));
        // SAFETY: see above.
        unsafe { tick_n(&core, 20) };
        assert!(
            !core.submit_tx(&data, 16, false, 0),
            "precondition: the cell still holds the frame, so this is the case \
             the cancel has to reach"
        );

        core.cancel_pending_tx();
        // SAFETY: see above. One tick is all the cancel needs.
        unsafe { tick_n(&core, 1) };
        assert!(
            core.submit_tx(&data, 16, false, 0),
            "the cancel must have emptied the cell — a full cell here is the bug"
        );
        core.cancel_pending_tx();
        // SAFETY: see above.
        unsafe { tick_n(&core, 1) };

        // SAFETY: single-threaded test; nothing else holds a reference.
        unsafe { *input.get() = RECESSIVE };
        // SAFETY: see above.
        assert!(
            !unsafe { transmitted_within(&core, 1024) },
            "a frame its caller gave up on must never reach the wire, however \
             long the bus takes to recover"
        );

        assert!(core.submit_tx(&data, 16, false, 0));
        // SAFETY: see above.
        assert!(
            unsafe { transmitted_within(&core, 1024) },
            "the bus recovered and the PHY transmits — so the silence above was \
             about the cancelled frame, not about a wedged rig"
        );
    }

    #[test]
    fn a_completion_from_an_abandoned_exchange_is_not_this_exchange_s() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;

        let epoch_n = core.begin_exchange();
        assert!(core.submit_tx(&data, 16, false, 0));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 8) };
        assert!(
            core.pop_session_event_for(epoch_n).is_none(),
            "precondition: the frame is still in flight, which is the window \
             this test is about"
        );

        let epoch_next = core.begin_exchange();
        assert_ne!(epoch_n, epoch_next);
        while core.pop_any_session_event().is_some() {}

        // SAFETY: see above.
        unsafe { tick_n(&core, 256) };
        assert!(
            core.pop_session_event_for(epoch_next).is_none(),
            "the new exchange must not be handed the old frame's completion — \
             its `pre_idle_ticks` describes a settling window that belongs to \
             another frame"
        );
        assert!(
            core.stale_session_events() > 0,
            "the discard must be counted, or a timeout that keeps happening is \
             invisible"
        );

        assert!(core.submit_tx(&data, 16, false, 0));
        assert!(
            // SAFETY: see above.
            unsafe { completes_within(&core, epoch_next, 1024) },
            "an exchange must still hear its OWN frame; the filter is a filter, \
             not a mute"
        );
    }

    #[test]
    fn an_acknowledged_cancel_cannot_delete_the_next_request() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let mut first = [0u8; HalfBitBuffer::DATA_LEN];
        first[0] = 0b0101_0101;
        first[1] = 0b0101_0101;
        let mut next = first;
        next[0] = 0b0110_0110;

        let abandoned = core.begin_exchange();
        assert!(core.submit_tx_for(&first, 16, false, 0, abandoned));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 8) };
        core.cancel_exchange(abandoned);
        unsafe { core.tick() };
        assert!(core.cancellation_acknowledged(abandoned));

        let retry = core.begin_exchange();
        assert!(core.submit_tx_for(&next, 16, false, 0, retry));
        assert!(
            unsafe { completes_within(&core, retry, 512) },
            "completion of the cancelled frame must not clear the retry"
        );
        assert!(core.pop_session_event_for(abandoned).is_none());
    }

    #[test]
    fn a_retry_is_rejected_until_the_cancel_is_acknowledged() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let mut first = [0u8; HalfBitBuffer::DATA_LEN];
        first[0] = 0b0101_0101;
        first[1] = 0b0101_0101;
        let mut retry_data = first;
        retry_data[0] = 0b0110_0110;

        let abandoned = core.begin_exchange();
        assert!(core.submit_tx_for(&first, 16, false, 0, abandoned));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 8) };
        core.cancel_exchange(abandoned);
        let retry = core.begin_exchange();
        assert!(
            !core.submit_tx_for(&retry_data, 16, false, 0, retry),
            "a delayed cancellation must close the producer gate"
        );

        // SAFETY: apply the cancellation before submitting the retry.
        unsafe { core.tick() };
        assert!(core.cancellation_acknowledged(abandoned));
        assert!(core.submit_tx_for(&retry_data, 16, false, 0, retry));
        assert!(unsafe { completes_within(&core, retry, 512) });
    }

    unsafe fn completes_within(core: &PhyIsrCore, epoch: ExchangeId, ticks: usize) -> bool {
        for _ in 0..ticks {
            // SAFETY: a single-threaded test stands in for the ISR context.
            unsafe { core.tick() };
            if matches!(
                core.pop_session_event_for(epoch),
                Some(SessionEvent::TxComplete { .. })
            ) {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_queued_batch_transmits_every_frame_in_order() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        for _ in 0..PhyIsrCore::tx_batch_capacity() {
            assert!(core.queue_tx(&data, 16, false, 2));
        }
        assert!(
            !core.queue_tx(&data, 16, false, 2),
            "the ring holds CAP-1, and the caller must be told when it is full"
        );

        let mut sent = 0usize;
        for _ in 0..4096 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            while let Some(event) = core.pop_session_event() {
                if matches!(event, SessionEvent::TxComplete { .. }) {
                    sent += 1;
                }
            }
        }
        assert_eq!(sent, PhyIsrCore::tx_batch_capacity());
        assert!(!core.take_batch_aborted(), "nothing invalidated this batch");
    }

    #[test]
    fn a_query_in_a_batch_holds_the_tail_until_its_window_closes() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.queue_tx(&data, 16, true, 2));
        assert!(core.queue_tx(&data, 16, false, 2));

        let mut sent = 0usize;
        for _ in 0..2048 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            while let Some(event) = core.pop_session_event() {
                if matches!(event, SessionEvent::TxComplete { .. }) {
                    sent += 1;
                }
            }
        }
        assert_eq!(
            sent, 1,
            "the second frame must wait behind the backward window of the first"
        );
        assert!(core.session_expects_backward(), "the window is still open");

        let _ = core.close_backward_window();
        for _ in 0..2048 {
            // SAFETY: see above.
            unsafe { core.tick() };
            while let Some(event) = core.pop_session_event() {
                if matches!(event, SessionEvent::TxComplete { .. }) {
                    sent += 1;
                }
            }
        }
        assert_eq!(sent, 2, "the tail must go out once the window is closed");
    }

    #[test]
    fn tx_completion_reports_the_idle_ticks_it_was_armed_after() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let idle_ticks_before_arming = 20u16;
        for _ in 0..idle_ticks_before_arming {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
        }

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, false, 0));

        let mut reported = None;
        for _ in 0..128 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            if let Some(SessionEvent::TxComplete { pre_idle_ticks }) = core.pop_session_event() {
                reported = Some(pre_idle_ticks);
                break;
            }
        }

        assert_eq!(
            reported,
            Some(idle_ticks_before_arming),
            "the completion event must carry the idle count from the arming tick"
        );
    }

    #[test]
    fn expecting_a_backward_frame_survives_tx_completion() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        assert!(core.submit_tx(&data, 8, true, 0));

        for _ in 0..128 {
            // SAFETY: as above.
            unsafe { core.tick() };
            if core.session_expects_backward() {
                break;
            }
        }
        assert!(
            core.session_expects_backward(),
            "a query must leave the window open for its answer"
        );

        let _ = core.close_backward_window();
        assert!(!core.session_expects_backward());
    }

    #[test]
    fn idle_ticks_climb_while_the_bus_is_recessive() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        assert_eq!(core.idle_ticks(), 0);
        for _ in 0..20 {
            // SAFETY: as above.
            unsafe { core.tick() };
        }
        assert!(core.idle_ticks() >= 20, "idle ticks: {}", core.idle_ticks());
    }

    #[test]
    fn overflowing_the_session_ring_is_counted_not_lost_silently() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);

        for _ in 0..SESSION_RING_CAP * 2 {
            core.push_session_event(SessionEvent::TxRejected);
        }
        assert!(core.take_session_dropped() > 0);
        assert_eq!(
            core.take_session_dropped(),
            0,
            "the count is read-and-clear"
        );
    }

    #[test]
    fn a_frame_submitted_during_a_foreign_frame_is_held_not_rejected() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        for _ in 0..64 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
        }

        // SAFETY: same provenance as the pointer the core reads through.
        unsafe { *input_ptr = 0; tick_n(&core, 8) };

        let data = test_forward_frame();
        assert!(core.submit_tx(&data, 16, false, 0));
        for _ in 0..8 {
            // SAFETY: see above.
            unsafe { core.tick() };
            assert!(
                !matches!(core.pop_session_event(), Some(SessionEvent::TxRejected)),
                "a foreign frame mid-air must HOLD the pending frame, not reject it"
            );
        }

        // SAFETY: same provenance as the pointer the core reads through.
        unsafe { *input_ptr = 0b0010 };
        let mut completed = false;
        for _ in 0..256 {
            // SAFETY: see above.
            unsafe { core.tick() };
            match core.pop_session_event() {
                Some(SessionEvent::TxComplete { .. }) => {
                    completed = true;
                    break;
                }
                Some(SessionEvent::TxRejected) => {
                    panic!("the held frame must not be rejected after the bus clears")
                }
                _ => {}
            }
        }
        assert!(
            completed,
            "the held frame must transmit once the bus is idle again"
        );
    }

    #[test]
    fn a_foreign_edge_inside_the_arming_lead_yields_and_the_frame_goes_out_after_it() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 64) };

        let data = test_forward_frame();
        assert!(core.submit_tx(&data, 16, false, 0));
        // SAFETY: same. One tick: the request is pulled and armed (lead = 3).
        unsafe { core.tick() };
        // SAFETY: same provenance as the pointer the core reads through.
        unsafe { *input_ptr = 0; core.tick() };
        assert_eq!(core.tx_yields(), 1, "the lead must yield to a dominant line");
        assert!(core.pop_session_event().is_none(), "a yield is not reported — the frame is still held");

        receive_foreign_frame(&core, input_ptr, &EIGHT_BITS);
        let mut completed = false;
        for _ in 0..256 {
            // SAFETY: same.
            unsafe { core.tick() };
            match core.pop_session_event() {
                Some(SessionEvent::TxComplete { .. }) => {
                    completed = true;
                    break;
                }
                Some(_) => panic!("the held frame must complete, not be rejected or voided"),
                None => {}
            }
        }
        assert!(
            completed,
            "the yielded frame must go out once the bus is idle again"
        );
        assert_eq!(core.tx_voided(), 0);
        assert!(
            !core.take_batch_aborted(),
            "a backward frame voids no held frame"
        );
    }

    #[test]
    fn a_foreign_forward_frame_voids_a_held_frame_and_the_session_hears_it() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 16) };

        let data = test_forward_frame();
        let held_exchange = core.begin_exchange();
        assert!(core.submit_tx_for(&data, 16, false, 200, held_exchange));
        // SAFETY: same.
        unsafe { core.tick() };
        assert!(core.pop_session_event().is_none());

        let later_exchange = core.begin_exchange();

        receive_foreign_frame(&core, input_ptr, &SIXTEEN_BITS);
        let mut voided = false;
        for _ in 0..8 {
            // SAFETY: same.
            unsafe { core.tick() };
            if let Some(SessionEvent::TxVoided) = core.pop_session_event_for(held_exchange) {
                voided = true;
                break;
            }
        }
        assert!(voided, "the void must be reported the tick the foreign frame completes");
        assert!(core.pop_session_event_for(later_exchange).is_none(),
            "a held request must not borrow the exchange open when it is voided");
        assert_eq!(core.tx_voided(), 1);
        assert!(
            !core.take_batch_aborted(),
            "a single-frame void must not leak into the next batch"
        );
        // SAFETY: same.
        unsafe { tick_n(&core, 400) };
        assert!(
            core.pop_session_event().is_none(),
            "a voided frame must not transmit later"
        );
        assert!(
            core.submit_tx(&data, 16, false, 0),
            "the cell is free after a void"
        );
    }

    #[test]
    fn a_foreign_edge_inside_an_answers_lead_counts_it_stale() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));
        // SAFETY: single-threaded test standing in for the ISR context.
        while core.last_answer_arm_idle_ticks() == 0 {
            unsafe { core.tick() };
        }
        // SAFETY: same provenance as the pointer the core reads through.
        unsafe { *input_ptr = 0 };
        // SAFETY: same.
        unsafe { core.tick() };
        let c = core.answer_counts();
        assert_eq!(c.stale, 1, "a yielded answer is stale: {c:?}");
        assert_eq!(
            c.sent + c.late + c.collided + c.rejected + c.expired,
            0,
            "{c:?}"
        );
    }

    static FAKE_CLOCK_US: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(1_000);
    unsafe extern "C" fn fake_clock() -> i64 {
        FAKE_CLOCK_US.load(Ordering::Relaxed)
    }

    #[test]
    fn a_late_tick_is_booked_with_its_gap_and_the_interrupted_task() {
        let (mut set, mut clr) = (0u32, 0u32);
        let input = 0b0010u32;
        let core = test_core(&mut set, &mut clr, &input);
        let frame: [u32; 2] = [0xCAFE_F00D, 0xBEEF_0004];
        let tcb: usize = frame.as_ptr() as usize;
        let tcb_ptr = core::ptr::addr_of!(tcb) as usize;
        let slot: usize = tcb_ptr;
        static NESTING: AtomicU32 = AtomicU32::new(1);
        FAKE_CLOCK_US.store(1_000, Ordering::Relaxed);
        core.install_tick_probe(fake_clock, &slot, &slot, NESTING.as_ptr(), NESTING.as_ptr());

        for i in 1..=4 {
            FAKE_CLOCK_US.store(
                1_000 + i64::from(i) * i64::from(PHY_TICK_US),
                Ordering::Relaxed,
            );
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
        }
        assert_eq!(core.late_tick_counts(), (0, 0));
        assert!(core.take_late_tick().is_none());

        FAKE_CLOCK_US.fetch_add(3 * i64::from(PHY_TICK_US), Ordering::Relaxed);
        // SAFETY: same.
        unsafe { core.tick() };
        let late = core
            .take_late_tick()
            .expect("a coalesced alarm is a late tick");
        assert_non_nested_late(late, tcb_ptr);
        assert_eq!(core.late_tick_counts(), (1, 3 * PHY_TICK_US));
        assert!(core.take_late_tick().is_none(), "cleared on read");

        NESTING.store(2, Ordering::Relaxed);
        FAKE_CLOCK_US.fetch_add(3 * i64::from(PHY_TICK_US), Ordering::Relaxed);
        // SAFETY: same.
        unsafe { core.tick() };
        let nested = core.take_late_tick().expect("still a late tick");
        assert_eq!(
            (
                nested.interrupted_pc,
                nested.interrupted_ra,
                nested.nesting_depth
            ),
            (0, 0, 2)
        );
    }

    const SIXTEEN_BITS: [bool; 16] = [
        true, false, true, false, true, false, true, false, false, false, false, false, true,
        true, true, true,
    ];
    const EIGHT_BITS: [bool; 8] = [true, false, true, false, true, false, true, false];

    fn assert_non_nested_late(late: LateTick, tcb_ptr: usize) {
        assert_eq!(late.gap_us, 3 * PHY_TICK_US);
        assert_eq!(late.interrupted_task, tcb_ptr, "the current-task slot is read");
        assert_eq!(late.interrupted_pc, 0xCAFE_F00D, "mepc through the TCB frame");
        assert_eq!(late.interrupted_ra, 0xBEEF_0004, "ra is the frame's second word");
        assert_eq!(late.nesting_depth, 1);
    }

    fn test_forward_frame() -> [u8; HalfBitBuffer::DATA_LEN] {
        let mut data = [0; HalfBitBuffer::DATA_LEN];
        data[..2].copy_from_slice(&[0b0101_0101, 0b0101_0101]);
        data
    }

    fn receive_foreign_frame(core: &PhyIsrCore, input_ptr: *mut u32, bits: &[bool]) {
        for &bit in core::iter::once(&true).chain(bits) {
            let (first, second) = if bit { (0, 0b0010) } else { (0b0010, 0) };
            drive(core, input_ptr, first, TX_HALF_BIT_TICKS);
            drive(core, input_ptr, second, TX_HALF_BIT_TICKS);
        }
        drive(core, input_ptr, 0b0010, 24);
    }

    unsafe fn send_query(core: &PhyIsrCore) {
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, true, 0));
        for _ in 0..256 {
            // SAFETY: a single-threaded test stands in for the ISR context.
            unsafe { core.tick() };
            if core.session_expects_backward() {
                break;
            }
        }
        assert!(
            core.session_expects_backward(),
            "the query must leave its window open"
        );
        assert!(matches!(
            core.pop_session_event(),
            Some(SessionEvent::TxComplete { .. })
        ));
    }

    #[test]
    fn a_reception_that_never_completes_still_reports_when_it_began() {
        fn settling_after_idle(idle_ticks: u32) -> u8 {
            let (mut set, mut clr) = (0u32, 0u32);
            let mut input = 0b0010u32;
            let input_ptr = std::ptr::addr_of_mut!(input);
            // SAFETY: `input` outlives the core; all writes use `input_ptr`.
            let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { send_query(&core) };
            assert_eq!(
                core.take_session_rx_started(),
                None,
                "an open window with nothing on the wire has seen no reception"
            );
            drive(&core, input_ptr, 0b0010, idle_ticks);
            drive(&core, input_ptr, 0, TX_HALF_BIT_TICKS);
            let reported = core
                .take_session_rx_started()
                .expect("a reception began inside the open window");
            assert_eq!(
                core.take_session_rx_started(),
                None,
                "the reading is taken, not latched"
            );
            reported
        }

        let short = settling_after_idle(20);
        let long = settling_after_idle(100);
        assert_eq!(
            u32::from(long) - u32::from(short),
            80,
            "reported settling must move with the wire ({short} then {long})"
        );
        assert_eq!(
            settling_after_idle(400),
            u8::MAX,
            "saturated means longer than any Table 22 window"
        );
    }

    #[test]
    fn a_forward_length_capture_during_an_open_window_reaches_the_sniffer_ring_with_a_marker() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { send_query(&core) };

        receive_foreign_frame(&core, input_ptr, &SIXTEEN_BITS);

        let capture = core
            .pop_rx()
            .expect("a forward-length capture goes to the sniffer");
        assert!(
            capture.sample_count >= FORWARD_LENGTH_MIN_SAMPLES,
            "{} samples is not a forward frame",
            capture.sample_count
        );
        assert!(
            matches!(
                core.pop_session_event(),
                Some(SessionEvent::ForeignForward { sample_count, .. })
                    if sample_count == capture.sample_count
            ),
            "the session must be told a foreign forward frame landed in its window"
        );
        assert!(
            !core.session_expects_backward(),
            "the marker closes the window: nothing arriving later is our answer"
        );
    }

    #[test]
    fn a_backward_length_capture_during_an_open_window_goes_to_the_session_ring() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { send_query(&core) };

        receive_foreign_frame(&core, input_ptr, &EIGHT_BITS);

        match core.pop_session_event() {
            Some(SessionEvent::RxComplete(ev)) => assert!(
                ev.sample_count < FORWARD_LENGTH_MIN_SAMPLES,
                "{} samples is not a backward frame",
                ev.sample_count
            ),
            _ => panic!("a backward-length capture in an open window belongs to the session"),
        }
        assert!(core.pop_rx().is_none(), "…and not to the sniffer");
        assert!(!core.session_expects_backward());
    }

    unsafe fn hold_a_two_frame_unit(core: &PhyIsrCore, gate: u8) {
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.queue_tx(&data, 16, false, gate));
        assert!(core.queue_tx(&data, 16, false, gate));
        // SAFETY: a single-threaded test stands in for the ISR context.
        unsafe { tick_n(core, 4) };
        assert!(core.pop_session_event().is_none());
    }

    #[test]
    fn a_foreign_forward_frame_voids_a_batch_tail() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { hold_a_two_frame_unit(&core, 200) };

        receive_foreign_frame(&core, input_ptr, &SIXTEEN_BITS);

        assert!(
            core.take_batch_aborted(),
            "a foreign forward frame voids the held tail (102 §11.7.14)"
        );
        assert!(
            core.pop_rx().is_some(),
            "the frame still reaches the sniffer"
        );
        // SAFETY: single-threaded test standing in for the ISR context.
        assert!(
            !unsafe { transmitted_within(&core, 600) },
            "nothing of the voided unit goes out"
        );
    }

    #[test]
    fn a_foreign_backward_frame_does_not_void_a_batch_tail() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { hold_a_two_frame_unit(&core, 100) };

        receive_foreign_frame(&core, input_ptr, &EIGHT_BITS);

        assert!(
            !core.take_batch_aborted(),
            "an answer on the wire is not a reason to drop our unit"
        );
        assert!(
            core.pop_rx().is_some(),
            "…while the capture still reaches the sniffer"
        );
        let mut completed = 0;
        for _ in 0..1000 {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
            while let Some(event) = core.pop_session_event() {
                if matches!(event, SessionEvent::TxComplete { .. }) {
                    completed += 1;
                }
            }
        }
        assert_eq!(
            completed, 2,
            "both frames of the unit go out once their gate opens"
        );
    }

    fn answer_frame() -> HalfBitBuffer {
        HalfBitBuffer {
            data: [0b0101_0101; HalfBitBuffer::DATA_LEN],
            length: 22,
        }
    }

    fn latest_capture_epoch(core: &PhyIsrCore, input_ptr: *mut u32, bits: &[bool]) -> u8 {
        receive_foreign_frame(core, input_ptr, bits);
        core.pop_rx()
            .expect("the foreign forward capture reaches the sniffer")
            .rx_epoch
    }

    #[test]
    fn an_answer_for_the_latest_capture_goes_out_at_the_target_tick() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 160) };

        let c = core.answer_counts();
        assert_eq!(c.sent, 1, "the answer must go out exactly once");
        assert_eq!(c.stale + c.expired + c.rejected + c.collided, 0);
        assert_eq!(
            core.last_answer_arm_idle_ticks(),
            u32::from(ANSWER_ARM_TARGET_IDLE_TICKS),
            "the answer must arm on the target tick, not ticks later"
        );
        assert!(
            core.pop_session_event().is_none(),
            "an answer is not a session event"
        );
        assert!(
            !core.session_expects_backward(),
            "and it leaves no backward expectation"
        );
    }

    #[test]
    fn a_staged_answer_is_discarded_when_a_newer_capture_arrives() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));
        receive_foreign_frame(&core, input_ptr, &SIXTEEN_BITS);
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 160) };

        let c = core.answer_counts();
        assert_eq!(c.sent, 0, "an answer for a passed window must not go out");
        assert!(c.stale >= 1, "it must be counted stale");
    }

    #[test]
    fn an_answer_staged_before_our_own_frame_went_out_is_stale() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.submit_tx(&data, 16, false, 0));
        // SAFETY: single-threaded test standing in for the ISR context.
        assert!(
            unsafe { transmitted_within(&core, 200) },
            "our frame must go out"
        );

        assert!(core.submit_answer(epoch, &answer_frame()));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 160) };

        let c = core.answer_counts();
        assert_eq!(
            c.sent, 0,
            "an answer staged before our own frame is not current"
        );
        assert!(c.stale >= 1);
    }

    #[test]
    fn an_answer_staged_past_the_window_expires_unsent() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 120) };
        assert!(core.submit_answer(epoch, &answer_frame()));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 8) };

        let c = core.answer_counts();
        assert_eq!(c.sent, 0, "nothing may go out past the window");
        assert!(c.expired >= 1, "it must be counted expired");
    }

    #[test]
    fn an_answer_is_held_while_a_foreign_frame_is_mid_air_and_dropped_when_it_completes() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));

        drive(&core, input_ptr, 0, 8);
        let mid = core.answer_counts();
        assert_eq!(mid.sent, 0, "nothing goes out while a frame is on the wire");
        assert_eq!(
            mid.stale + mid.expired,
            0,
            "…and it is not discarded yet, only held"
        );

        receive_foreign_frame(&core, input_ptr, &SIXTEEN_BITS);
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 160) };
        let c = core.answer_counts();
        assert_eq!(c.sent, 0);
        assert!(c.stale >= 1);
    }

    #[test]
    fn a_cancel_of_the_command_path_leaves_a_staged_answer_alone() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));
        core.cancel_pending_tx();
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 160) };

        assert_eq!(
            core.answer_counts().sent,
            1,
            "the cancel must not touch the answer slot"
        );
    }

    #[test]
    fn a_batch_abort_leaves_a_staged_answer_alone() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let mut data = [0u8; HalfBitBuffer::DATA_LEN];
        data[0] = 0b0101_0101;
        data[1] = 0b0101_0101;
        assert!(core.queue_tx(&data, 16, false, 0));
        assert!(core.queue_tx(&data, 16, false, 0));
        // SAFETY: single-threaded test standing in for the ISR context.
        unsafe { tick_n(&core, 2 + usize::from(TX_ARM_LEAD_TICKS)) };
        assert!(core.submit_answer(1, &answer_frame()));

        drive(&core, input_ptr, 0, 14);
        drive(&core, input_ptr, 0b0010, 220);
        assert!(
            core.take_batch_aborted(),
            "the collided batch tail must be voided"
        );
        assert_eq!(
            core.answer_counts().sent,
            1,
            "the abort must leave the answer slot alone"
        );
    }

    #[test]
    fn staging_a_second_answer_while_one_is_pending_is_refused() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes below use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        let epoch = latest_capture_epoch(&core, input_ptr, &SIXTEEN_BITS);
        assert!(core.submit_answer(epoch, &answer_frame()));
        assert!(
            !core.submit_answer(epoch, &answer_frame()),
            "the slot holds one answer"
        );
    }

    fn drive(core: &PhyIsrCore, input_ptr: *mut u32, level: u32, ticks: u32) {
        // SAFETY: same provenance as the pointer the core reads through.
        unsafe { *input_ptr = level };
        for _ in 0..ticks {
            // SAFETY: single-threaded test standing in for the ISR context.
            unsafe { core.tick() };
        }
    }

    #[test]
    fn a_held_line_is_counted_once_and_measured_then_reset() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        drive(&core, input_ptr, 0b0010, 8);
        assert_eq!(
            core.take_line_held_counts(),
            (0, 0),
            "a healthy bus holds nothing"
        );

        drive(
            &core,
            input_ptr,
            0,
            u32::from(crate::fsm::LINE_HELD_TICKS) - 1,
        );
        assert_eq!(
            core.take_line_held_counts(),
            (0, 0),
            "a run shorter than the threshold is a frame, not a held line"
        );

        let held = u32::from(crate::fsm::LINE_HELD_TICKS) + 40;
        drive(&core, input_ptr, 0, held);
        drive(&core, input_ptr, 0b0010, 8);
        let (runs, longest) = core.take_line_held_counts();
        assert_eq!(runs, 1, "one run, counted once however long it lasts");
        assert!(
            longest >= held && longest <= held + u32::from(crate::fsm::LINE_HELD_TICKS),
            "the longest run is measured, got {longest} ticks for a {held}-tick hold"
        );
        assert_eq!(
            core.take_line_held_counts(),
            (0, 0),
            "read-and-reset: a max that only grew would name the worst hold since \
             boot forever, which no correlation against the other board can use"
        );
    }

    #[test]
    fn a_stuck_active_bus_crosses_power_down_then_system_failure() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        drive(&core, input_ptr, 0b0010, 8);
        assert_eq!(
            core.bus_failure_flags(),
            0,
            "a healthy bus classifies as nothing"
        );

        drive(
            &core,
            input_ptr,
            0,
            u32::from(crate::fsm::BUS_POWER_DOWN_TICKS) - 1,
        );
        assert_eq!(
            core.bus_failure_flags(),
            0,
            "44,9 ms of active line is a long frame, not a failure"
        );
        drive(&core, input_ptr, 0, 1);
        assert_eq!(
            core.bus_failure_flags(),
            BUS_FAILURE_POWER_DOWN,
            "45 ms of active line is bus power down"
        );

        drive(
            &core,
            input_ptr,
            0,
            u32::from(crate::fsm::SYSTEM_FAILURE_TICKS - crate::fsm::BUS_POWER_DOWN_TICKS),
        );
        assert_eq!(
            core.bus_failure_flags(),
            BUS_FAILURE_POWER_DOWN | BUS_FAILURE_SYSTEM,
            "550 ms of active line is a system failure"
        );
    }

    #[test]
    fn a_recovered_bus_clears_its_flags_and_keeps_its_entry_counts() {
        let (mut set, mut clr) = (0u32, 0u32);
        let mut input = 0b0010u32;
        let input_ptr = std::ptr::addr_of_mut!(input);
        // SAFETY: `input` outlives the core; all writes use `input_ptr`.
        let core = test_core(&mut set, &mut clr, unsafe { &*input_ptr });

        drive(
            &core,
            input_ptr,
            0,
            u32::from(crate::fsm::SYSTEM_FAILURE_TICKS),
        );
        assert!(
            !frame_active(BusState::Rx, core.bus_failure_flags()),
            "a DOWN bus must release the frame-active gate — nothing can be mid-frame"
        );

        drive(&core, input_ptr, 0b0010, 64);
        assert_eq!(
            core.bus_failure_flags(),
            0,
            "a recovered line is healthy again"
        );
        assert_eq!(core.bus_power_down_entries(), 1, "one entry, counted once");
        assert_eq!(core.system_failure_entries(), 1);
        assert!(
            frame_active(BusState::Rx, 0),
            "a frame on a healthy bus is active"
        );
        assert!(
            !frame_active(BusState::Idle, 0),
            "an idle bus is not mid-frame"
        );
    }
}
