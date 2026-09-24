pub const DALI_TX_GPIO: u8 = 14;

pub const DALI_RX_GPIO: u8 = 17;

pub const DALI_RX_HV_TAP_GPIO: u8 = 18;

pub const I2C_SDA_GPIO: u8 = 33;
pub const I2C_SCL_GPIO: u8 = 32;

pub const ETH_MDC_GPIO: u8 = 31;
pub const ETH_MDIO_GPIO: u8 = 52;
pub const ETH_REF_CLK_GPIO: u8 = 50;
pub const ETH_PHY_RESET_GPIO: u8 = 51;

pub const ETH_TX_EN_GPIO: u8 = 49;
pub const ETH_TXD0_GPIO: u8 = 34;
pub const ETH_TXD1_GPIO: u8 = 35;
pub const ETH_CRS_DV_GPIO: u8 = 28;
pub const ETH_RXD0_GPIO: u8 = 29;
pub const ETH_RXD1_GPIO: u8 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dali_pins_match_hat_header_positions() {
        assert_eq!(DALI_TX_GPIO, 14, "header pin 9 / Pico GP6");
        assert_eq!(DALI_RX_GPIO, 17, "header pin 5 / Pico GP3");
        assert_eq!(DALI_RX_HV_TAP_GPIO, 18, "header pin 4 / Pico GP2");
    }

    #[test]
    fn i2c_pins() {
        assert_eq!(I2C_SDA_GPIO, 33);
        assert_eq!(I2C_SCL_GPIO, 32);
    }

    #[test]
    fn assigned_pins_avoid_onboard_peripherals() {
        const ETHERNET: &[u8] = &[28, 29, 30, 31, 34, 49, 50, 51, 52];
        const CONSOLE_UART: &[u8] = &[37, 38];
        const MICROSD: &[u8] = &[39, 40, 41, 42, 43, 44];
        const CODEC_I2S: &[u8] = &[9, 10, 11, 12, 13];
        const SHARED_I2C: &[u8] = &[7, 8];
        const USB_JTAG: &[u8] = &[24, 25];

        for pin in [
            DALI_TX_GPIO,
            DALI_RX_GPIO,
            DALI_RX_HV_TAP_GPIO,
            I2C_SDA_GPIO,
            I2C_SCL_GPIO,
        ] {
            for (name, group) in [
                ("ethernet", ETHERNET),
                ("console uart", CONSOLE_UART),
                ("microsd", MICROSD),
                ("codec i2s", CODEC_I2S),
                ("shared i2c", SHARED_I2C),
                ("usb-jtag", USB_JTAG),
            ] {
                assert!(
                    !group.contains(&pin),
                    "GPIO{pin} collides with the on-board {name}"
                );
            }
        }
    }

    #[test]
    fn assigned_pins_are_not_strapping_pins() {
        const STRAPPING: &[u8] = &[34, 35, 36, 37, 38];
        for pin in [DALI_TX_GPIO, DALI_RX_GPIO, I2C_SDA_GPIO, I2C_SCL_GPIO] {
            assert!(!STRAPPING.contains(&pin), "GPIO{pin} is a strapping pin");
        }
    }

    #[test]
    fn ethernet_pins_do_not_reach_the_pico_header() {
        const HEADER_GPIOS: &[u8] = &[
            54, 19, 18, 17, 16, 15, 14, 6, 5, 4, 3, 2, 8, 7, 48, 47, 46, 33, 32, 27, 26, 23, 22,
            21, 20,
        ];
        for pin in [
            ETH_MDC_GPIO,
            ETH_MDIO_GPIO,
            ETH_REF_CLK_GPIO,
            ETH_PHY_RESET_GPIO,
            ETH_TX_EN_GPIO,
            ETH_TXD0_GPIO,
            ETH_TXD1_GPIO,
            ETH_CRS_DV_GPIO,
            ETH_RXD0_GPIO,
            ETH_RXD1_GPIO,
        ] {
            assert!(
                !HEADER_GPIOS.contains(&pin),
                "GPIO{pin} is an Ethernet signal AND is broken out to the header"
            );
        }
    }

    #[test]
    fn ethernet_pins_are_distinct() {
        let mut pins = [
            ETH_MDC_GPIO,
            ETH_MDIO_GPIO,
            ETH_REF_CLK_GPIO,
            ETH_PHY_RESET_GPIO,
            ETH_TX_EN_GPIO,
            ETH_TXD0_GPIO,
            ETH_TXD1_GPIO,
            ETH_CRS_DV_GPIO,
            ETH_RXD0_GPIO,
            ETH_RXD1_GPIO,
        ];
        pins.sort_unstable();
        let before = pins.len();
        let mut dedup = pins.to_vec();
        dedup.dedup();
        assert_eq!(dedup.len(), before, "duplicate Ethernet pin assignment");
    }
}
