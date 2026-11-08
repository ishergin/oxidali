pub const CMD_DISPLAY_OFF: u8 = 0xAE;
pub const CMD_DISPLAY_ON: u8 = 0xAF;
const CMD_SET_DISPLAY_CLOCK_DIV: u8 = 0xD5;
const CMD_SET_MULTIPLEX: u8 = 0xA8;
const CMD_SET_DISPLAY_OFFSET: u8 = 0xD3;
const CMD_SET_START_LINE: u8 = 0x40;
const CMD_SET_CHARGE_PUMP: u8 = 0x8D;
const CMD_MEMORY_MODE: u8 = 0x20;
pub const CMD_SEG_REMAP_REVERSED: u8 = 0xA1;
pub const CMD_SEG_REMAP_NORMAL: u8 = 0xA0;
pub const CMD_COM_SCAN_DEC: u8 = 0xC8;
pub const CMD_COM_SCAN_INC: u8 = 0xC0;
const CMD_SET_COM_PINS: u8 = 0xDA;
const CMD_SET_CONTRAST: u8 = 0x81;
const CMD_SET_PRECHARGE: u8 = 0xD9;
const CMD_SET_VCOM_DETECT: u8 = 0xDB;
const CMD_DISPLAY_ALL_ON_RESUME: u8 = 0xA4;
const CMD_NORMAL_DISPLAY: u8 = 0xA6;

const CLOCK_DIV_DEFAULT: u8 = 0x80;
const MULTIPLEX_64_ROWS: u8 = 0x3F;
const NO_OFFSET: u8 = 0x00;
const CHARGE_PUMP_ON: u8 = 0x14;
const HORIZONTAL_ADDRESSING: u8 = 0x00;
const COM_PINS_ALTERNATIVE: u8 = 0x12;
const CONTRAST: u8 = 0xCF;
const PRECHARGE: u8 = 0xF1;
const VCOM_DESELECT: u8 = 0x40;

pub const INIT_LEN: usize = 24;

pub fn init_sequence(upside_down: bool) -> [u8; INIT_LEN] {
    let (seg_remap, com_scan) = if upside_down {
        (CMD_SEG_REMAP_NORMAL, CMD_COM_SCAN_INC)
    } else {
        (CMD_SEG_REMAP_REVERSED, CMD_COM_SCAN_DEC)
    };
    [
        CMD_DISPLAY_OFF,
        CMD_SET_DISPLAY_CLOCK_DIV,
        CLOCK_DIV_DEFAULT,
        CMD_SET_MULTIPLEX,
        MULTIPLEX_64_ROWS,
        CMD_SET_DISPLAY_OFFSET,
        NO_OFFSET,
        CMD_SET_START_LINE,
        CMD_SET_CHARGE_PUMP,
        CHARGE_PUMP_ON,
        CMD_MEMORY_MODE,
        HORIZONTAL_ADDRESSING,
        seg_remap,
        com_scan,
        CMD_SET_COM_PINS,
        COM_PINS_ALTERNATIVE,
        CMD_SET_CONTRAST,
        CONTRAST,
        CMD_SET_PRECHARGE,
        PRECHARGE,
        CMD_SET_VCOM_DETECT,
        VCOM_DESELECT,
        CMD_DISPLAY_ALL_ON_RESUME,
        CMD_NORMAL_DISPLAY,
    ]
}

pub const fn power_command(on: bool) -> u8 {
    if on {
        CMD_DISPLAY_ON
    } else {
        CMD_DISPLAY_OFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upright_mounting_mirrors_columns_and_scans_rows_from_the_bottom() {
        let init = init_sequence(false);
        assert!(init.contains(&CMD_SEG_REMAP_REVERSED) && init.contains(&CMD_COM_SCAN_DEC));
        assert!(!init.contains(&CMD_SEG_REMAP_NORMAL) && !init.contains(&CMD_COM_SCAN_INC));
    }

    #[test]
    fn half_a_turn_flips_both_the_column_and_the_row_direction() {
        let upright = init_sequence(false);
        let flipped = init_sequence(true);
        let differing: Vec<(u8, u8)> =
            upright.iter().zip(&flipped).filter(|(a, b)| a != b).map(|(a, b)| (*a, *b)).collect();
        assert_eq!(
            differing,
            vec![(CMD_SEG_REMAP_REVERSED, CMD_SEG_REMAP_NORMAL), (CMD_COM_SCAN_DEC, CMD_COM_SCAN_INC)]
        );
    }

    #[test]
    fn the_init_sequence_leaves_the_panel_dark_until_it_is_powered() {
        let init = init_sequence(false);
        assert_eq!(init[0], CMD_DISPLAY_OFF);
        assert!(!init.contains(&CMD_DISPLAY_ON));
        assert_eq!(power_command(true), CMD_DISPLAY_ON);
        assert_eq!(power_command(false), CMD_DISPLAY_OFF);
    }
}
