use core::ptr;

use dali2rust_dali_phy::{
    dali_phy_alarm_isr, AnswerCounts, HalfBitBuffer, PhyIsrCore, RegisterGpio, RxCompletedEvent,
    PHY_TICK_US,
};
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys::{
    esp_rom_delay_us, esp_timer_get_time, gptimer_alarm_cb_t, gptimer_alarm_config_t,
    gptimer_config_t, gptimer_count_direction_t_GPTIMER_COUNT_UP, gptimer_enable,
    gptimer_event_callbacks_t, gptimer_handle_t, gptimer_new_timer,
    gptimer_register_event_callbacks, gptimer_set_alarm_action, gptimer_start,
    heap_caps_aligned_alloc, soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT, EspError,
    ESP_ERR_NO_MEM, GPIO_IN_REG, GPIO_OUT_W1TC_REG, GPIO_OUT_W1TS_REG, MALLOC_CAP_8BIT,
    MALLOC_CAP_INTERNAL,
};

pub const DALI_TX_GPIO: u8 = 14;
pub const DALI_RX_GPIO: u8 = 17;

pub const PHY_INTERRUPT_PRIORITY: i32 = 3;

const TIMER_RES_HZ: u32 = 1_000_000;
const TIMER_ALARM_TICKS: u64 = PHY_TICK_US as u64;

const PROBE_MS: u32 = 300;
const PROBE_SAMPLE_US: u32 = 20;
const US_PER_MS: u32 = 1_000;

const _: () = assert!(DALI_TX_GPIO < 32 && DALI_RX_GPIO < 32);

unsafe extern "C" {
    static pxCurrentTCBs: [*mut core::ffi::c_void; 2];
    static port_uxInterruptNesting: [u32; 2];
}

fn out_regs(pin: u8) -> (*mut u32, *mut u32, u32) {
    (
        GPIO_OUT_W1TS_REG as *mut u32,
        GPIO_OUT_W1TC_REG as *mut u32,
        1u32 << pin,
    )
}

fn in_reg(pin: u8) -> (*const u32, u32) {
    (GPIO_IN_REG as *const u32, 1u32 << pin)
}

pub struct GearPhy {
    isr: &'static PhyIsrCore,
    probe: (u32, bool),
    _tx_pin: PinDriver<'static, Output>,
    _rx_pin: PinDriver<'static, Input>,
    _timer: gptimer_handle_t,
}

// SAFETY: fields are `Sync` (`PhyIsrCore`'s rings are SPSC atomics) or never touched after construction.
unsafe impl Sync for GearPhy {}
// SAFETY: as above; the pin drivers and the timer handle only keep the peripherals claimed.
unsafe impl Send for GearPhy {}

impl GearPhy {
    pub fn start(peripherals: Peripherals) -> Result<&'static Self, EspError> {
        let pins = peripherals.pins;
        let tx = PinDriver::output(pins.gpio14)?;
        let rx = PinDriver::input(pins.gpio17, Pull::Floating)?;

        let (tx_set, tx_clr, tx_mask) = out_regs(DALI_TX_GPIO);
        let (rx_in, rx_mask) = in_reg(DALI_RX_GPIO);
        // SAFETY: ESP-IDF's GPIO register constants for this chip, mapped for the program's life; pads set up above.
        let gpio = unsafe { RegisterGpio::new(tx_set, tx_clr, rx_in, tx_mask, rx_mask) };

        // SAFETY: the FSM is not running yet, so nothing else writes this pad.
        unsafe { ptr::write_volatile(tx_clr, tx_mask) };

        let probe = probe_rx(rx_in, rx_mask, PROBE_MS);

        let isr = alloc_internal(PhyIsrCore::new(gpio))?;
        // SAFETY: the timer below is not running yet, so the interrupt cannot race the probe's installation.
        unsafe { arm_late_tick_probe(isr) };
        let timer = new_phy_timer()?;
        // SAFETY: `isr` is never freed, so it outlives the timer; nothing else touches the FSM once the interrupt starts.
        unsafe { start_phy_timer(timer, isr)? };

        // SAFETY: one PHY per boot; the pins, the timer and the interrupt's context live for the program.
        Ok(Box::leak(Box::new(GearPhy {
            isr,
            probe,
            _tx_pin: tx,
            _rx_pin: rx,
            _timer: timer,
        })))
    }

    pub fn pop_rx(&self) -> Option<RxCompletedEvent> {
        self.isr.pop_rx()
    }

    pub fn submit_answer(&self, rx_epoch: u8, buffer: &HalfBitBuffer) -> bool {
        self.isr.submit_answer(rx_epoch, buffer)
    }

    pub fn answer_counts(&self) -> AnswerCounts {
        self.isr.answer_counts()
    }

    pub fn last_answer_arm_idle_ticks(&self) -> u32 {
        self.isr.last_answer_arm_idle_ticks()
    }

    pub fn late_tick_counts(&self) -> (u32, u32) {
        self.isr.late_tick_counts()
    }

    pub fn probe(&self) -> (u32, bool) {
        self.probe
    }

    pub fn take_sniff_dropped(&self) -> u32 {
        self.isr.take_sniff_dropped()
    }
}

fn alloc_internal(core: PhyIsrCore) -> Result<&'static PhyIsrCore, EspError> {
    let align = core::mem::align_of::<PhyIsrCore>().max(4);
    // SAFETY: plain capability-tagged allocation of a valid size/alignment pair.
    let p = unsafe {
        heap_caps_aligned_alloc(
            align,
            core::mem::size_of::<PhyIsrCore>(),
            MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT,
        )
    }
    .cast::<PhyIsrCore>();
    if p.is_null() {
        return Err(EspError::from_infallible::<ESP_ERR_NO_MEM>());
    }
    // SAFETY: `p` is a live, unaliased allocation sized and aligned for `PhyIsrCore`, and it is never freed.
    unsafe {
        p.write(core);
        Ok(&*p)
    }
}

/// # Safety
/// The GPTimer driving `isr` must not be running yet.
unsafe fn arm_late_tick_probe(isr: &PhyIsrCore) {
    // SAFETY: the kernel's and the port's per-core tables are process-lifetime globals, used only as addresses.
    unsafe {
        isr.install_tick_probe(
            esp_timer_get_time,
            core::ptr::addr_of!(pxCurrentTCBs[0]).cast::<usize>(),
            core::ptr::addr_of!(pxCurrentTCBs[1]).cast::<usize>(),
            core::ptr::addr_of!(port_uxInterruptNesting[0]),
            core::ptr::addr_of!(port_uxInterruptNesting[1]),
        );
    }
}

fn new_phy_timer() -> Result<gptimer_handle_t, EspError> {
    // SAFETY: plain FFI construction; `zeroed` gives the driver's documented defaults for `flags`.
    unsafe {
        let mut cfg: gptimer_config_t = core::mem::zeroed();
        cfg.clk_src = soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT;
        cfg.direction = gptimer_count_direction_t_GPTIMER_COUNT_UP;
        cfg.resolution_hz = TIMER_RES_HZ;
        cfg.intr_priority = PHY_INTERRUPT_PRIORITY;
        let mut handle: gptimer_handle_t = ptr::null_mut();
        EspError::convert(gptimer_new_timer(&cfg, &mut handle))?;
        Ok(handle)
    }
}

/// # Safety
/// `isr` must outlive the timer and its rings must have no other consumer yet.
unsafe fn start_phy_timer(timer: gptimer_handle_t, isr: &'static PhyIsrCore) -> Result<(), EspError> {
    const _: () = assert!(
        core::mem::size_of::<gptimer_handle_t>() == core::mem::size_of::<*mut core::ffi::c_void>()
    );
    // SAFETY: fn-pointer cast between `extern "C"` signatures that are ABI-identical, as asserted above.
    let on_alarm: gptimer_alarm_cb_t = Some(unsafe {
        core::mem::transmute::<
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
        >(dali_phy_alarm_isr)
    });
    let cbs = gptimer_event_callbacks_t { on_alarm };
    // SAFETY: the callback table and the context outlive the timer; the timer is created and not yet enabled.
    unsafe {
        EspError::convert(gptimer_register_event_callbacks(
            timer,
            &cbs,
            isr as *const PhyIsrCore as *mut core::ffi::c_void,
        ))?;
        let mut alarm: gptimer_alarm_config_t = core::mem::zeroed();
        alarm.alarm_count = TIMER_ALARM_TICKS;
        alarm.reload_count = 0;
        alarm.flags.set_auto_reload_on_alarm(1);
        EspError::convert(gptimer_set_alarm_action(timer, &alarm))?;
        EspError::convert(gptimer_enable(timer))?;
        EspError::convert(gptimer_start(timer))?;
    }
    Ok(())
}

fn probe_rx(rx_in: *const u32, rx_mask: u32, ms: u32) -> (u32, bool) {
    let mut edges = 0u32;
    // SAFETY: a plain read of a mapped register the pad is already configured for.
    let mut last = unsafe { ptr::read_volatile(rx_in) } & rx_mask != 0;
    let start_level = last;
    for _ in 0..(ms * US_PER_MS / PROBE_SAMPLE_US) {
        // SAFETY: as above.
        let now = unsafe { ptr::read_volatile(rx_in) } & rx_mask != 0;
        if now != last {
            edges += 1;
            last = now;
        }
        // busy-wait-ok: a boot-time edge count on a pad the interrupt does not own yet, sampled faster than a tick.
        // SAFETY: ROM delay, no state.
        unsafe { esp_rom_delay_us(PROBE_SAMPLE_US) };
    }
    (edges, start_level)
}

pub fn isr_core() -> Option<u32> {
    dali2rust_dali_phy::isr_core_id()
}
