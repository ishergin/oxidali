pub const QUERY_OPCODES: &[u8] = &[
    0x90,
    0x91,
    0x92,
    0x93,
    0x94,
    0x95,
    0x96,
    0x97,
    0x98,
    0x99,
    0x9A,
    0x9B,
    0x9C,
    0x9D,
    0x9E,
    0x9F,
    0xA0,
    0xA1,
    0xA2,
    0xA3,
    0xA4,
    0xA5,
    0xA6,
    0xA7,
    0xA8,
    0xAA,
    0xC2,
    0xC3,
    0xC4,
];

#[inline]
pub const fn is_query_opcode(opcode: u8) -> bool {
    if (opcode >= 0xB0 && opcode <= 0xBF) || opcode == 0xC0 || opcode == 0xC1 {
        return true;
    }

    let mut i = 0;
    while i < QUERY_OPCODES.len() {
        if QUERY_OPCODES[i] == opcode {
            return true;
        }
        i += 1;
    }
    false
}

#[inline]
pub const fn is_config_opcode_range(opcode: u8) -> bool {
    opcode >= 0x80 && opcode <= 0x8F
}

pub const DAPC_LEVEL_MIN: u8 = 0x00;
pub const DAPC_LEVEL_MAX: u8 = 0xFE;
pub const DAPC_LEVEL_MASK: u8 = 0xFF;

pub const ARC_POWER_CONTROL_START: u8 = 0x00;
pub const ARC_POWER_CONTROL_END: u8 = 0x0F;

pub const GO_TO_SCENE_BASE: u8 = 0x10;

pub const CONFIG_START: u8 = 0x20;
pub const CONFIG_END: u8 = 0x7F;
pub const CONFIG_RESET: u8 = 0x20;
pub const CONFIG_SET_MAX_LEVEL: u8 = 0x2A;
pub const CONFIG_SET_MIN_LEVEL: u8 = 0x2B;
pub const CONFIG_SET_SYSTEM_FAILURE_LEVEL: u8 = 0x2C;
pub const CONFIG_SET_POWER_ON_LEVEL: u8 = 0x2D;
pub const CONFIG_SET_FADE_TIME: u8 = 0x2E;
pub const CONFIG_SET_FADE_RATE: u8 = 0x2F;
pub const CONFIG_SET_EXTENDED_FADE_TIME: u8 = 0x30;
pub const CONFIG_SET_SCENE_BASE: u8 = 0x40;
pub const CONFIG_REMOVE_SCENE_BASE: u8 = 0x50;
pub const CONFIG_ADD_TO_GROUP_BASE: u8 = 0x60;
pub const CONFIG_REMOVE_FROM_GROUP_BASE: u8 = 0x70;

pub const CONFIG_EXTENDED_START: u8 = 0x80;
pub const CONFIG_EXTENDED_END: u8 = 0x87;
pub const CONFIG_SET_SHORT_ADDRESS: u8 = 0x80;
pub const CONFIG_ENABLE_WRITE_MEMORY: u8 = 0x81;

pub const QUERY_START: u8 = 0x90;

pub const QUERY_SCENE_BASE: u8 = 0xB0;

pub const QUERY_GROUPS_0_7: u8 = 0xC0;
pub const QUERY_GROUPS_8_15: u8 = 0xC1;

pub const READ_MEMORY_LOCATION_OPCODE: u8 = 0xC5;

pub const EXTENDED_START: u8 = 0xE0;
pub const EXTENDED_END: u8 = 0xFF;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_query_opcodes_are_unique() {
        let mut seen = [false; 256];
        for &op in QUERY_OPCODES {
            assert!(!seen[op as usize], "duplicate opcode 0x{op:02X}");
            seen[op as usize] = true;
        }
    }

    #[test]
    fn is_query_opcode_matches_array() {
        for &op in QUERY_OPCODES {
            assert!(is_query_opcode(op), "0x{op:02X} should be a query opcode");
        }
        assert!(!is_query_opcode(0x00));
        assert!(!is_query_opcode(0x20));
        assert!(!is_query_opcode(0x40));
        assert!(!is_query_opcode(0xFF));
    }

    #[test]
    fn query_scene_and_group_ranges_expect_backward() {
        assert!(is_query_opcode(0xB0));
        assert!(is_query_opcode(0xBF));
        assert!(is_query_opcode(0xC0));
        assert!(is_query_opcode(0xC1));
        assert!(is_query_opcode(0xC2));
        assert!(is_query_opcode(0xC3));
        assert!(is_query_opcode(0xC4));
        assert!(!is_query_opcode(0xC5));
    }

    #[test]
    fn config_and_query_helpers_are_distinct() {
        assert!(is_config_opcode_range(0x80));
        assert!(is_config_opcode_range(0x8F));
        assert!(!is_config_opcode_range(0x90));
        assert!(!is_config_opcode_range(0x9F));
        assert!(is_query_opcode(0x90));
        assert!(is_query_opcode(0x9F));
        assert!(!is_config_opcode_range(0xA0));
        assert!(!is_config_opcode_range(0x00));
    }
}
