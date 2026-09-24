use esp_idf_svc::sys::{
    esp_intr_alloc, esp_intr_enable, esp_intr_free, esp_intr_get_cpu, esp_intr_get_intno,
    esp_rom_delay_us, esprv_int_get_priority, gptimer_alarm_config_t, gptimer_enable,
    gptimer_handle_t, gptimer_set_alarm_action, gptimer_start, intr_handle_t, intr_handler_set,
    periph_interrupt_t_ETS_TG0_T0_INTR_SOURCE, EspError, DR_REG_TIMERGROUP0_BASE,
    ESP_INTR_FLAG_INTRDISABLED, ESP_INTR_FLAG_IRAM, ESP_INTR_FLAG_LEVEL4, ESP_INTR_FLAG_LEVEL5,
};

const _: () = assert!(ESP_INTR_FLAG_LEVEL4 == 1 << 4 && ESP_INTR_FLAG_LEVEL5 == 1 << 5);

use dali2rust_dali_phy::timer_alarm::{
    dali_phy_raw_isr, TimerAlarmRegs, TIMG_ALARM_EN_MASK, TIMG_AUTORELOAD_MASK,
    TIMG_INT_ENA_OFFSET, TIMG_INT_RAW_OFFSET, TIMG_T0_MASK,
};
use dali2rust_dali_phy::{PhyIsrCore, PHY_TICK_US};

use super::esp_idf::EspIdfDaliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhyIsrLevel {
    Driver3,
    Raw4,
    Raw5,
}

impl PhyIsrLevel {
    pub fn from_knob(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") | Some("5") => Self::Raw5,
            Some("4") => Self::Raw4,
            Some("3") => Self::Driver3,
            Some(other) => {
                log::warn!("DALI2RUST_PHY_ISR_LEVEL={other:?} is not 3, 4 or 5 — using 5");
                Self::Raw5
            }
        }
    }

    pub fn level(self) -> i32 {
        match self {
            Self::Driver3 => 3,
            Self::Raw4 => 4,
            Self::Raw5 => 5,
        }
    }

    pub fn driver_priority(self) -> i32 {
        if self == Self::Driver3 {
            self.level()
        } else {
            0
        }
    }

    fn intr_flag(self) -> u32 {
        1u32 << self.level()
    }
}

const PHY_TIMG_BASE: usize = DR_REG_TIMERGROUP0_BASE as usize;
const PHY_TIMG_TIMER: u8 = 0;

pub fn timer_alarm_regs() -> TimerAlarmRegs {
    // SAFETY: ESP-IDF's base constant for timer group 0 on this chip, timer 0, mapped for the program's life.
    unsafe { TimerAlarmRegs::for_timg(PHY_TIMG_BASE, PHY_TIMG_TIMER) }
}

/// # Safety
/// `timer` is a live handle from `gptimer_new_timer`.
pub unsafe fn set_phy_alarm(timer: gptimer_handle_t, alarm_ticks: u64) -> Result<(), EspError> {
    // SAFETY: plain FFI; `zeroed` gives the driver's defaults for `flags`.
    unsafe {
        let mut alarm: gptimer_alarm_config_t = core::mem::zeroed();
        alarm.alarm_count = alarm_ticks;
        alarm.reload_count = 0;
        alarm.flags.set_auto_reload_on_alarm(1);
        EspError::convert(gptimer_set_alarm_action(timer, &alarm))
    }
}

/// # Safety
/// `timer` live and unarmed; `arg` an internal-RAM `PhyAlarmIsr` outliving the handle; run on CPU0, rings drained.
pub unsafe fn start_raw(
    timer: gptimer_handle_t,
    arg: *mut core::ffi::c_void,
    isr: &PhyIsrCore,
    level: PhyIsrLevel,
    alarm_ticks: u64,
) -> Result<intr_handle_t, EspIdfDaliError> {
    // SAFETY: the caller's contract for each step.
    unsafe {
        set_phy_alarm(timer, alarm_ticks)?;
        check_alarm_armed()?;
        let handle = alloc_interrupt(level)?;
        let started = install_and_start(handle, timer, arg, level)
            .and_then(|()| verify_ticking(isr, handle, level));
        if let Err(e) = started {
            let _ = esp_intr_free(handle);
            disable_alarm_interrupt();
            return Err(e);
        }
        Ok(handle)
    }
}

/// # Safety
/// Reads a mapped peripheral register.
unsafe fn check_alarm_armed() -> Result<(), EspIdfDaliError> {
    // SAFETY: timer 0's CONFIG register of group 0, mapped for life.
    let config = unsafe { core::ptr::read_volatile(PHY_TIMG_BASE as *const u32) };
    let want = TIMG_ALARM_EN_MASK | TIMG_AUTORELOAD_MASK;
    if config & want != want {
        log::error!("DALI PHY: TG0 T0 CONFIG=0x{config:08x} lacks alarm/auto-reload — the driver picked another timer");
        return Err(EspIdfDaliError::Other("dali phy: gptimer is not TG0 T0"));
    }
    Ok(())
}

/// # Safety
/// FFI; runs on the core the interrupt will serve.
unsafe fn alloc_interrupt(level: PhyIsrLevel) -> Result<intr_handle_t, EspIdfDaliError> {
    let flags = level.intr_flag() | ESP_INTR_FLAG_IRAM | ESP_INTR_FLAG_INTRDISABLED;
    let mut handle: intr_handle_t = core::ptr::null_mut();
    // SAFETY: plain FFI; a NULL handler and argument are what a level above 3 requires.
    EspError::convert(unsafe {
        esp_intr_alloc(
            periph_interrupt_t_ETS_TG0_T0_INTR_SOURCE as i32,
            flags as i32,
            None,
            core::ptr::null_mut(),
            &mut handle,
        )
    })?;
    Ok(handle)
}

/// # Safety
/// See [`start_raw`].
unsafe fn install_and_start(
    handle: intr_handle_t,
    timer: gptimer_handle_t,
    arg: *mut core::ffi::c_void,
    level: PhyIsrLevel,
) -> Result<(), EspIdfDaliError> {
    // SAFETY: the caller's contract.
    unsafe {
        let intno = esp_intr_get_intno(handle);
        let cpu = esp_intr_get_cpu(handle);
        let here = esp_idf_svc::hal::cpu::core() as i32;
        if cpu != here {
            log::error!("DALI PHY: interrupt allocated on core {cpu}, installing from core {here}");
            return Err(EspIdfDaliError::Other("dali phy: interrupt core mismatch"));
        }
        intr_handler_set(intno, Some(dali_phy_raw_isr), arg);
        let got = esprv_int_get_priority(intno);
        if got != level.level() {
            log::error!("DALI PHY: cpu int {intno} reads level {got}, asked for {}", level.level());
            return Err(EspIdfDaliError::Other("dali phy: interrupt level not applied"));
        }
        enable_alarm_interrupt();
        EspError::convert(gptimer_enable(timer))?;
        EspError::convert(gptimer_start(timer))?;
        prove_ack_path()?;
        EspError::convert(esp_intr_enable(handle))?;
        log::info!(
            "DALI PHY interrupt: level {}, cpu int {intno}, core {cpu}, source TG0_T0, raw handler",
            level.level()
        );
    }
    Ok(())
}

const PROVE_ALARM_WAIT_US: u32 = 10 * PHY_TICK_US;

/// # Safety
/// Mapped peripheral registers; the timer runs and nothing else acknowledges its alarm yet.
unsafe fn prove_ack_path() -> Result<(), EspIdfDaliError> {
    let regs = timer_alarm_regs();
    for _ in 0..2 {
        // SAFETY: the caller's contract.
        unsafe {
            wait_for_alarm()?;
            regs.ack_and_rearm();
        }
        if int_raw() & TIMG_T0_MASK != 0 {
            log::error!("DALI PHY: TG0 INT_RAW=0x{:08x} still set after the ISR's clear", int_raw());
            return Err(EspIdfDaliError::Other("dali phy: INT_CLR does not clear the alarm"));
        }
    }
    Ok(())
}

/// # Safety
/// Mapped peripheral register.
unsafe fn wait_for_alarm() -> Result<(), EspIdfDaliError> {
    for _ in 0..PROVE_ALARM_WAIT_US {
        if int_raw() & TIMG_T0_MASK != 0 {
            return Ok(());
        }
        // busy-wait-ok: boot-only, bounded to ten 104 µs periods.
        // SAFETY: ROM delay, no preconditions.
        unsafe { esp_rom_delay_us(1) };
    }
    log::error!("DALI PHY: no alarm in TG0 INT_RAW within {PROVE_ALARM_WAIT_US} us — ALARM_EN not re-armed?");
    Err(EspIdfDaliError::Other("dali phy: the alarm does not re-arm"))
}

fn int_raw() -> u32 {
    let raw = PHY_TIMG_BASE.wrapping_add(TIMG_INT_RAW_OFFSET) as *const u32;
    // SAFETY: timer group 0's `INT_RAW`, mapped for the life of the program.
    unsafe { core::ptr::read_volatile(raw) }
}

/// # Safety
/// Mapped peripheral register; called once, at composition.
unsafe fn enable_alarm_interrupt() {
    let ena = PHY_TIMG_BASE.wrapping_add(TIMG_INT_ENA_OFFSET) as *mut u32;
    // SAFETY: see the function docs.
    unsafe { core::ptr::write_volatile(ena, core::ptr::read_volatile(ena) | TIMG_T0_MASK) };
}

/// # Safety
/// As [`enable_alarm_interrupt`]; called after `esp_intr_free`.
pub unsafe fn disable_alarm_interrupt() {
    let ena = PHY_TIMG_BASE.wrapping_add(TIMG_INT_ENA_OFFSET) as *mut u32;
    // SAFETY: see `enable_alarm_interrupt`.
    unsafe { core::ptr::write_volatile(ena, core::ptr::read_volatile(ena) & !TIMG_T0_MASK) };
}

const VERIFY_WAIT_MS: u64 = 5;

/// # Safety
/// `handle` is the live allocation from [`alloc_interrupt`].
unsafe fn verify_ticking(
    isr: &PhyIsrCore,
    handle: intr_handle_t,
    level: PhyIsrLevel,
) -> Result<(), EspIdfDaliError> {
    let before = isr.wire_tick_counts().0;
    // sleep-ok: boot only, >= one FreeRTOS tick
    std::thread::sleep(core::time::Duration::from_millis(VERIFY_WAIT_MS));
    let ticked = isr.wire_tick_counts().0.wrapping_sub(before);
    let expected_min = (VERIFY_WAIT_MS as u32 * 1000 / PHY_TICK_US) / 2;
    if ticked >= expected_min {
        return Ok(());
    }
    let raw_bits = int_raw();
    // SAFETY: `handle` is live.
    let intno = unsafe { esp_intr_get_intno(handle) };
    log::error!(
        "DALI PHY: level {} interrupt (cpu int {intno}) ticked {ticked} times in {VERIFY_WAIT_MS} ms, \
         want >= {expected_min}; TG0 INT_RAW=0x{raw_bits:08x}",
        level.level()
    );
    Err(EspIdfDaliError::Other("dali phy: timer interrupt does not tick"))
}
