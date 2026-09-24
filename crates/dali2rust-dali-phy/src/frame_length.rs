pub const MIN_SAMPLES_FOR_DECODE: u16 = 8;
pub const BACKWARD8_SAMPLE_COUNT: u16 = 11;
pub const FORWARD16_SAMPLE_COUNT: u16 = 19;
pub const FORWARD24_SAMPLE_COUNT: u16 = 27;
pub const FORWARD_LENGTH_MIN_SAMPLES: u16 = FORWARD16_SAMPLE_COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameLengthClass {
    Backward,
    Forward,
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
// IEC 62386-101 §7.4.4
pub const fn frame_length_class(sample_count: u16) -> FrameLengthClass {
    if sample_count >= FORWARD_LENGTH_MIN_SAMPLES {
        FrameLengthClass::Forward
    } else {
        FrameLengthClass::Backward
    }
}

const _: () = assert!(MIN_SAMPLES_FOR_DECODE < BACKWARD8_SAMPLE_COUNT);
const _: () = assert!(BACKWARD8_SAMPLE_COUNT < FORWARD_LENGTH_MIN_SAMPLES);
const _: () = assert!(FORWARD16_SAMPLE_COUNT < FORWARD24_SAMPLE_COUNT);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_length_class_bands_pin_their_boundaries() {
        assert_eq!(frame_length_class(MIN_SAMPLES_FOR_DECODE), FrameLengthClass::Backward);
        assert_eq!(frame_length_class(BACKWARD8_SAMPLE_COUNT), FrameLengthClass::Backward);
        assert_eq!(frame_length_class(18), FrameLengthClass::Backward);
        assert_eq!(frame_length_class(19), FrameLengthClass::Forward);
        assert_eq!(frame_length_class(FORWARD16_SAMPLE_COUNT), FrameLengthClass::Forward);
        assert_eq!(frame_length_class(FORWARD24_SAMPLE_COUNT), FrameLengthClass::Forward);
        assert_eq!(frame_length_class(u16::MAX), FrameLengthClass::Forward);
    }
}
