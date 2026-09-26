use core::ptr;
use core::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::sys::EspError;
use esp_idf_svc::sys::{
    esp_intr_free, esp_random, gptimer_alarm_cb_t, gptimer_config_t,
    gptimer_count_direction_t_GPTIMER_COUNT_UP, gptimer_del_timer, gptimer_disable, gptimer_enable,
    gptimer_event_callbacks_t, gptimer_handle_t, gptimer_new_timer,
    gptimer_register_event_callbacks, gptimer_start, gptimer_stop, intr_handle_t,
    soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT,
};

use super::phy_interrupt::{self, PhyIsrLevel};
use dali2rust_dali_phy::timer_alarm::PhyAlarmIsr;

use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::net::address::{decode_wire_address, DaliAddress};
use dali2rust_platform::dali::{BatchOutcome, DaliTransport, Frame24Error, TransferOutcome};

use dali2rust_dali_phy::backward_window::ANSWER_ARM_TARGET_IDLE_TICKS;
use dali2rust_dali_phy::fsm::RX_IDLE_LINE_HIGH_TICKS;
use dali2rust_dali_phy::{
    dali_phy_alarm_isr, settle_ticks_on_wire, ExchangeId, HalfBitBuffer, PhyIsrCore, RegisterGpio,
    RxCompletedEvent, SessionEvent, TxGates, BACKWARD8_SAMPLE_COUNT, BUS_POWER_DOWN_TICKS,
    FORWARD16_SAMPLE_COUNT, FORWARD24_SAMPLE_COUNT, MIN_SAMPLES_FOR_DECODE, PHY_TICK_US,
    RX_RING_CAP, TX_ARM_LEAD_TICKS, TX_HALF_BIT_TICKS,
};

use crate::dali::transport::{
    arrived_too_early_for_a_backward_frame, arrived_too_late_for_a_backward_frame, backward_timing,
    collision_restart_gate, foreign_forward_outcome, held_capture_outcome,
    incomplete_reception_verdict, BackwardTiming, BACKWARD_ACCEPTANCE_FLOOR_US,
    BACKWARD_ACCEPTANCE_LIMIT_US,
};
use dali2rust_dali_codec::codec::{encode_forward16_raw, encode_forward24_raw};
use dali2rust_dali_codec::rx_decode::{RxDecode, SniffedDecode, SniffedFrame};

pub fn isr_core_id() -> Option<u32> {
    dali2rust_dali_phy::isr_core_id()
}

const TIMER_RES_HZ: u32 = 1_000_000;
const TIMER_ALARM_TICKS: u64 = PHY_TICK_US as u64;
const TX_HALF_BIT_MIN_US: u32 = 400;
const TX_HALF_BIT_MAX_US: u32 = 433;
const TX_DOUBLE_HALF_BIT_MIN_US: u32 = 800;
const TX_DOUBLE_HALF_BIT_MAX_US: u32 = 866;
const TX_HALF_BIT_US: u32 = PHY_TICK_US * TX_HALF_BIT_TICKS;
const TX_DOUBLE_HALF_BIT_US: u32 = TX_HALF_BIT_US * 2;
const DEFAULT_IDLE_SETTLE_US: u32 =
    dali2rust_domain::dali::ses::session::DaliPriority::Configuration.min_settle_us();

const SESSION_TIMEOUT_MS: u64 = 500;
const CANCEL_ACK_TIMEOUT_MS: u64 = 10;
const SESSION_QUERY_BACKWARD_LEAD_MS: u64 = 10;
const SESSION_QUERY_BACKWARD_WINDOW_MS: u64 = 25;
const SESSION_POLL_MS: u32 = 1;

const SNIFFER_POLL_MS: u32 = 10;
// DiiA 351 §7
const ARBITRATION_POLL_MS: u32 = 1;

unsafe extern "C" {
    static pxCurrentTCBs: [*mut core::ffi::c_void; 2];
    static port_uxInterruptNesting: [u32; 2];
}

/// # Safety
/// `isr` must be the transport's core and the GPTimer must not be running.
unsafe fn arm_late_tick_probe(isr: &PhyIsrCore) {
    // SAFETY: the kernel's and the port's per-core tables are process-lifetime globals, used only as addresses.
    unsafe {
        isr.install_tick_probe(
            esp_idf_svc::sys::esp_timer_get_time,
            core::ptr::addr_of!(pxCurrentTCBs[0]).cast::<usize>(),
            core::ptr::addr_of!(pxCurrentTCBs[1]).cast::<usize>(),
            core::ptr::addr_of!(port_uxInterruptNesting[0]),
            core::ptr::addr_of!(port_uxInterruptNesting[1]),
        );
    }
    log::info!("DALI ISR late-tick probe armed (esp_timer clock, per-core task + nesting slots)");
}
const SNIFFER_DIAG_INTERVAL_MS: u64 = 5_000;

const fn settle_us_to_idle_ticks(settle_us: u32) -> u32 {
    settle_us.div_ceil(PHY_TICK_US)
}

const TX_SETTLE_UNMEASURED: u32 = u32::MAX;

#[derive(Clone, Copy)]
enum TxFrameLabel {
    Forward16(u16),
    Forward24([u8; 3]),
}

const NO_PREVIOUS_FRAME: u32 = u32::MAX;
const FRAME24_KEY_MARK: u32 = 1 << 24;

impl TxFrameLabel {
    fn key(self) -> u32 {
        match self {
            Self::Forward16(frame) => u32::from(frame),
            Self::Forward24([b0, b1, b2]) => FRAME24_KEY_MARK | u32::from_be_bytes([0, b0, b1, b2]),
        }
    }

    fn addresses_single_gear(self) -> bool {
        let first = match self {
            Self::Forward16(frame) => (frame >> 8) as u8,
            Self::Forward24(bytes) => bytes[0],
        };
        super::forward_addresses_single_gear(first)
    }
}

impl core::fmt::Display for TxFrameLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Forward16(frame) => write!(f, "forward16 0x{frame:04x}"),
            Self::Forward24(b) => {
                write!(f, "forward24 0x{:02x}{:02x}{:02x}", b[0], b[1], b[2])
            }
        }
    }
}

enum TxAttemptOutcome {
    Sent {
        pre_idle_ticks: u16,
    },
    Collision,
    BusBusy,
}

fn out_regs(pin: u8) -> (*mut u32, *mut u32, u32) {
    use esp_idf_svc::sys::{
        GPIO_OUT1_W1TC_REG, GPIO_OUT1_W1TS_REG, GPIO_OUT_W1TC_REG, GPIO_OUT_W1TS_REG,
    };
    if pin < 32 {
        (
            GPIO_OUT_W1TS_REG as *mut u32,
            GPIO_OUT_W1TC_REG as *mut u32,
            1u32 << pin,
        )
    } else {
        (
            GPIO_OUT1_W1TS_REG as *mut u32,
            GPIO_OUT1_W1TC_REG as *mut u32,
            1u32 << (pin - 32),
        )
    }
}

fn in_reg(pin: u8) -> (*const u32, u32) {
    use esp_idf_svc::sys::{GPIO_IN1_REG, GPIO_IN_REG};
    if pin < 32 {
        (GPIO_IN_REG as *const u32, 1u32 << pin)
    } else {
        (GPIO_IN1_REG as *const u32, 1u32 << (pin - 32))
    }
}

struct TransportInner {
    isr: PhyIsrCore,
    timer: gptimer_handle_t,
    intr: intr_handle_t,
    alarm_isr: PhyAlarmIsr,
    _tx_pin: PinDriver<'static, Output>,
    _rx_pin: PinDriver<'static, Input>,
    foreign_frames: AtomicU32,
    last_tx_settle_ticks: AtomicU32,
    previous_forward: AtomicU32,
    cancel_unacknowledged: AtomicU32,
    forward_single_gear: AtomicBool,
    observed_tx: std::sync::OnceLock<dali2rust_platform::dali::ObservedFrameSender>,
    sniffer_counters: std::sync::OnceLock<Arc<dali2rust_platform::dali::PhySnifferCounters>>,
    wire_counters: std::sync::OnceLock<Arc<dali2rust_platform::dali::DaliWireCounters>>,
    arbitration_reflex:
        std::sync::OnceLock<Arc<dali2rust_platform::arbitration::ArbitrationReflex>>,
}

fn new_phy_timer(level: PhyIsrLevel) -> Result<gptimer_handle_t, EspError> {
    // SAFETY: plain FFI construction; `zeroed` gives the driver's documented defaults for `flags`.
    unsafe {
        let mut cfg: gptimer_config_t = core::mem::zeroed();
        cfg.clk_src = soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT;
        cfg.direction = gptimer_count_direction_t_GPTIMER_COUNT_UP;
        cfg.intr_priority = level.driver_priority();
        cfg.resolution_hz = TIMER_RES_HZ;
        let mut handle: gptimer_handle_t = ptr::null_mut();
        EspError::convert(gptimer_new_timer(&cfg, &mut handle))?;
        Ok(handle)
    }
}

/// # Safety
/// As [`start_phy_timer_driver`]; on the raw path also `phy_interrupt::start_raw`.
unsafe fn start_phy_timer(
    inner: *mut TransportInner,
    level: PhyIsrLevel,
) -> Result<intr_handle_t, EspIdfDaliError> {
    if level == PhyIsrLevel::Driver3 {
        // SAFETY: the caller's contract.
        unsafe { start_phy_timer_driver(inner)? };
        return Ok(ptr::null_mut());
    }
    // SAFETY: the caller's contract; `alarm_isr` shares `isr`'s internal-RAM block and outlives the interrupt.
    unsafe {
        let arg = core::ptr::addr_of_mut!((*inner).alarm_isr).cast();
        phy_interrupt::start_raw((*inner).timer, arg, &(*inner).isr, level, TIMER_ALARM_TICKS)
    }
}

/// # Safety
/// `inner` is live, outlives the timer, and the consumers of its rings are already running.
unsafe fn start_phy_timer_driver(inner: *mut TransportInner) -> Result<(), EspError> {
    const _: () = assert!(
        core::mem::size_of::<gptimer_handle_t>() == core::mem::size_of::<*mut core::ffi::c_void>()
    );
    // SAFETY: fn-pointer cast between `extern "C"` signatures asserted ABI-identical above.
    let on_alarm: gptimer_alarm_cb_t = Some(core::mem::transmute::<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            *const core::ffi::c_void,
            *mut core::ffi::c_void,
        ) -> bool,
        unsafe extern "C" fn(
            gptimer_handle_t,
            *const esp_idf_svc::sys::gptimer_alarm_event_data_t,
            *mut core::ffi::c_void,
        ) -> bool,
    >(dali_phy_alarm_isr));
    let cbs = gptimer_event_callbacks_t { on_alarm };
    EspError::convert(gptimer_register_event_callbacks(
        (*inner).timer,
        &cbs,
        core::ptr::addr_of!((*inner).isr) as *mut core::ffi::c_void,
    ))?;
    phy_interrupt::set_phy_alarm((*inner).timer, TIMER_ALARM_TICKS)?;
    EspError::convert(gptimer_enable((*inner).timer))?;
    EspError::convert(gptimer_start((*inner).timer))?;
    log::info!("DALI PHY interrupt: level 3, gptimer driver handler");
    Ok(())
}

pub struct EspIdfDaliTransport {
    inner: *mut TransportInner,
    sniffer_stop: Arc<AtomicBool>,
    sniffer: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: task code reaches `inner` only through atomics and SPSC ends; `Drop` joins the sniffer first.
unsafe impl Send for EspIdfDaliTransport {}

#[derive(Debug)]
pub enum EspIdfDaliError {
    Hal(EspError),
    Other(&'static str),
}

impl From<EspError> for EspIdfDaliError {
    fn from(e: EspError) -> Self {
        Self::Hal(e)
    }
}

fn log_backward_timeout_and_clear(p: *mut TransportInner, window_was_open: bool) -> TransferOutcome {
    let waited_ms = if window_was_open {
        SESSION_QUERY_BACKWARD_WINDOW_MS
    } else {
        SESSION_QUERY_BACKWARD_LEAD_MS
    };
    let rx_started = unsafe { (*p).isr.close_backward_window() };
    if let Some(rx_pre_idle) = rx_started {
        let (idle_ticks, failure_flags) =
            unsafe { ((*p).isr.idle_ticks(), (*p).isr.bus_failure_flags()) };
        let outcome = match super::incomplete_reception_outcome(rx_pre_idle) {
            TransferOutcome::CorruptedInWindow => settle_held_reception(p),
            other => other,
        };
        let (cause, verdict) = match backward_timing(rx_pre_idle) {
            BackwardTiming::TooEarly => (
                BackwardCause::EarlyRejected,
                "before Table 20, not a conforming answer",
            ),
            BackwardTiming::InWindow => (
                BackwardCause::Incomplete,
                "inside Table 20, §8.2.5 backward frame",
            ),
            BackwardTiming::TooLate => {
                (BackwardCause::LateRejected, "past Table 20, not our answer")
            }
        };
        count_backward_cause(p, cause);
        log::warn!(
            "DALI PHY RX: reception began and never completed ({waited_ms} ms; \
             rx_pre_idle={rx_pre_idle} ticks ({} us, {verdict}) \
             idle_ticks={idle_ticks} bus_failure=0x{failure_flags:x})",
            u32::from(rx_pre_idle).saturating_mul(u32::from(PHY_TICK_US))
        );
        return outcome;
    }
    log::info!("DALI PHY RX: backward timeout ({waited_ms} ms)");
    TransferOutcome::NoAnswer
}

fn settle_held_reception(p: *mut TransportInner) -> TransferOutcome {
    let bound = u64::from(BUS_POWER_DOWN_TICKS) * u64::from(PHY_TICK_US);
    let deadline = std::time::Instant::now() + Duration::from_micros(bound);
    loop {
        // SAFETY: `p` is valid for the transport's lifetime, as everywhere on this path.
        let flags = unsafe { (*p).isr.bus_failure_flags() };
        let active = dali2rust_platform::dali::PHY_FRAME_ACTIVE.load(Ordering::Relaxed);
        if let Some(outcome) = incomplete_reception_verdict(flags, active) {
            return outcome;
        }
        if !EspIdfDaliTransport::wait_for_notification_or_deadline(p, deadline) {
            return TransferOutcome::CorruptedInWindow;
        }
    }
}

impl EspIdfDaliTransport {
    fn sleep_while_frame_is_on_the_wire(half_bits: u8, deadline: std::time::Instant) {
        let ticks = u32::from(TX_ARM_LEAD_TICKS) + u32::from(half_bits) * TX_HALF_BIT_TICKS;
        let on_wire = Duration::from_micros(u64::from(ticks) * u64::from(PHY_TICK_US));
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        // sleep-ok: the frame's own measured time on the wire, not a poll step
        std::thread::sleep(on_wire.min(remaining));
    }

    fn wait_for_notification_or_deadline(
        p: *mut TransportInner,
        deadline: std::time::Instant,
    ) -> bool {
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        let step = remaining.min(Duration::from_millis(u64::from(SESSION_POLL_MS)));
        let step_ms = u32::try_from(step.as_millis()).unwrap_or(u32::MAX);
        if !dali2rust_platform::dali::sleep_parks_the_task(step_ms, TICK_MS) {
            // SAFETY: `p` is valid for the transport's lifetime, as everywhere on this path.
            if let Some(w) = unsafe { (*p).wire_counters.get() } {
                w.session_poll_spins.fetch_add(1, Ordering::Relaxed);
            }
        }
        // sleep-ok: one tick at 1 kHz, so this parks
        std::thread::sleep(step);
        true
    }

    fn take_session_event(p: *mut TransportInner) -> Option<SessionEvent> {
        unsafe { (*p).isr.pop_session_event() }
    }

    fn drain_session_events(p: *mut TransportInner) {
        unsafe {
            while (*p).isr.pop_any_session_event().is_some() {}
            let dropped = (*p).isr.take_session_dropped();
            if dropped > 0 {
                log::warn!("DALI PHY session: {dropped} event(s) dropped");
            }
        }
    }

    fn prepare_session_exchange(p: *mut TransportInner) -> Result<ExchangeId, EspIdfDaliError> {
        let pending = unsafe { (*p).cancel_unacknowledged.load(Ordering::Acquire) };
        if pending != 0 {
            let cancelled = ExchangeId(pending);
            if !unsafe { (*p).isr.cancellation_acknowledged(cancelled) } {
                return Err(EspIdfDaliError::Other(
                    "DALI TX cancellation acknowledgement pending",
                ));
            }
            unsafe {
                (*p).cancel_unacknowledged.store(0, Ordering::Release);
            }
        }
        let exchange_id = unsafe { (*p).isr.begin_exchange() };
        Self::drain_session_events(p);
        Ok(exchange_id)
    }

    fn cancel_current_exchange(p: *mut TransportInner) -> Result<(), EspIdfDaliError> {
        let exchange_id = unsafe { (*p).isr.current_exchange() };
        unsafe {
            (*p).cancel_unacknowledged
                .store(exchange_id.0, Ordering::Release);
        }
        unsafe { (*p).isr.cancel_exchange(exchange_id) };
        let deadline = std::time::Instant::now() + Duration::from_millis(CANCEL_ACK_TIMEOUT_MS);
        while !unsafe { (*p).isr.cancellation_acknowledged(exchange_id) } {
            if !Self::wait_for_notification_or_deadline(p, deadline) {
                return Err(EspIdfDaliError::Other(
                    "DALI TX cancellation acknowledgement timeout",
                ));
            }
        }
        unsafe {
            (*p).cancel_unacknowledged.store(0, Ordering::Release);
        }
        Ok(())
    }

    fn cancel_keeping_verdict(p: *mut TransportInner) {
        if let Err(error) = Self::cancel_current_exchange(p) {
            log::warn!(
                "DALI TX: {error:?}; the next exchange refuses to open until the interrupt applies it"
            );
        }
    }

    fn send_one_run(
        &mut self,
        p: *mut TransportInner,
        run: &[(u16, bool, u32)],
    ) -> Result<BatchOutcome, EspIdfDaliError> {
        if run.len() > PhyIsrCore::tx_batch_capacity() {
            return Err(EspIdfDaliError::Other(
                "DALI TX batch exceeds queue capacity",
            ));
        }
        let exchange_id = Self::prepare_session_exchange(p)?;
        unsafe { (*p).isr.take_batch_aborted() };
        for &(frame, expects_backward, min_idle_us) in run {
            let hb = encode_forward16_raw(frame);
            let label = TxFrameLabel::Forward16(frame);
            let gates = Self::tx_gates(p, label, settle_us_to_idle_ticks(min_idle_us));
            if !unsafe {
                (*p).isr
                    .queue_tx_for(&hb.data, hb.length, expects_backward, gates, exchange_id)
            } {
                Self::cancel_keeping_verdict(p);
                return Err(EspIdfDaliError::Other("DALI TX batch queue overflow"));
            }
        }
        self.collect_run_outcomes(p, run)
    }

    fn collect_run_outcomes(
        &mut self,
        p: *mut TransportInner,
        run: &[(u16, bool, u32)],
    ) -> Result<BatchOutcome, EspIdfDaliError> {
        let mut last = TransferOutcome::NoAnswer;
        for &(frame, expects_backward, _) in run {
            let outcome = match Self::await_queued_frame(p, frame)? {
                TxAttemptOutcome::BusBusy => TransferOutcome::BusBusy,
                TxAttemptOutcome::Collision => TransferOutcome::Collision,
                TxAttemptOutcome::Sent { pre_idle_ticks } => {
                    Self::record_tx_settle(p, pre_idle_ticks);
                    if expects_backward {
                        Self::do_receive_backward_outcome(p, Some(frame))?
                    } else {
                        TransferOutcome::NoAnswer
                    }
                }
            };
            last = outcome;
            if unsafe { (*p).isr.take_batch_aborted() } || outcome == TransferOutcome::Collision {
                Self::cancel_keeping_verdict(p);
                return Ok(BatchOutcome::Aborted(outcome));
            }
        }
        Ok(BatchOutcome::Completed(last))
    }

    fn await_queued_frame(
        p: *mut TransportInner,
        frame: u16,
    ) -> Result<TxAttemptOutcome, EspIdfDaliError> {
        let deadline = std::time::Instant::now() + Duration::from_millis(SESSION_TIMEOUT_MS);
        // SAFETY: `p` is valid for the transport lifetime (see callers).
        unsafe {
            (*p).forward_single_gear
                .store(TxFrameLabel::Forward16(frame).addresses_single_gear(), Ordering::Relaxed)
        };
        Self::wait_for_tx_outcome(p, deadline, &format!("forward16 0x{frame:04x} (batched)"))
    }

    fn wait_for_tx_outcome(
        p: *mut TransportInner,
        deadline: std::time::Instant,
        label: &str,
    ) -> Result<TxAttemptOutcome, EspIdfDaliError> {
        loop {
            while let Some(event) = Self::take_session_event(p) {
                if let Some(outcome) = Self::tx_outcome_of(event, label) {
                    return Ok(outcome);
                }
            }
            if std::time::Instant::now() >= deadline
                || !Self::wait_for_notification_or_deadline(p, deadline)
            {
                Self::cancel_keeping_verdict(p);
                return Ok(TxAttemptOutcome::BusBusy);
            }
        }
    }

    fn tx_outcome_of(event: SessionEvent, label: &str) -> Option<TxAttemptOutcome> {
        match event {
            SessionEvent::TxComplete { pre_idle_ticks } => {
                log::info!("DALI PHY TX: {label} (ISR-owned bitbang)");
                Some(TxAttemptOutcome::Sent { pre_idle_ticks })
            }
            SessionEvent::TxCollision => {
                log::warn!("DALI PHY TX: collision on {label}");
                Some(TxAttemptOutcome::Collision)
            }
            SessionEvent::TxRejected => {
                log::warn!("DALI PHY TX: start_tx rejected for {label}");
                Some(TxAttemptOutcome::BusBusy)
            }
            SessionEvent::TxVoided => {
                log::warn!("DALI PHY TX: {label} voided by a foreign forward frame");
                Some(TxAttemptOutcome::BusBusy)
            }
            SessionEvent::RxComplete(_) | SessionEvent::ForeignForward { .. } => {
                log::warn!("DALI PHY TX: unexpected backward RX while waiting for TX completion");
                None
            }
        }
    }

    // IEC 62386-101 §9.1.4, §9.3
    fn tx_gates(p: *mut TransportInner, frame: TxFrameLabel, min_idle_ticks: u32) -> TxGates {
        let key = frame.key();
        // SAFETY: `p` is valid for the transport lifetime (see callers).
        let previous = unsafe { (*p).previous_forward.swap(key, Ordering::Relaxed) };
        // SAFETY: a plain call into the hardware random number generator.
        let draw = unsafe { esp_random() };
        let previous = (previous != NO_PREVIOUS_FRAME).then_some(previous);
        TxGates {
            min_idle_ticks: u8::try_from(min_idle_ticks).unwrap_or(u8::MAX),
            restart_gate: collision_restart_gate(key, previous, draw),
        }
    }

    fn do_send_once(
        p: *mut TransportInner,
        exchange_id: ExchangeId,
        hb: &HalfBitBuffer,
        frame: TxFrameLabel,
        expects_backward: bool,
        min_idle_ticks: u32,
    ) -> Result<TxAttemptOutcome, EspIdfDaliError> {
        let deadline = std::time::Instant::now() + Duration::from_millis(SESSION_TIMEOUT_MS);
        // SAFETY: `p` is valid for the transport lifetime (see callers).
        unsafe {
            (*p).forward_single_gear
                .store(frame.addresses_single_gear(), Ordering::Relaxed)
        };
        let gates = Self::tx_gates(p, frame, min_idle_ticks);
        while !unsafe {
            (*p).isr
                .submit_tx_for(&hb.data, hb.length, expects_backward, gates, exchange_id)
        } {
            if std::time::Instant::now() >= deadline {
                return Err(EspIdfDaliError::Other(
                    "DALI TX command slot busy (timeout)",
                ));
            }
            Self::wait_for_notification_or_deadline(p, deadline);
        }
        Self::sleep_while_frame_is_on_the_wire(hb.length, deadline);
        Self::wait_for_tx_outcome(p, deadline, &frame.to_string())
    }

    fn record_tx_settle(p: *mut TransportInner, pre_idle_ticks: u16) {
        let on_wire = settle_ticks_on_wire(pre_idle_ticks, TX_ARM_LEAD_TICKS);
        unsafe {
            (*p).last_tx_settle_ticks
                .store(u32::from(on_wire), Ordering::Release);
        }
    }

    fn backward_window_has_closed(p: *mut TransportInner) -> bool {
        let idle_ticks = unsafe { (*p).isr.idle_ticks() };
        idle_ticks.saturating_mul(PHY_TICK_US) >= BACKWARD_ACCEPTANCE_LIMIT_US
    }

    fn do_receive_backward_outcome(
        p: *mut TransportInner,
        forward_frame: Option<u16>,
    ) -> Result<TransferOutcome, EspIdfDaliError> {
        let start = std::time::Instant::now();
        let lead = Duration::from_millis(SESSION_QUERY_BACKWARD_LEAD_MS);
        let window = Duration::from_millis(SESSION_QUERY_BACKWARD_WINDOW_MS);
        let mut deadline = start + lead;
        let mut window_was_open = false;

        loop {
            if let Some(outcome) = Self::take_backward_session_outcome(p, forward_frame) {
                return Ok(outcome);
            }

            if Self::backward_window_has_closed(p) {
                return Ok(Self::last_look_then_give_up(
                    p,
                    forward_frame,
                    window_was_open,
                ));
            }

            if unsafe { (*p).isr.session_expects_backward() } {
                window_was_open = true;
                deadline = start + window;
            }

            if std::time::Instant::now() >= deadline {
                return Ok(Self::last_look_then_give_up(
                    p,
                    forward_frame,
                    window_was_open,
                ));
            }
            if !Self::wait_for_notification_or_deadline(p, deadline) {
                return Ok(Self::last_look_then_give_up(
                    p,
                    forward_frame,
                    window_was_open,
                ));
            }
        }
    }

    fn last_look_then_give_up(
        p: *mut TransportInner,
        forward_frame: Option<u16>,
        window_was_open: bool,
    ) -> TransferOutcome {
        Self::take_backward_session_outcome(p, forward_frame)
            .unwrap_or_else(|| log_backward_timeout_and_clear(p, window_was_open))
    }

    fn take_backward_session_outcome(
        p: *mut TransportInner,
        forward_frame: Option<u16>,
    ) -> Option<TransferOutcome> {
        while let Some(event) = Self::take_session_event(p) {
            match event {
                SessionEvent::RxComplete(ev) => {
                    return Some(Self::backward_outcome_of_capture(p, forward_frame, &ev));
                }
                SessionEvent::ForeignForward {
                    pre_idle_ticks,
                    sample_count,
                } => {
                    return Some(Self::foreign_forward_in_window(
                        p,
                        pre_idle_ticks,
                        sample_count,
                    ));
                }
                SessionEvent::TxComplete { .. }
                | SessionEvent::TxCollision
                | SessionEvent::TxRejected
                | SessionEvent::TxVoided => {}
            }
        }
        None
    }

    fn backward_outcome_of_capture(
        p: *mut TransportInner,
        forward_frame: Option<u16>,
        ev: &RxCompletedEvent,
    ) -> TransferOutcome {
        if let Some(outcome) = Self::outside_the_backward_window(p, ev) {
            return outcome;
        }
        if ev.timing_degraded {
            log::warn!(
                "DALI PHY RX: {} us ISR sample gap during backward capture; retaining decode",
                ev.max_sample_gap_us
            );
        }
        let b = ev.decode_backward_byte();
        log::info!("DALI PHY RX: backward decode -> {b:?}");
        if let Some(frame) = forward_frame {
            log_random_address_rx_samples(frame, ev, b);
        }
        if let Some(backward) = b {
            return TransferOutcome::Answer(backward);
        }
        Self::undecodable_in_window(p, ev)
    }

    fn outside_the_backward_window(
        p: *mut TransportInner,
        ev: &RxCompletedEvent,
    ) -> Option<TransferOutcome> {
        if arrived_too_late_for_a_backward_frame(ev) {
            count_foreign_frame(p);
            count_backward_cause(p, BackwardCause::LateRejected);
            log::info!(
                "DALI PHY RX: frame arrived {} ticks after ours — past the \
                 IEC 62386-101 Table 20 backward window, not our answer",
                ev.pre_idle_ticks
            );
            return Some(TransferOutcome::ForeignInWindow);
        }
        if arrived_too_early_for_a_backward_frame(ev) {
            count_foreign_frame(p);
            count_backward_cause(p, BackwardCause::EarlyRejected);
            log::info!(
                "DALI PHY RX: frame arrived {} ticks after ours — below the \
                 IEC 62386-101 Table 20 floor ({} us), counted as foreign",
                ev.pre_idle_ticks,
                BACKWARD_ACCEPTANCE_FLOOR_US
            );
            return Some(TransferOutcome::ForeignInWindow);
        }
        None
    }

    fn undecodable_in_window(p: *mut TransportInner, ev: &RxCompletedEvent) -> TransferOutcome {
        let decoded = ev.decode_sniffed_with_phase();
        match decoded.frame {
            SniffedFrame::Forward16(raw) => {
                count_foreign_frame(p);
                log_sniffed_forward16(raw, decoded.phase_ticks);
                TransferOutcome::ForeignInWindow
            }
            SniffedFrame::Forward24(bytes) => {
                count_foreign_frame(p);
                log::info!(
                    "DALI PHY RX: foreign forward24 in backward window 0x{:02x}{:02x}{:02x}",
                    bytes[0],
                    bytes[1],
                    bytes[2]
                );
                TransferOutcome::ForeignInWindow
            }
            SniffedFrame::Backward8(_) => {
                count_backward_cause(p, BackwardCause::Undecodable);
                log::warn!("DALI PHY RX: undecodable backward frame in window");
                TransferOutcome::CorruptedInWindow
            }
            SniffedFrame::UnsupportedLength(bits) => {
                count_backward_cause(p, BackwardCause::FrameSize);
                log::warn!(
                    "DALI PHY RX: frame size violation in backward window ({bits} bits) \
                     — §8.2.5 backward frame"
                );
                TransferOutcome::CorruptedInWindow
            }
            SniffedFrame::DecodeFailed => {
                count_backward_cause(p, BackwardCause::Undecodable);
                Self::log_undecodable_capture(ev)
            }
        }
    }

    fn log_undecodable_capture(ev: &RxCompletedEvent) -> TransferOutcome {
        let run = ev.longest_dominant_run_ticks();
        let held_ms = PHY_TICK_US.saturating_mul(u32::from(run)) / 1000;
        let outcome = held_capture_outcome(run);
        if outcome == TransferOutcome::BusBusy {
            log::warn!(
                "DALI PHY RX: active {run} ticks ({held_ms} ms) through the backward window \
                 — bus power down past 45 ms (101 Tables 18/19 footnote b), not an answer"
            );
        } else if ev.is_line_held() {
            log::warn!(
                "DALI PHY RX: line HELD through the backward window, {run} ticks \
                 ({held_ms} ms), no edges — a bit-timing violation (101 Tables 18/19), \
                 so a §8.2.5 backward frame"
            );
        } else {
            log::warn!(
                "DALI PHY RX: bit timing violation in backward window (longest \
                 dominant run {run} ticks) — §8.2.5 backward frame"
            );
        }
        outcome
    }

    fn foreign_forward_in_window(
        p: *mut TransportInner,
        pre_idle_ticks: u8,
        sample_count: u16,
    ) -> TransferOutcome {
        count_foreign_frame(p);
        match backward_timing(pre_idle_ticks) {
            BackwardTiming::TooEarly => count_backward_cause(p, BackwardCause::EarlyRejected),
            BackwardTiming::TooLate => count_backward_cause(p, BackwardCause::LateRejected),
            BackwardTiming::InWindow => {}
        }
        let outcome = foreign_forward_outcome(pre_idle_ticks);
        log::info!(
            "DALI PHY RX: foreign forward frame ({sample_count} samples) {pre_idle_ticks} ticks \
             after ours — {}",
            match outcome {
                TransferOutcome::NoAnswer => "past the IEC 62386-101 Table 20 window, our query unanswered",
                _ => "inside our backward window, counted as foreign",
            }
        );
        outcome
    }
}

impl EspIdfDaliTransport {
    fn spawn_sniffer(
        raw: *mut TransportInner,
        stop: Arc<AtomicBool>,
    ) -> Result<std::thread::JoinHandle<()>, EspIdfDaliError> {
        let sniff_ctx = raw as usize;
        match dali2rust_bsp::esp_thread::try_spawn_named_stack(
            c"dali-sniff",
            dali2rust_bsp::std_thread_stack::RING_CONSUMER_STACK,
            move || dali_sniffer_loop(sniff_ctx, stop),
        ) {
            Ok(handle) => Ok(handle),
            Err(_) => {
                // SAFETY: the sniffer failed to spawn and the timer never started, so nothing else holds `raw`.
                unsafe {
                    free_internal(raw);
                }
                Err(EspIdfDaliError::Other("dali sniffer thread spawn failed"))
            }
        }
    }

    fn announce_phy_timing() {
        debug_assert!((TX_HALF_BIT_MIN_US..=TX_HALF_BIT_MAX_US).contains(&TX_HALF_BIT_US));
        debug_assert!((TX_DOUBLE_HALF_BIT_MIN_US..=TX_DOUBLE_HALF_BIT_MAX_US)
            .contains(&TX_DOUBLE_HALF_BIT_US));

        log::info!(
            "DALI PHY (bitbang): GPTimer {} Hz, alarm {} ticks (~{:.1} µs/tick); GPIO TX inverted, RX floating",
            TIMER_RES_HZ,
            TIMER_ALARM_TICKS,
            TIMER_ALARM_TICKS as f64 * 1_000_000f64 / TIMER_RES_HZ as f64
        );

        log::info!(
            "DALI PHY timing: half-bit {} µs (IEC 400..433), double-half-bit {} µs (IEC 800..866)",
            TX_HALF_BIT_US,
            TX_DOUBLE_HALF_BIT_US
        );

        log::info!(
            "DALI sniff: ISR→ring→task (up to {} queued frames); decode logs on success",
            RX_RING_CAP - 1
        );
    }

    fn alloc_transport_inner(
        gpio: RegisterGpio,
        timer: gptimer_handle_t,
        tx: PinDriver<'static, Output>,
        rx: PinDriver<'static, Input>,
    ) -> Result<*mut TransportInner, EspIdfDaliError> {
        // SAFETY: released by `free_internal` exactly once, in `Drop` or on the spawn-failure path.
        unsafe {
            alloc_internal(TransportInner {
                isr: PhyIsrCore::new(gpio),
                timer,
                intr: ptr::null_mut(),
                alarm_isr: PhyAlarmIsr::unarmed(),
                _tx_pin: tx,
                _rx_pin: rx,
                foreign_frames: AtomicU32::new(0),
                last_tx_settle_ticks: AtomicU32::new(TX_SETTLE_UNMEASURED),
                previous_forward: AtomicU32::new(NO_PREVIOUS_FRAME),
                cancel_unacknowledged: AtomicU32::new(0),
                forward_single_gear: AtomicBool::new(true),
                observed_tx: std::sync::OnceLock::new(),
                sniffer_counters: std::sync::OnceLock::new(),
                arbitration_reflex: std::sync::OnceLock::new(),
                wire_counters: std::sync::OnceLock::new(),
            })
        }
    }

    pub fn try_new<Tx, Rx>(tx_pin: Tx, rx_pin: Rx) -> Result<Self, EspIdfDaliError>
    where
        Tx: esp_idf_svc::hal::gpio::OutputPin + 'static,
        Rx: esp_idf_svc::hal::gpio::InputPin + 'static,
    {
        Self::try_new_with_level(tx_pin, rx_pin, PhyIsrLevel::Raw5)
    }

    pub fn try_new_with_level<Tx, Rx>(
        tx_pin: Tx,
        rx_pin: Rx,
        level: PhyIsrLevel,
    ) -> Result<Self, EspIdfDaliError>
    where
        Tx: esp_idf_svc::hal::gpio::OutputPin + 'static,
        Rx: esp_idf_svc::hal::gpio::InputPin + 'static,
    {
        let tx_num = tx_pin.pin() as u8;
        let rx_num = rx_pin.pin() as u8;
        let tx = PinDriver::output(tx_pin)?;
        let rx = PinDriver::input(rx_pin, Pull::Floating)?;
        let (tx_set_reg, tx_clr_reg, tx_mask) = out_regs(tx_num);
        let (rx_in_reg, rx_mask) = in_reg(rx_num);
        // SAFETY: GPIO bank addresses are ESP-IDF constants mapped for life; `TransportInner` keeps the pins.
        let gpio =
            unsafe { RegisterGpio::new(tx_set_reg, tx_clr_reg, rx_in_reg, tx_mask, rx_mask) };

        let timer = new_phy_timer(level)?;

        let raw = Self::alloc_transport_inner(gpio, timer, tx, rx)?;

        // SAFETY: `raw` is valid and the timer not started; `alarm_isr` lives in the same block as the ISR core.
        unsafe {
            arm_late_tick_probe(&(*raw).isr);
            (*raw).alarm_isr = PhyAlarmIsr::new(
                core::ptr::addr_of!((*raw).isr),
                phy_interrupt::timer_alarm_regs(),
            );
        }

        let sniffer_stop = Arc::new(AtomicBool::new(false));
        let sniffer = Self::spawn_sniffer(raw, Arc::clone(&sniffer_stop))?;

        let transport = Self {
            inner: raw,
            sniffer_stop,
            sniffer: Some(sniffer),
        };

        // SAFETY: `raw` is valid and the sniffer already drains the rings; the ISR, which writes them, starts last.
        unsafe { (*raw).intr = start_phy_timer(raw, level)? };

        Self::announce_phy_timing();

        Ok(transport)
    }
}

impl DaliTransport for EspIdfDaliTransport {
    type Error = EspIdfDaliError;

    fn exchange_frame24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        self.exchange_frame24_with_settle(frame, expects_backward, DEFAULT_IDLE_SETTLE_US)
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Frame24Error<Self::Error>> {
        let p = self.inner;
        let hb = encode_forward24_raw(frame[0], frame[1], frame[2]);
        let min_idle_ticks = settle_us_to_idle_ticks(min_idle_us);
        let exchange_id = Self::prepare_session_exchange(p).map_err(Frame24Error::Transport)?;
        let attempt = Self::do_send_once(
            p,
            exchange_id,
            &hb,
            TxFrameLabel::Forward24(frame),
            expects_backward,
            min_idle_ticks,
        )
        .map_err(Frame24Error::Transport)?;
        match attempt {
            TxAttemptOutcome::BusBusy => Ok(TransferOutcome::BusBusy),
            TxAttemptOutcome::Collision => Ok(TransferOutcome::Collision),
            TxAttemptOutcome::Sent { pre_idle_ticks } => {
                Self::record_tx_settle(p, pre_idle_ticks);
                if expects_backward {
                    Self::do_receive_backward_outcome(p, None).map_err(Frame24Error::Transport)
                } else {
                    Ok(TransferOutcome::NoAnswer)
                }
            }
        }
    }

    fn supports_frame24(&self) -> bool {
        true
    }

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        match self.exchange_frame_with_settle(frame, false, DEFAULT_IDLE_SETTLE_US)? {
            TransferOutcome::NoAnswer => Ok(()),
            TransferOutcome::Collision => Err(EspIdfDaliError::Other("DALI TX collision")),
            TransferOutcome::BusBusy => Err(EspIdfDaliError::Other("DALI bus busy")),
            TransferOutcome::ForeignInWindow
            | TransferOutcome::CorruptedInWindow
            | TransferOutcome::Answer(_) => Err(EspIdfDaliError::Other(
                "unexpected non-query exchange outcome",
            )),
        }
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        let p = self.inner;
        let _exchange_id = Self::prepare_session_exchange(p)?;
        let res = match Self::do_receive_backward_outcome(p, None)? {
            TransferOutcome::Answer(backward) => Ok(Some(backward)),
            TransferOutcome::NoAnswer
            | TransferOutcome::ForeignInWindow
            | TransferOutcome::CorruptedInWindow => Ok(None),
            TransferOutcome::Collision | TransferOutcome::BusBusy => {
                Err(EspIdfDaliError::Other("unexpected receive-only outcome"))
            }
        };
        res
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        let idle = unsafe {
            (*self.inner).isr.idle_ticks() >= settle_us_to_idle_ticks(DEFAULT_IDLE_SETTLE_US)
        };
        Ok(idle)
    }

    fn set_observed_frame_sender(&mut self, sender: dali2rust_platform::dali::ObservedFrameSender) {
        // SAFETY: `inner` lives as long as the transport; `OnceLock::set` is task-side and the ISR never reads it.
        let _ = unsafe { (*self.inner).observed_tx.set(sender) };
    }

    fn set_sniffer_counters(&mut self, sink: Arc<dali2rust_platform::dali::PhySnifferCounters>) {
        // SAFETY: same contract as `set_observed_frame_sender` above.
        let _ = unsafe { (*self.inner).sniffer_counters.set(sink) };
    }

    fn set_wire_counters(&mut self, sink: Arc<dali2rust_platform::dali::DaliWireCounters>) {
        // SAFETY: same contract as `set_observed_frame_sender` above.
        let _ = unsafe { (*self.inner).wire_counters.set(sink) };
    }

    fn set_arbitration_reflex(
        &mut self,
        reflex: Arc<dali2rust_platform::arbitration::ArbitrationReflex>,
    ) {
        // SAFETY: same contract as `set_observed_frame_sender` above.
        let _ = unsafe { (*self.inner).arbitration_reflex.set(reflex) };
    }

    fn exchange_frame(
        &mut self,
        frame: u16,
        expects_backward: bool,
    ) -> Result<TransferOutcome, Self::Error> {
        self.exchange_frame_with_settle(frame, expects_backward, DEFAULT_IDLE_SETTLE_US)
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let p = self.inner;
        let hb = encode_forward16_raw(frame);
        let min_idle_ticks = settle_us_to_idle_ticks(min_idle_us);
        let exchange_id = Self::prepare_session_exchange(p)?;
        let outcome = match Self::do_send_once(
            p,
            exchange_id,
            &hb,
            TxFrameLabel::Forward16(frame),
            expects_backward,
            min_idle_ticks,
        )? {
            TxAttemptOutcome::BusBusy => Ok(TransferOutcome::BusBusy),
            TxAttemptOutcome::Collision => Ok(TransferOutcome::Collision),
            TxAttemptOutcome::Sent { pre_idle_ticks } => {
                Self::record_tx_settle(p, pre_idle_ticks);
                if expects_backward {
                    Self::do_receive_backward_outcome(p, Some(frame))
                } else {
                    Ok(TransferOutcome::NoAnswer)
                }
            }
        };
        outcome
    }

    fn foreign_activity(&self) -> u32 {
        unsafe { (*self.inner).foreign_frames.load(Ordering::Acquire) }
    }

    fn last_tx_settle_ticks(&self) -> Option<u16> {
        let raw = unsafe { (*self.inner).last_tx_settle_ticks.load(Ordering::Acquire) };
        u16::try_from(raw).ok()
    }

    fn honours_settle(&self) -> bool {
        true
    }

    fn supports_tx_batching(&self) -> bool {
        true
    }

    fn tx_batch_capacity(&self) -> usize {
        PhyIsrCore::tx_batch_capacity()
    }

    fn exchange_transaction(
        &mut self,
        frames: &[(u16, bool, u32)],
    ) -> Result<BatchOutcome, Self::Error> {
        if frames.len() > PhyIsrCore::tx_batch_capacity() {
            return Err(EspIdfDaliError::Other(
                "DALI transaction exceeds ISR batch capacity",
            ));
        }
        self.send_one_run(self.inner, frames)
    }
}

/// # Safety
/// The returned pointer owns `value` and must be released with [`free_internal`] exactly once.
unsafe fn alloc_internal<T>(value: T) -> Result<*mut T, EspIdfDaliError> {
    use esp_idf_svc::sys::{heap_caps_aligned_alloc, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL};

    let align = core::mem::align_of::<T>().max(4);
    // SAFETY: plain capability-tagged allocation of a valid size/alignment pair.
    let p = unsafe {
        heap_caps_aligned_alloc(
            align,
            core::mem::size_of::<T>(),
            MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT,
        )
    }
    .cast::<T>();
    if p.is_null() {
        return Err(EspIdfDaliError::Other("dali phy state: no internal SRAM"));
    }
    // SAFETY: `p` is a live, uninitialised, unaliased allocation sized and aligned for `T`.
    unsafe { p.write(value) };
    Ok(p)
}

/// # Safety
/// `p` must come from [`alloc_internal`] and must not be used afterwards.
unsafe fn free_internal<T>(p: *mut T) {
    // SAFETY: the caller's contract — `p` is initialised and owned here.
    unsafe {
        core::ptr::drop_in_place(p);
        esp_idf_svc::sys::heap_caps_free(p.cast());
    }
}

impl Drop for EspIdfDaliTransport {
    fn drop(&mut self) {
        self.sniffer_stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.sniffer.take() {
            let _ = h.join();
        }
        if self.inner.is_null() {
            return;
        }
        let p = self.inner;
        self.inner = ptr::null_mut();
        // SAFETY: the sniffer is joined and the timer stopped before the free; `p` came from `alloc_internal`.
        unsafe {
            let timer = (*p).timer;
            if !(*p).intr.is_null() {
                let _ = esp_intr_free((*p).intr);
                phy_interrupt::disable_alarm_interrupt();
            }
            let _ = gptimer_stop(timer);
            let _ = gptimer_disable(timer);
            let _ = gptimer_del_timer(timer);
            free_internal(p);
        }
    }
}

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

use super::{LateReportLine, LATE_REPORT_BYTES};

struct SnifferDiagnostics {
    last_report: std::time::Instant,
    line_held: (u32, u32),
    since_calibration_dump: u32,
    tick_ledger: super::IsrTickLedger,
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
    late_by_task: std::collections::HashMap<LateTaskKey, LateTally>,
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
    fn new() -> Self {
        Self {
            last_report: std::time::Instant::now(),
            line_held: (0, 0),
            since_calibration_dump: 0,
            tick_ledger: super::IsrTickLedger::default(),
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
            late_by_task: std::collections::HashMap::new(),
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

    fn record_dropped(&mut self, dropped: u32) {
        self.dropped = self.dropped.saturating_add(dropped);
    }

    fn record_frame(&mut self, sample_count: u16, decoded: SniffedDecode) {
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

    fn account_isr_ticks(&mut self, now_us: i64, ticks: u32) -> (u32, u32, u32, u32) {
        let (missing, extra, raw_deficit, raw_surplus) = self.tick_ledger.account(now_us, ticks);
        self.isr_ticks_deficit_raw = self.isr_ticks_deficit_raw.saturating_add(raw_deficit);
        self.isr_ticks_surplus_raw = self.isr_ticks_surplus_raw.saturating_add(raw_surplus);
        if missing > 0 {
            self.isr_ticks_lost = self.isr_ticks_lost.saturating_add(missing);
            self.isr_tick_windows_short = self.isr_tick_windows_short.saturating_add(1);
            self.isr_last_short_window = (missing, now_us / 1000);
        }
        if extra > 0 {
            self.isr_ticks_extra = self.isr_ticks_extra.saturating_add(extra);
        }
        (missing, extra, raw_deficit, raw_surplus)
    }

    fn note_stage_delay(&mut self, ticks: u32, budget: u32) {
        self.stage_samples = self.stage_samples.saturating_add(1);
        self.stage_max_ticks = self.stage_max_ticks.max(ticks);
        if ticks > budget {
            self.stage_over_budget = self.stage_over_budget.saturating_add(1);
        }
    }

    fn note_poll(&mut self, now_us: i64, poll_ms: u32) -> Option<i64> {
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

    fn note_late_tick(&mut self, late: dali2rust_dali_phy::LateTick) {
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
        use core::fmt::Write as _;
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

    fn report_timing(&mut self, late_ticks: u32, max_gap_us: u32) {
        let notable = self.stage_over_budget > 0
            || self.poll_late > 0
            || self.isr_tick_windows_short > 0
            || self.isr_ticks_extra > 0
            || self.isr_ticks_raw_reported
                != (self.isr_ticks_deficit_raw, self.isr_ticks_surplus_raw);
        if notable {
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

    fn note_tx_holds(&mut self, yields: u32, voided: u32) {
        self.tx_holds = (yields, voided);
    }

    fn report_line_held(&mut self) {
        let (runs, max_ticks) = self.line_held;
        self.line_held = (0, 0);
        if runs == 0 {
            return;
        }
        log::warn!(
            "DALI line held: {runs} run(s) longer than {} ticks, longest {max_ticks} ticks ({} ms)",
            dali2rust_dali_phy::LINE_HELD_TICKS,
            u32::from(PHY_TICK_US).saturating_mul(max_ticks) / 1000
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
        let (waits, wait_ms, timeouts) = dali2rust_platform::dali::persist_gate_stats();
        log::info!(
            "DALI persist overlap: {during} of {total} commit(s) landed while a frame was on the wire; \
             gate waits={waits} ({wait_ms} ms) timeouts={timeouts}"
        );
    }

    fn report_side_channels(&mut self, scratch: &mut [u8]) {
        self.report_line_held();
        self.report_persist_overlap();
        self.report_tx_holds();
        let (late_ticks, max_gap_us) = self.probe_counts;
        self.report_timing(late_ticks, max_gap_us);
        self.report_late_ticks(scratch);
    }

    fn maybe_report(&mut self, scratch: &mut [u8]) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_report) < Duration::from_millis(SNIFFER_DIAG_INTERVAL_MS) {
            return;
        }
        self.report_side_channels(scratch);

        if self.frames == 0 && self.dropped == 0 {
            self.last_report = now;
            return;
        }

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
        self.start_interval(now);
    }

    fn start_interval(&mut self, now: std::time::Instant) {
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

/// # Safety
/// `inner` must be valid for the transport's lifetime (see `dali_sniffer_loop`).
unsafe fn mirror_isr_timing(
    inner: *mut TransportInner,
    window: (u32, u32),
    probe: (u32, u32),
    raw: (u32, u32),
) {
    use core::sync::atomic::Ordering as AtomOrd;
    // SAFETY: the caller's contract.
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    let (lost, extra) = window;
    if lost > 0 {
        sink.isr_ticks_lost.fetch_add(lost, AtomOrd::Relaxed);
    }
    if extra > 0 {
        sink.isr_ticks_extra.fetch_add(extra, AtomOrd::Relaxed);
    }
    let (deficit, surplus) = raw;
    if deficit > 0 {
        sink.isr_ticks_deficit_raw
            .fetch_add(deficit, AtomOrd::Relaxed);
    }
    if surplus > 0 {
        sink.isr_ticks_surplus_raw
            .fetch_add(surplus, AtomOrd::Relaxed);
    }
    let (late, max_gap) = probe;
    sink.isr_late_ticks.store(late, AtomOrd::Relaxed);
    sink.isr_max_gap_us.fetch_max(max_gap, AtomOrd::Relaxed);
}

/// # Safety
/// `inner` must be valid for the transport's lifetime (see `dali_sniffer_loop`).
unsafe fn mirror_poll_gap(inner: *mut TransportInner, gap_us: Option<i64>, poll_ms: u32) {
    let Some(gap_us) = gap_us else {
        return;
    };
    // SAFETY: the caller's contract.
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    let gap = gap_to_u32(gap_us);
    sink.sniff_poll_gap_max_us.fetch_max(gap, Ordering::Relaxed);
    if dali2rust_platform::dali::sniffer_poll_is_late(gap, poll_ms) {
        sink.sniff_poll_late.fetch_add(1, Ordering::Relaxed);
    }
}

fn gap_to_u32(gap_us: i64) -> u32 {
    u32::try_from(gap_us.max(0)).unwrap_or(u32::MAX)
}

/// # Safety
/// `inner` must be valid for the transport's lifetime (see `dali_sniffer_loop`).
unsafe fn note_answer_staged(
    inner: *mut TransportInner,
    diagnostics: &mut SnifferDiagnostics,
    ticks: u32,
) {
    let budget = u32::from(ANSWER_ARM_TARGET_IDLE_TICKS)
        .saturating_sub(u32::from(RX_IDLE_LINE_HIGH_TICKS));
    diagnostics.note_stage_delay(ticks, budget);
    // SAFETY: the caller's contract.
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    sink.answer_staged.fetch_add(1, Ordering::Relaxed);
    sink.answer_stage_max_ticks.fetch_max(ticks, Ordering::Relaxed);
    if ticks > budget {
        sink.answer_stage_late.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
enum BackwardCause {
    Undecodable,
    FrameSize,
    Incomplete,
    EarlyRejected,
    LateRejected,
}

fn count_backward_cause(inner: *mut TransportInner, cause: BackwardCause) {
    use core::sync::atomic::Ordering as AtomOrd;
    // SAFETY: every caller holds `inner` for the transport lifetime.
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    // SAFETY: same pointer and lifetime; this task wrote the flag before sending the frame.
    let single_gear = unsafe { (*inner).forward_single_gear.load(AtomOrd::Relaxed) };
    let counter = match cause {
        BackwardCause::Undecodable | BackwardCause::FrameSize if !single_gear => {
            &sink.backward_multi_answer
        }
        BackwardCause::Undecodable => &sink.backward_undecodable,
        BackwardCause::FrameSize => &sink.backward_frame_size,
        BackwardCause::Incomplete => &sink.backward_incomplete,
        BackwardCause::EarlyRejected => &sink.backward_early_rejected,
        BackwardCause::LateRejected => &sink.backward_late_rejected,
    };
    counter.fetch_add(1, AtomOrd::Relaxed);
}

fn mirror_sniffer_counters(inner: *mut TransportInner, frame: SniffedFrame) {
    use core::sync::atomic::Ordering as AtomOrd;
    // SAFETY: `inner` is valid for the transport lifetime (see `dali_sniffer_loop`).
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    sink.frames.fetch_add(1, AtomOrd::Relaxed);
    let counter = match frame {
        SniffedFrame::Backward8(_) => &sink.backward8,
        SniffedFrame::Forward16(_) => &sink.forward16,
        SniffedFrame::Forward24(_) => &sink.forward24,
        SniffedFrame::UnsupportedLength(_) => &sink.unsupported_len,
        SniffedFrame::DecodeFailed => &sink.decode_failed,
    };
    counter.fetch_add(1, AtomOrd::Relaxed);
}

fn push_observed_raw_frame(
    inner: *mut TransportInner,
    bytes: [u8; 3],
    kind: dali2rust_platform::dali::ObservedRawFrameKind,
) {
    // SAFETY: `inner` is valid for the transport lifetime (see `dali_sniffer_loop`).
    let Some(tx) = (unsafe { (*inner).observed_tx.get() }) else {
        return;
    };
    let frame = dali2rust_platform::dali::ObservedRawFrame {
        bytes,
        kind,
        observed_at_ms: dali2rust_bsp::unix_clock::unix_wall_clock_millis(),
        observed_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms()
            .wrapping_sub(SNIFFER_POLL_MS),
    };
    if tx.try_send(frame).is_err() {
        log::warn!("DALI sniff: observed-frame channel full; frame dropped");
        // SAFETY: `inner` is valid for the transport lifetime (see above).
        if let Some(sink) = unsafe { (*inner).sniffer_counters.get() } {
            sink.dropped
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// # Safety
/// `inner` must be valid for the transport's lifetime, as it is for the whole sniffer thread.
unsafe fn mirror_bus_failure_counters(inner: *mut TransportInner) {
    let Some(w) = (unsafe { (*inner).wire_counters.get() }) else {
        return;
    };
    let flags = unsafe { (*inner).isr.bus_failure_flags() };
    w.bus_power_down_active.store(
        u32::from(flags & dali2rust_dali_phy::isr::BUS_FAILURE_POWER_DOWN != 0),
        Ordering::Relaxed,
    );
    w.system_failure_active.store(
        u32::from(flags & dali2rust_dali_phy::isr::BUS_FAILURE_SYSTEM != 0),
        Ordering::Relaxed,
    );
    w.bus_power_down_entries.store(
        unsafe { (*inner).isr.bus_power_down_entries() },
        Ordering::Relaxed,
    );
    w.system_failure_entries.store(
        unsafe { (*inner).isr.system_failure_entries() },
        Ordering::Relaxed,
    );
}

/// # Safety
/// `inner` must be valid for the transport's lifetime.
unsafe fn mirror_collision_restarts(inner: *mut TransportInner) {
    // SAFETY: the caller's contract.
    let Some(w) = (unsafe { (*inner).wire_counters.get() }) else {
        return;
    };
    // SAFETY: same.
    w.collision_restarts
        .store(unsafe { (*inner).isr.collision_restarts() }, Ordering::Relaxed);
}

/// # Safety
/// `inner` must be valid for the transport's lifetime.
unsafe fn mirror_arbitration_counters(inner: *mut TransportInner) {
    let Some(reflex) = (unsafe { (*inner).arbitration_reflex.get() }) else {
        return;
    };
    let c = unsafe { (*inner).isr.answer_counts() };
    reflex.mirror_transmit(c.sent, c.stale, c.expired, c.late);
}

unsafe fn mirror_wire_occupancy(
    inner: *mut TransportInner,
    window: &mut dali2rust_platform::dali::WireLoadWindow,
) {
    let Some(w) = (unsafe { (*inner).wire_counters.get() }) else {
        return;
    };
    let (total, active, tx) = unsafe { (*inner).isr.wire_tick_counts() };
    w.wire_ticks_total.store(total, Ordering::Relaxed);
    w.wire_ticks_active.store(active, Ordering::Relaxed);
    w.wire_ticks_tx.store(tx, Ordering::Relaxed);
    if let Some(load) = window.sample(total, active, tx) {
        w.load_permille
            .store(u32::from(load.permille), Ordering::Relaxed);
        w.load_own_permille
            .store(u32::from(load.own_permille), Ordering::Relaxed);
    }
}

/// # Safety
/// `inner` must be valid for the transport's lifetime.
unsafe fn sniffer_poll_ms(inner: *mut TransportInner) -> u32 {
    // SAFETY: see the doc comment.
    let Some(reflex) = (unsafe { (*inner).arbitration_reflex.get() }) else {
        return SNIFFER_POLL_MS;
    };
    if reflex.is_armed(dali2rust_platform::liveness::monotonic_ms()) {
        ARBITRATION_POLL_MS
    } else {
        SNIFFER_POLL_MS
    }
}

const TICK_MS: u32 = 1_000 / esp_idf_svc::sys::configTICK_RATE_HZ;

/// # Safety
/// `inner` must be valid for the transport's lifetime.
unsafe fn note_poll_pace(inner: *mut TransportInner, poll_ms: u32) {
    // SAFETY: see the doc comment.
    let Some(sink) = (unsafe { (*inner).sniffer_counters.get() }) else {
        return;
    };
    if dali2rust_platform::dali::sleep_parks_the_task(poll_ms, TICK_MS) {
        return;
    }
    sink.poll_fast.fetch_add(1, Ordering::Relaxed);
}

/// # Safety
/// `inner` must be valid for the transport's lifetime.
unsafe fn answer_arbitration_query(
    inner: *mut TransportInner,
    frame: [u8; 3],
    rx_epoch: u8,
) -> bool {
    // SAFETY: see the doc comment.
    let Some(reflex) = (unsafe { (*inner).arbitration_reflex.get() }) else {
        return false;
    };
    let now_ms = dali2rust_platform::liveness::monotonic_ms();
    // SAFETY: the ISR core outlives the sniffer thread, which `Drop` joins before freeing the allocation.
    let isr = unsafe { &(*inner).isr };
    super::arbitration_answer::answer_published_frame(isr, reflex, rx_epoch, frame, now_ms)
}

fn dali_sniffer_loop(inner_addr: usize, stop: Arc<AtomicBool>) {
    let inner = inner_addr as *mut TransportInner;
    // SAFETY: `inner` lives as long as `EspIdfDaliTransport`, and `Drop` joins this thread before freeing it.
    let mut diagnostics = Box::new(SnifferDiagnostics::new());
    let mut late_scratch = vec![0u8; LATE_REPORT_BYTES].into_boxed_slice();
    let mut load_window =
        dali2rust_platform::dali::WireLoadWindow::new(super::WIRE_LOAD_WINDOW_TICKS);
    while !stop.load(Ordering::Acquire) {
        // SAFETY: `inner` is valid for the transport lifetime (see above).
        let poll_ms = unsafe { sniffer_poll_ms(inner) };
        // SAFETY: same.
        unsafe { note_poll_pace(inner, poll_ms) };
        // sleep-ok: >= one tick at either pace
        std::thread::sleep(Duration::from_millis(u64::from(poll_ms)));
        if stop.load(Ordering::Acquire) {
            break;
        }
        // SAFETY: `inner` is valid for the transport lifetime (see above).
        unsafe { mirror_isr_state(inner, &mut diagnostics, &mut load_window) };
        // SAFETY: a double read around one clock read; an ISR update across the sample skips this pass.
        let sample = unsafe { coherent_isr_tick_sample(inner) };
        let now_us = sample
            .map(|(clock, _)| clock)
            .unwrap_or_else(|| unsafe { esp_idf_svc::sys::esp_timer_get_time() });
        let gap_us = diagnostics.note_poll(now_us, poll_ms);
        // SAFETY: same.
        unsafe { mirror_poll_gap(inner, gap_us, poll_ms) };
        let window = sample
            .map(|(clock, ticks)| diagnostics.account_isr_ticks(clock, ticks))
            .unwrap_or((0, 0, 0, 0));
        // SAFETY: same.
        diagnostics.probe_counts = unsafe { (*inner).isr.late_tick_counts() };
        // SAFETY: same as above — `inner` is valid for the transport lifetime.
        unsafe {
            mirror_isr_timing(
                inner,
                (window.0, window.1),
                diagnostics.probe_counts,
                (window.2, window.3),
            )
        };
        // SAFETY: same.
        if let Some(late) = unsafe { (*inner).isr.take_late_tick() } {
            diagnostics.note_late_tick(late);
        }
        // SAFETY: same.
        unsafe { process_sniffed_captures(inner, &mut diagnostics) };
        diagnostics.maybe_report(&mut late_scratch);
    }
}

/// # Safety
/// `inner` must remain valid for the transport lifetime.
unsafe fn process_sniffed_captures(
    inner: *mut TransportInner,
    diagnostics: &mut SnifferDiagnostics,
) {
    while let Some(ev) = unsafe { (*inner).isr.pop_rx() } {
        if ev.sample_count < MIN_SAMPLES_FOR_DECODE {
            continue;
        }
        let decoded = ev.decode_sniffed_with_phase();
        diagnostics.record_frame(ev.sample_count, decoded);
        mirror_sniffer_counters(inner, decoded.frame);
        // SAFETY: `inner` remains valid for this transport's lifetime.
        unsafe { dispatch_sniffed_capture(inner, &ev, decoded, diagnostics) };
    }
}

/// # Safety
/// `inner` must remain valid for the transport lifetime.
unsafe fn coherent_isr_tick_sample(inner: *mut TransportInner) -> Option<(i64, u32)> {
    let before = unsafe { (*inner).isr.wire_tick_counts().0 };
    let now_us = unsafe { esp_idf_svc::sys::esp_timer_get_time() };
    let after = unsafe { (*inner).isr.wire_tick_counts().0 };
    (before == after).then_some((now_us, before))
}

fn count_foreign_frame(p: *mut TransportInner) {
    // SAFETY: every caller holds `p` for the transport lifetime.
    unsafe {
        (*p).foreign_frames.fetch_add(1, Ordering::Relaxed);
    }
}

/// # Safety
/// `inner` must be valid for the transport lifetime; the sniffer thread is joined before it is freed.
unsafe fn mirror_isr_state(
    inner: *mut TransportInner,
    diagnostics: &mut SnifferDiagnostics,
    load_window: &mut dali2rust_platform::dali::WireLoadWindow,
) {
    // SAFETY: the caller guarantees `inner` for the transport lifetime.
    unsafe { mirror_bus_failure_counters(inner) };
    // SAFETY: same.
    unsafe { mirror_wire_occupancy(inner, load_window) };
    // SAFETY: same.
    unsafe { mirror_arbitration_counters(inner) };
    // SAFETY: same pointer; read-and-reset, so counts accumulate across the polls of one report window.
    let (runs, max_ticks) = unsafe { (*inner).isr.take_line_held_counts() };
    diagnostics.line_held.0 = diagnostics.line_held.0.saturating_add(runs);
    diagnostics.line_held.1 = diagnostics.line_held.1.max(max_ticks);
    // SAFETY: same.
    unsafe { mirror_collision_restarts(inner) };
    // SAFETY: same.
    let holds = unsafe { ((*inner).isr.tx_yields(), (*inner).isr.tx_voided()) };
    diagnostics.note_tx_holds(holds.0, holds.1);
    // SAFETY: same.
    let dropped = unsafe { (*inner).isr.take_sniff_dropped() };
    if dropped > 0 {
        log::warn!("DALI sniff: {dropped} frame(s) dropped (RX ring full)");
        diagnostics.record_dropped(dropped);
        // SAFETY: same.
        if let Some(sink) = unsafe { (*inner).sniffer_counters.get() } {
            sink.dropped
                .fetch_add(dropped, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// # Safety
/// `inner` must be valid for the transport lifetime.
unsafe fn sniffed_forward24(
    inner: *mut TransportInner,
    b: [u8; 3],
    ev: &RxCompletedEvent,
    diagnostics: &mut SnifferDiagnostics,
) {
    // SAFETY: the caller guarantees `inner`.
    count_foreign_frame(inner);
    // SAFETY: same.
    if unsafe { answer_arbitration_query(inner, b, ev.rx_epoch) } {
        // SAFETY: same.
        let now = unsafe { (*inner).isr.wire_tick_counts().0 };
        // SAFETY: same.
        unsafe { note_answer_staged(inner, diagnostics, now.wrapping_sub(ev.rx_tick)) };
        log::debug!(
            "DALI arbitration: answered 0x{:02x}{:02x}{:02x} (351 §7)",
            b[0], b[1], b[2]
        );
    }
    log::info!(
        "DALI sniff: foreign forward24=0x{:02x}{:02x}{:02x} (addr=0x{:02x})",
        b[0],
        b[1],
        b[2],
        b[0]
    );
    push_observed_raw_frame(
        inner,
        b,
        dali2rust_platform::dali::ObservedRawFrameKind::Forward24,
    );
}

/// # Safety
/// `inner` must be valid for the transport lifetime.
unsafe fn dispatch_sniffed_capture(
    inner: *mut TransportInner,
    ev: &RxCompletedEvent,
    decoded: SniffedDecode,
    diagnostics: &mut SnifferDiagnostics,
) {
    match decoded.frame {
        SniffedFrame::Forward16(v) => {
            // SAFETY: the caller guarantees `inner`.
            count_foreign_frame(inner);
            log_sniffed_forward16(v, decoded.phase_ticks);
            push_observed_raw_frame(
                inner,
                [(v >> 8) as u8, (v & 0xFF) as u8, 0],
                dali2rust_platform::dali::ObservedRawFrameKind::Forward16,
            );
        }
        // SAFETY: same.
        SniffedFrame::Forward24(b) => unsafe { sniffed_forward24(inner, b, ev, diagnostics) },
        SniffedFrame::Backward8(b) => {
            log::info!("DALI sniff: foreign backward8=0x{b:02x}");
            push_observed_raw_frame(
                inner,
                [b, 0, 0],
                dali2rust_platform::dali::ObservedRawFrameKind::Backward8,
            );
        }
        SniffedFrame::UnsupportedLength(bits) => {
            log_sniff_capture(
                ev,
                &decoded,
                format_args!("unsupported decoded length (bits={bits})"),
            );
        }
        SniffedFrame::DecodeFailed => {
            log_sniff_capture(ev, &decoded, "decode failed");
        }
    }
    maybe_dump_calibration_capture(ev, decoded, diagnostics);
}

fn log_sniffed_forward16(raw: u16, phase_ticks: i8) {
    let frame = ForwardFrame(raw);
    let addr = frame.address_byte();
    let data = frame.command_byte();
    let mode = if frame.is_direct_arc_power() {
        "dapc"
    } else {
        "command"
    };
    log_sniffed_forward16_address(raw, mode, addr, data, phase_ticks);
}

fn log_sniffed_forward16_address(raw: u16, mode: &str, addr: u8, data: u8, phase_ticks: i8) {
    match decode_wire_address(addr) {
        Ok(DaliAddress::Short(short)) => {
            log_sniff_forward16_short(raw, mode, short, addr, data, phase_ticks)
        }
        Ok(DaliAddress::Group(group)) => {
            log_sniff_forward16_group(raw, mode, group, addr, data, phase_ticks)
        }
        Ok(DaliAddress::Broadcast) => log_sniff_forward16_line(
            raw,
            mode,
            format_args!("broadcast"),
            addr,
            data,
            phase_ticks,
        ),
        Ok(DaliAddress::BroadcastUnaddressed) => log_sniff_forward16_line(
            raw,
            mode,
            format_args!("broadcast-unaddressed"),
            addr,
            data,
            phase_ticks,
        ),
        Err(_) => log_sniff_forward16_reserved(raw, mode, addr, data, phase_ticks),
    }
}

fn log_sniff_forward16_short(raw: u16, mode: &str, short: u8, addr: u8, data: u8, phase: i8) {
    log_sniff_forward16_line(raw, mode, format_args!("short={short}"), addr, data, phase);
}

fn log_sniff_forward16_group(raw: u16, mode: &str, group: u8, addr: u8, data: u8, phase: i8) {
    log_sniff_forward16_line(raw, mode, format_args!("group={group}"), addr, data, phase);
}

fn log_sniff_forward16_reserved(raw: u16, mode: &str, addr: u8, data: u8, phase: i8) {
    log_sniff_forward16_line(
        raw,
        mode,
        format_args!("reserved addr=0x{addr:02x}"),
        addr,
        data,
        phase,
    );
}

fn log_sniff_forward16_line(
    raw: u16,
    mode: &str,
    addr_label: core::fmt::Arguments<'_>,
    addr: u8,
    data: u8,
    phase_ticks: i8,
) {
    if phase_ticks == 0 {
        log::info!(
            "DALI sniff: foreign forward16=0x{raw:04x} ({mode}, {addr_label}, addr=0x{addr:02x}, data=0x{data:02x})"
        );
    } else {
        log::info!(
            "DALI sniff: foreign forward16=0x{raw:04x} ({mode}, {addr_label}, addr=0x{addr:02x}, data=0x{data:02x}, phase-adjusted, phase_ticks={phase_ticks})"
        );
    }
}

const RANDOM_ADDR_DIAG_MAX_SAMPLES: usize = 32;

fn log_random_address_rx_samples(forward_frame: u16, ev: &RxCompletedEvent, decoded: Option<u8>) {
    let byte_label = match (forward_frame & 0x00FF) as u8 {
        0xC2 => "H",
        0xC3 => "M",
        0xC4 => "L",
        _ => return,
    };
    let n = core::cmp::min(ev.sample_count as usize, RANDOM_ADDR_DIAG_MAX_SAMPLES)
        .min(ev.samples.len());
    let mut hex_buf = [0u8; RANDOM_ADDR_DIAG_MAX_SAMPLES * 3];
    let hex_len = write_hex_bytes(&ev.samples[..n], &mut hex_buf);
    let hex = core::str::from_utf8(&hex_buf[..hex_len]).unwrap_or("");
    log::info!(
        "DALI PHY RX diag: QueryRandomAddress{byte_label} forward16=0x{forward_frame:04x} decoded={decoded:?} sample_count={count} raw=[{hex}]",
        count = ev.sample_count
    );
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

const CALIBRATION_DUMP_EVERY: u32 = 256;

fn maybe_dump_calibration_capture(
    ev: &RxCompletedEvent,
    decoded: SniffedDecode,
    diagnostics: &mut SnifferDiagnostics,
) {
    if !matches!(
        decoded.frame,
        SniffedFrame::Forward16(_) | SniffedFrame::Forward24(_) | SniffedFrame::Backward8(_)
    ) {
        return;
    }
    diagnostics.since_calibration_dump = diagnostics.since_calibration_dump.saturating_add(1);
    if diagnostics.since_calibration_dump < CALIBRATION_DUMP_EVERY {
        return;
    }
    diagnostics.since_calibration_dump = 0;
    log_sniff_capture(ev, &decoded, "calibration (this one DECODED)");
}

fn log_sniff_capture(
    ev: &RxCompletedEvent,
    decoded: &SniffedDecode,
    detail: impl core::fmt::Display,
) {
    let n = core::cmp::min(usize::from(ev.sample_count), RxCompletedEvent::MAX_SAMPLES);
    let mut hex_buf = [0u8; RxCompletedEvent::MAX_SAMPLES * 3];
    let hex_len = write_hex_bytes(&ev.samples[..n], &mut hex_buf);
    let hex = core::str::from_utf8(&hex_buf[..hex_len]).unwrap_or("");
    log::warn!(
        "DALI sniff: {detail}: samples={} pre_idle={} phase={} score={} capture=[{hex}]",
        ev.sample_count,
        ev.pre_idle_ticks,
        decoded.phase_ticks,
        decoded.score
    );
}

fn write_hex_bytes(bytes: &[u8], out: &mut [u8]) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut pos = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 {
            if pos >= out.len() {
                break;
            }
            out[pos] = b' ';
            pos += 1;
        }
        if pos + 2 > out.len() {
            break;
        }
        out[pos] = HEX[(b >> 4) as usize];
        out[pos + 1] = HEX[(b & 0x0F) as usize];
        pos += 2;
    }
    pos
}
