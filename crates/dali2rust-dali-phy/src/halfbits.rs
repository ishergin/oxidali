const START_PAIR: u8 = 0b10;
const ONE_PAIR: u8 = 0b10;
const ZERO_PAIR: u8 = 0b01;
const STOP_PAIR: u8 = 0b00;
const PAIR_TOP_SHIFT: u8 = 6;

#[derive(Clone, Debug, Default)]
pub struct HalfBitBuffer {
    pub data: [u8; Self::DATA_LEN],
    pub length: u8,
}

impl HalfBitBuffer {
    pub const MAX_DATA_HALF_BITS: u8 = 72;
    pub const DATA_LEN: usize = 9;

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn encode(data: u32, bit_count: u8) -> Self {
        let mut out = Self { data: [0; Self::DATA_LEN], length: 0 };
        out.push_pair(START_PAIR);
        let mut remaining = bit_count;
        while remaining > 0 {
            remaining = remaining.wrapping_sub(1);
            let one = data.checked_shr(u32::from(remaining)).unwrap_or(0) & 1 != 0;
            out.push_pair(if one { ONE_PAIR } else { ZERO_PAIR });
        }
        out.push_pair(STOP_PAIR);
        out.push_pair(STOP_PAIR);
        out
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn push_pair(&mut self, pair: u8) {
        let length = self.length;
        if let Some(byte) = self.data.get_mut(usize::from(length >> 3)) {
            let shift = PAIR_TOP_SHIFT.wrapping_sub(length & 7);
            *byte |= pair.checked_shl(u32::from(shift)).unwrap_or(0);
        }
        self.length = length.saturating_add(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backward_byte_is_a_start_pair_eight_bit_pairs_and_two_stop_pairs() {
        let frame = HalfBitBuffer::encode(0b1010_0001, 8);
        assert_eq!(frame.length, 22);
        assert_eq!(&frame.data[..3], &[0b1010_0110, 0b0101_0101, 0b1000_0000]);
    }
}
