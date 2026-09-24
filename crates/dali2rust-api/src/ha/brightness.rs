pub const HA_BRIGHTNESS_SCALE: u8 = dali2rust_domain::dali::device::MAX_ARC_POWER_LEVEL;

pub fn level_to_ha(level: u8) -> u8 {
    level.min(HA_BRIGHTNESS_SCALE)
}

pub fn ha_to_level(brightness: u8) -> u8 {
    brightness.min(HA_BRIGHTNESS_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dali_level_survives_a_round_trip_through_home_assistant() {
        for level in 0..=HA_BRIGHTNESS_SCALE {
            assert_eq!(ha_to_level(level_to_ha(level)), level, "level {level}");
        }
    }

    #[test]
    fn nothing_from_the_wire_can_exceed_the_dali_maximum() {
        for brightness in 0..=u8::MAX {
            assert!(ha_to_level(brightness) <= HA_BRIGHTNESS_SCALE);
        }
        assert_eq!(ha_to_level(255), 254);
    }
}
