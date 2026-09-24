pub const MISSED_FRAMES_WORD: usize = 8;

const NO_DESCRIPTOR_MASK: u32 = 0xFFFF;
const NO_DESCRIPTOR_ROLLOVER: u32 = 1 << 16;
const NO_DESCRIPTOR_SPAN: u32 = 1 << 16;
const FIFO_SHIFT: u32 = 17;
const FIFO_MASK: u32 = 0x7FF;
const FIFO_ROLLOVER: u32 = 1 << 28;
const FIFO_SPAN: u32 = 1 << 11;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MissedFrames {
    pub no_descriptor: u32,
    pub fifo_overflow: u32,
}

pub fn decode_missed_frames(raw: u32) -> MissedFrames {
    let rolled = |bit: u32, span: u32| if raw & bit == 0 { 0 } else { span };
    MissedFrames {
        no_descriptor: (raw & NO_DESCRIPTOR_MASK) + rolled(NO_DESCRIPTOR_ROLLOVER, NO_DESCRIPTOR_SPAN),
        fifo_overflow: ((raw >> FIFO_SHIFT) & FIFO_MASK) + rolled(FIFO_ROLLOVER, FIFO_SPAN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_register_reports_nothing_missed() {
        assert_eq!(decode_missed_frames(0), MissedFrames::default());
    }

    #[test]
    fn the_two_counters_come_from_their_own_fields() {
        let raw = 37 | (5 << FIFO_SHIFT);
        assert_eq!(
            decode_missed_frames(raw),
            MissedFrames { no_descriptor: 37, fifo_overflow: 5 }
        );
    }

    #[test]
    fn a_rollover_bit_adds_the_full_span_of_its_counter() {
        let raw = NO_DESCRIPTOR_ROLLOVER | 3 | FIFO_ROLLOVER | (2 << FIFO_SHIFT);
        assert_eq!(
            decode_missed_frames(raw),
            MissedFrames { no_descriptor: 65_539, fifo_overflow: 2_050 }
        );
    }

    #[test]
    fn the_reserved_top_bits_count_for_nothing() {
        assert_eq!(decode_missed_frames(0xE000_0000), MissedFrames::default());
    }
}
