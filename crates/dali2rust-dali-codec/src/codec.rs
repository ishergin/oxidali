pub use dali2rust_dali_phy::HalfBitBuffer;

pub fn encode_to_half_bits(data: u32, bit_count: u8) -> HalfBitBuffer {
    HalfBitBuffer::encode(data, bit_count)
}

#[inline]
pub fn encode_forward16_raw(frame: u16) -> HalfBitBuffer {
    encode_to_half_bits(frame as u32, 16)
}

#[inline]
pub fn encode_forward24_raw(addr: u8, opcode: u8, data: u8) -> HalfBitBuffer {
    let raw: u32 = ((addr as u32) << 16) | ((opcode as u32) << 8) | (data as u32);
    encode_to_half_bits(raw, 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward16_halfbit_len_reasonable() {
        let b = encode_forward16_raw(0x0300);
        assert!(b.length > 32 && b.length <= HalfBitBuffer::MAX_DATA_HALF_BITS);
    }

    #[test]
    fn forward24_halfbit_len_reasonable() {
        let b = encode_forward24_raw(0xA1, 0x02, 0x03);
        assert!(b.length > 48 && b.length <= HalfBitBuffer::MAX_DATA_HALF_BITS);
    }

    #[test]
    fn encode_24bit_roundtrip_via_generic() {
        let a = encode_forward24_raw(0xA1, 0x02, 0x03);
        let b = encode_to_half_bits(0xA10203, 24);
        assert_eq!(a.length, b.length);
        assert_eq!(&a.data[..], &b.data[..]);
    }
}

pub fn merge_dominant(bytes: &[u8], bit_count: u8) -> Option<HalfBitBuffer> {
    let (&first, rest) = bytes.split_first()?;
    let mut merged = encode_to_half_bits(u32::from(first), bit_count);
    for &byte in rest {
        let other = encode_to_half_bits(u32::from(byte), bit_count);
        for (slot, extra) in merged.data.iter_mut().zip(other.data.iter()) {
            *slot |= *extra;
        }
    }
    Some(merged)
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    const BACKWARD_BITS: u8 = 8;

    #[test]
    fn a_single_answer_is_just_that_answer() {
        let merged = merge_dominant(&[0x5A], BACKWARD_BITS).expect("an answer");
        let plain = encode_to_half_bits(0x5A, BACKWARD_BITS);
        assert_eq!(merged.length, plain.length);
        assert_eq!(merged.data, plain.data);
    }

    #[test]
    fn identical_answers_stay_readable() {
        let merged = merge_dominant(&[0xFF, 0xFF, 0xFF], BACKWARD_BITS).expect("an answer");
        let plain = encode_to_half_bits(0xFF, BACKWARD_BITS);
        assert_eq!(
            merged.data, plain.data,
            "aligned identical answers merge to that answer — an emulator property (one clock), not a claim about a real segment; see `merge_dominant`"
        );
    }

    #[test]
    fn differing_answers_produce_neither_of_them() {
        let merged = merge_dominant(&[0x00, 0xFF], BACKWARD_BITS).expect("an answer");
        assert_ne!(merged.data, encode_to_half_bits(0x00, BACKWARD_BITS).data);
        assert_ne!(merged.data, encode_to_half_bits(0xFF, BACKWARD_BITS).data);
    }

    fn ones(buffer: &HalfBitBuffer) -> u32 {
        buffer.data.iter().map(|b| b.count_ones()).sum()
    }

    #[test]
    fn merging_only_ever_adds_dominant_half_bits() {
        let a = encode_to_half_bits(0x0F, BACKWARD_BITS);
        let b = encode_to_half_bits(0xF0, BACKWARD_BITS);
        let merged = merge_dominant(&[0x0F, 0xF0], BACKWARD_BITS).expect("an answer");
        assert!(ones(&merged) >= ones(&a));
        assert!(ones(&merged) >= ones(&b));
    }

    #[test]
    fn no_answers_means_no_frame() {
        assert!(merge_dominant(&[], BACKWARD_BITS).is_none());
    }
}

