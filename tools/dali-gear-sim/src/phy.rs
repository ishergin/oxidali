use core::ptr;

use dali2rust_dali_phy::{
    dali_phy_alarm_isr, PhyIsrCore, RegisterGpio, RxCompletedEvent, SessionEvent, TxGates,
    PHY_TICK_US,
};
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys::{
    esp_rom_delay_us, gptimer_alarm_cb_t, gptimer_alarm_config_t, gptimer_config_t,
    gptimer_count_direction_t_GPTIMER_COUNT_UP, gptimer_enable, gptimer_event_callbacks_t,
    gptimer_handle_t, gptimer_new_timer, gptimer_register_event_callbacks,
    gptimer_set_alarm_action, gptimer_start, soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT,
    EspError, GPIO_IN_REG, GPIO_OUT_W1TC_REG, GPIO_OUT_W1TS_REG,
};

pub const DALI_TX_GPIO: u8 = 14;
pub const DALI_RX_GPIO: u8 = 5;

const TIMER_RES_HZ: u32 = 1_000_000;
const TIMER_ALARM_TICKS: u64 = PHY_TICK_US as u64;

const _: () = assert!(DALI_TX_GPIO < 32 && DALI_RX_GPIO < 32);

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
unsafe impl Send for GearPhy {}

impl GearPhy {
    pub fn start(peripherals: Peripherals) -> Result<&'static Self, EspError> {
        let pins = peripherals.pins;
        let tx = PinDriver::output(pins.gpio14)?;
        let rx = PinDriver::input(pins.gpio5, Pull::Floating)?;

        let (tx_set, tx_clr, tx_mask) = out_regs(DALI_TX_GPIO);
        let (rx_in, rx_mask) = in_reg(DALI_RX_GPIO);
        // SAFETY: ESP-IDF's GPIO register constants for this chip, mapped for the program's life; pads set up above.
        let gpio = unsafe { RegisterGpio::new(tx_set, tx_clr, rx_in, tx_mask, rx_mask) };

        // SAFETY: the FSM is not running yet, so nothing else writes this pad.
        unsafe { ptr::write_volatile(tx_clr, tx_mask) };

        let probe = probe_rx(rx_in, rx_mask, PROBE_MS);

        let isr: &'static PhyIsrCore = Box::leak(Box::new(PhyIsrCore::new(gpio)));
        let timer = new_phy_timer()?;
        // SAFETY: `isr` is leaked, so it outlives the timer; nothing else touches the FSM once the interrupt starts.
        unsafe { start_phy_timer(timer, isr)? };

        let phy: &'static GearPhy = Box::leak(Box::new(GearPhy {
            isr,
            probe,
            _tx_pin: tx,
            _rx_pin: rx,
            _timer: timer,
        }));
        Ok(phy)
    }

    pub fn pop_rx(&self) -> Option<RxCompletedEvent> {
        self.isr.pop_rx()
    }

    pub fn idle_ticks(&self) -> u32 {
        self.isr.idle_ticks()
    }

    pub fn submit_backward(&self, data: &[u8; 9], len: u8) -> bool {
        self.isr.submit_tx(data, len, false, TxGates::settle(0))
    }

    pub fn probe(&self) -> (u32, bool) {
        self.probe
    }

    pub fn take_sniff_dropped(&self) -> u32 {
        self.isr.take_sniff_dropped()
    }

    pub fn pop_session_event(&self) -> Option<SessionEvent> {
        self.isr.pop_session_event()
    }
}

fn new_phy_timer() -> Result<gptimer_handle_t, EspError> {
    // SAFETY: plain FFI construction; `zeroed` gives the driver's documented defaults for `flags` and priority.
    unsafe {
        let mut cfg: gptimer_config_t = core::mem::zeroed();
        cfg.clk_src = soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT;
        cfg.direction = gptimer_count_direction_t_GPTIMER_COUNT_UP;
        cfg.resolution_hz = TIMER_RES_HZ;
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
    Ok(())
}

const PROBE_MS: u32 = 300;

fn probe_rx(rx_in: *const u32, rx_mask: u32, ms: u32) -> (u32, bool) {
    let mut edges = 0u32;
    // SAFETY: a plain read of a mapped register the pad is already configured for.
    let mut last = unsafe { ptr::read_volatile(rx_in) } & rx_mask != 0;
    let start_level = last;
    for _ in 0..(ms * 1000 / PROBE_SAMPLE_US) {
        // SAFETY: as above.
        let now = unsafe { ptr::read_volatile(rx_in) } & rx_mask != 0;
        if now != last {
            edges += 1;
            last = now;
        }
        // SAFETY: ROM delay, no state.
        unsafe { esp_rom_delay_us(PROBE_SAMPLE_US) };
    }
    (edges, start_level)
}

const PROBE_SAMPLE_US: u32 = 20;

pub fn isr_core() -> Option<u32> {
    dali2rust_dali_phy::isr_core_id()
}
