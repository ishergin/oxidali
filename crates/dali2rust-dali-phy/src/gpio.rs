use dali2rust_platform::hal::BitbangHal;

pub struct RegisterGpio {
    tx_set_reg: *mut u32,
    tx_clr_reg: *mut u32,
    rx_in_reg: *const u32,
    tx_mask: u32,
    rx_mask: u32,
}

impl RegisterGpio {
    /// # Safety
    /// The addresses are live GPIO registers; the masks select a TX output and an RX input that nothing else drives.
    pub const unsafe fn new(
        tx_set_reg: *mut u32,
        tx_clr_reg: *mut u32,
        rx_in_reg: *const u32,
        tx_mask: u32,
        rx_mask: u32,
    ) -> Self {
        Self {
            tx_set_reg,
            tx_clr_reg,
            rx_in_reg,
            tx_mask,
            rx_mask,
        }
    }
}

impl BitbangHal for RegisterGpio {
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn bus_is_high(&mut self) -> bool {
        // SAFETY: a read-only memory-mapped input register whose validity `new`'s caller guarantees.
        unsafe { (core::ptr::read_volatile(self.rx_in_reg) & self.rx_mask) != 0 }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn bus_set_low(&mut self) {
        // SAFETY: write-1-to-set register; only our bit is affected.
        unsafe { core::ptr::write_volatile(self.tx_set_reg, self.tx_mask) }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn bus_set_high(&mut self) {
        // SAFETY: write-1-to-clear register; only our bit is affected.
        unsafe { core::ptr::write_volatile(self.tx_clr_reg, self.tx_mask) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_hit_the_right_register_and_only_their_own_bit() {
        let mut set: u32 = 0;
        let mut clr: u32 = 0;
        let input: u32 = 0b1010;

        // SAFETY: the three "registers" are live locals for the whole test.
        let mut gpio = unsafe {
            RegisterGpio::new(&raw mut set, &raw mut clr, &raw const input, 0b0100, 0b0010)
        };

        gpio.bus_set_low();
        assert_eq!(set, 0b0100, "dominant writes the set register");
        assert_eq!(clr, 0, "dominant leaves the clear register alone");

        set = 0;
        gpio.bus_set_high();
        assert_eq!(clr, 0b0100, "recessive writes the clear register");
        assert_eq!(set, 0, "recessive leaves the set register alone");

        assert!(gpio.bus_is_high(), "rx bit set reads recessive");

        let low_input: u32 = 0b1000;
        // SAFETY: same as above.
        let mut gpio_low = unsafe {
            RegisterGpio::new(
                &raw mut set,
                &raw mut clr,
                &raw const low_input,
                0b0100,
                0b0010,
            )
        };
        assert!(!gpio_low.bus_is_high(), "rx bit clear reads dominant");
    }
}
