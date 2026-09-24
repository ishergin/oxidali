use dali2rust_contracts::msg::StatusFlags;

// IEC 62386-102 Table 12
pub const STATUS_GEAR_FAILURE: u8 = 0x01;
pub const STATUS_LAMP_FAILURE: u8 = 0x02;
pub const STATUS_LAMP_ON: u8 = 0x04;
pub const STATUS_LIMIT_ERROR: u8 = 0x08;
pub const STATUS_FADE_RUNNING: u8 = 0x10;
pub const STATUS_RESET_STATE: u8 = 0x20;
pub const STATUS_MISSING_SHORT_ADDRESS: u8 = 0x40;
pub const STATUS_POWER_CYCLE_SEEN: u8 = 0x80;

#[must_use]
pub const fn decode(raw: u8) -> StatusFlags {
    StatusFlags {
        raw,
        gear_failure: raw & STATUS_GEAR_FAILURE != 0,
        lamp_failure: raw & STATUS_LAMP_FAILURE != 0,
        lamp_on: raw & STATUS_LAMP_ON != 0,
        limit_error: raw & STATUS_LIMIT_ERROR != 0,
        fade_running: raw & STATUS_FADE_RUNNING != 0,
        reset_state: raw & STATUS_RESET_STATE != 0,
        missing_short_address: raw & STATUS_MISSING_SHORT_ADDRESS != 0,
        power_cycle_seen: raw & STATUS_POWER_CYCLE_SEEN != 0,
    }
}

#[must_use]
pub const fn encode(f: &StatusFlags) -> u8 {
    let mut raw = 0u8;
    if f.gear_failure {
        raw |= STATUS_GEAR_FAILURE;
    }
    if f.lamp_failure {
        raw |= STATUS_LAMP_FAILURE;
    }
    if f.lamp_on {
        raw |= STATUS_LAMP_ON;
    }
    if f.limit_error {
        raw |= STATUS_LIMIT_ERROR;
    }
    if f.fade_running {
        raw |= STATUS_FADE_RUNNING;
    }
    if f.reset_state {
        raw |= STATUS_RESET_STATE;
    }
    if f.missing_short_address {
        raw |= STATUS_MISSING_SHORT_ADDRESS;
    }
    if f.power_cycle_seen {
        raw |= STATUS_POWER_CYCLE_SEEN;
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_and_encode_round_trip_over_every_byte() {
        for raw in 0..=u8::MAX {
            let flags = decode(raw);
            assert_eq!(flags.raw, raw);
            assert_eq!(encode(&flags), raw, "0x{raw:02X} did not survive the pair");
        }
    }

    #[test]
    fn the_bits_are_iec_62386_102_table_12() {
        let all = decode(0xFF);
        assert!(all.gear_failure && all.power_cycle_seen);
        assert!(decode(0b0000_0001).gear_failure);
        assert!(decode(0b0000_0010).lamp_failure);
        assert!(decode(0b0000_0100).lamp_on);
        assert!(decode(0b0000_1000).limit_error);
        assert!(decode(0b0001_0000).fade_running);
        assert!(decode(0b0010_0000).reset_state);
        assert!(decode(0b0100_0000).missing_short_address);
        assert!(decode(0b1000_0000).power_cycle_seen);
    }

    #[test]
    fn lamp_failure_and_gear_failure_are_independent() {
        let lamp_only = decode(STATUS_LAMP_FAILURE);
        assert!(lamp_only.lamp_failure && !lamp_only.gear_failure);
        let gear_only = decode(STATUS_GEAR_FAILURE);
        assert!(gear_only.gear_failure && !gear_only.lamp_failure);
    }
}
