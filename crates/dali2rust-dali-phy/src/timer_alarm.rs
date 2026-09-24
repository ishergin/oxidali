use crate::isr::PhyIsrCore;

pub const TIMG_TX_CONFIG_STRIDE: usize = 0x24;
pub const TIMG_INT_ENA_OFFSET: usize = 0x70;
pub const TIMG_INT_RAW_OFFSET: usize = 0x74;
pub const TIMG_INT_CLR_OFFSET: usize = 0x7C;
pub const TIMG_T0_MASK: u32 = 0x0000_0001;
pub const TIMG_T1_MASK: u32 = 0x0000_0002;
pub const TIMG_ALARM_EN_MASK: u32 = 0x0000_0400;
pub const TIMG_AUTORELOAD_MASK: u32 = 0x2000_0000;
pub const TIMG_EN_MASK: u32 = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerAlarmRegs {
    int_clr: *mut u32,
    config: *mut u32,
    int_mask: u32,
    alarm_en_mask: u32,
}

impl TimerAlarmRegs {
    /// # Safety
    /// `group_base` is a timer group's register base on this chip, and `timer` is 0 or 1.
    pub const unsafe fn for_timg(group_base: usize, timer: u8) -> Self {
        let (stride, int_mask) = if timer == 0 {
            (0, TIMG_T0_MASK)
        } else {
            (TIMG_TX_CONFIG_STRIDE, TIMG_T1_MASK)
        };
        Self {
            int_clr: group_base.wrapping_add(TIMG_INT_CLR_OFFSET) as *mut u32,
            config: group_base.wrapping_add(stride) as *mut u32,
            int_mask,
            alarm_en_mask: TIMG_ALARM_EN_MASK,
        }
    }

    /// # Safety
    /// Both pointers stay valid for volatile reads and writes for as long as the value is used.
    pub const unsafe fn new(
        int_clr: *mut u32,
        config: *mut u32,
        int_mask: u32,
        alarm_en_mask: u32,
    ) -> Self {
        Self {
            int_clr,
            config,
            int_mask,
            alarm_en_mask,
        }
    }

    pub const fn null() -> Self {
        Self {
            int_clr: core::ptr::null_mut(),
            config: core::ptr::null_mut(),
            int_mask: 0,
            alarm_en_mask: 0,
        }
    }

    /// # Safety
    /// Registers from `for_timg` or `new`; nothing else writes `CONFIG` while the interrupt is enabled.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub unsafe fn ack_and_rearm(&self) {
        // SAFETY: the caller's contract; INT_CLR is write-1-to-clear, so only this timer's bit is affected.
        unsafe {
            core::ptr::write_volatile(self.int_clr, self.int_mask);
            let config = core::ptr::read_volatile(self.config);
            core::ptr::write_volatile(self.config, config | self.alarm_en_mask);
        }
    }
}

pub struct PhyAlarmIsr {
    core: *const PhyIsrCore,
    regs: TimerAlarmRegs,
}

// SAFETY: only the interrupt dereferences the pointer; `PhyIsrCore` is shared through atomics and SPSC cells.
unsafe impl Sync for PhyAlarmIsr {}
// SAFETY: as above — moving the value moves two addresses and two masks.
unsafe impl Send for PhyAlarmIsr {}

impl PhyAlarmIsr {
    pub const fn unarmed() -> Self {
        Self {
            core: core::ptr::null(),
            regs: TimerAlarmRegs::null(),
        }
    }

    /// # Safety
    /// `core` outlives the interrupt this value is registered with; `regs` meet `ack_and_rearm`'s contract.
    pub const unsafe fn new(core: *const PhyIsrCore, regs: TimerAlarmRegs) -> Self {
        Self { core, regs }
    }

    /// # Safety
    /// Called only from the alarm interrupt, on a value built by `new`.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub unsafe fn service(&self) {
        // SAFETY: the caller's contract, and `new`'s.
        unsafe {
            self.regs.ack_and_rearm();
            (*self.core).tick();
        }
    }
}

/// # Safety
/// `arg` is the `PhyAlarmIsr` passed to `intr_handler_set`; it is freed only after `esp_intr_free`.
#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub unsafe extern "C" fn dali_phy_raw_isr(arg: *mut core::ffi::c_void) {
    // SAFETY: `arg` is the registered `PhyAlarmIsr`, alive until `esp_intr_free`.
    unsafe { (*arg.cast::<PhyAlarmIsr>()).service() };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpio::RegisterGpio;

    #[test]
    fn for_timg_derives_the_header_addresses() {
        // SAFETY: the value is compared, never dereferenced.
        let t0 = unsafe { TimerAlarmRegs::for_timg(0x500C_2000, 0) };
        // SAFETY: same.
        let expected0 = unsafe {
            TimerAlarmRegs::new(0x500C_207C as *mut u32, 0x500C_2000 as *mut u32, 1, 0x400)
        };
        assert_eq!(t0, expected0);
        // SAFETY: same.
        let t1 = unsafe { TimerAlarmRegs::for_timg(0x500C_2000, 1) };
        // SAFETY: same.
        let expected1 = unsafe {
            TimerAlarmRegs::new(0x500C_207C as *mut u32, 0x500C_2024 as *mut u32, 2, 0x400)
        };
        assert_eq!(t1, expected1);
    }

    #[test]
    fn ack_writes_only_the_clear_mask_and_rearm_sets_only_alarm_en() {
        let mut clr = 0u32;
        let mut config = TIMG_EN_MASK | 0x4000_0000 | TIMG_AUTORELOAD_MASK;
        // SAFETY: two live locals.
        let regs =
            unsafe { TimerAlarmRegs::new(&raw mut clr, &raw mut config, 1, TIMG_ALARM_EN_MASK) };
        // SAFETY: same.
        unsafe { regs.ack_and_rearm() };
        assert_eq!(clr, 1);
        assert_eq!(config, 0xE000_0400);
    }

    #[test]
    fn the_raw_trampoline_acks_rearms_and_ticks_once() {
        let (mut set, mut clr_gpio) = (0u32, 0u32);
        let input = 0b0010u32;
        // SAFETY: scratch "registers" that outlive the core.
        let gpio =
            unsafe { RegisterGpio::new(&raw mut set, &raw mut clr_gpio, &raw const input, 1, 2) };
        let core = PhyIsrCore::new(gpio);
        let (mut int_clr, mut config) = (0u32, 0u32);
        // SAFETY: two live locals.
        let regs = unsafe { TimerAlarmRegs::new(&raw mut int_clr, &raw mut config, 1, 0x400) };
        // SAFETY: `core` outlives `arg`.
        let mut arg = unsafe { PhyAlarmIsr::new(&raw const core, regs) };
        let before = core.wire_tick_counts().0;
        // SAFETY: single-threaded test standing in for the interrupt.
        unsafe { dali_phy_raw_isr((&raw mut arg).cast()) };
        assert_eq!(core.wire_tick_counts().0.wrapping_sub(before), 1);
        assert_eq!(int_clr, 1);
        assert_eq!(config, TIMG_ALARM_EN_MASK);
    }
}
