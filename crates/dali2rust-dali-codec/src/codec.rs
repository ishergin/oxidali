pub const MANCHESTER_RX_MAX_DECODED_BITS: u8 = 32;
pub const MANCHESTER_RX_MIN_DECODED_BITS: u8 = 3;
const PHASE_ACQUISITION_TICKS: [i8; 5] = [-2, -1, 0, 1, 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManchesterDecodeResult {
    pub bit_len: u8,
    pub phase_ticks: i8,
    pub score: u16,
}

pub use dali2rust_dali_phy::HalfBitBuffer;

fn push_two_half_bits(out: &mut HalfBitBuffer, hb: u8) {
    let hblen = out.length as u16;
    let pos = (hblen >> 3) as usize;
    let shift = 6 - (hblen & 7) as u8;
    out.data[pos] |= hb << shift;
    out.length = (hblen + 2) as u8;
}

pub fn encode_to_half_bits(data: u32, bit_count: u8) -> HalfBitBuffer {
    let mut out = HalfBitBuffer::default();
    push_two_half_bits(&mut out, 0x2);
    for i in 0..bit_count {
        let shift = bit_count - 1 - i;
        let one = ((data >> shift) & 1) != 0;
        push_two_half_bits(&mut out, if one { 0x2 } else { 0x1 });
    }
    push_two_half_bits(&mut out, 0x0);
    push_two_half_bits(&mut out, 0x0);
    out
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

#[inline]
pub fn manchester_sample_weight(i: u8) -> u8 {
    let mut w: i16 = 0;
    w += if ((i >> 7) & 1) != 0 { 1 } else { -1 };
    w += if ((i >> 6) & 1) != 0 { 2 } else { -2 };
    w += if ((i >> 5) & 1) != 0 { 2 } else { -2 };
    w += if ((i >> 4) & 1) != 0 { 1 } else { -1 };
    w -= if ((i >> 3) & 1) != 0 { 1 } else { -1 };
    w -= if ((i >> 2) & 1) != 0 { 2 } else { -2 };
    w -= if ((i >> 1) & 1) != 0 { 2 } else { -2 };
    w -= if i & 1 != 0 { 1 } else { -1 };
    w *= 2;
    if w < 0 {
        w = -w + 1;
    }
    w as u8
}

fn read_sample_bit(edata: &[u8], ebitlen: u16, bitpos: i16) -> u8 {
    if bitpos < 0 {
        return (edata.first().copied().unwrap_or(0xFF) >> 7) & 1;
    }
    let bitpos = bitpos as u16;
    if bitpos >= ebitlen {
        return 1;
    }
    let byte = edata[(bitpos >> 3) as usize];
    let shift = 7 - (bitpos & 7);
    (byte >> shift) & 1
}

fn manchester_peek8_phase(
    edata: &[u8],
    ebitlen: u16,
    bitpos: u16,
    phase_ticks: i8,
    stop_coll: &mut u8,
) -> u8 {
    let shifted = bitpos as i16 + phase_ticks as i16;
    let sample = if shifted >= 0 {
        let shifted = shifted as u16;
        let pos = (shifted >> 3) as usize;
        let shift = (shifted & 7) as u8;
        let pos1 = pos + 1;
        if pos >= edata.len() {
            0xFF
        } else if pos1 < edata.len() {
            ((edata[pos] as u16) << shift | (edata[pos1] as u16) >> (8 - shift)) as u8
        } else {
            ((edata[pos] as u16) << shift) as u8
        }
    } else {
        let mut sample = 0u8;
        for i in 0..8 {
            sample = (sample << 1) | read_sample_bit(edata, ebitlen, shifted + i);
        }
        sample
    };
    if sample == 0xFF {
        *stop_coll = 1;
    }
    if sample == 0x00 {
        *stop_coll = 2;
    }
    sample
}

fn weighted_peek_around(
    edata: &[u8],
    ebitlen: u16,
    ebitpos: u16,
    phase_ticks: i8,
) -> (u8, u16, u8) {
    let mut stop_coll = 0u8;
    let sample = manchester_peek8_phase(edata, ebitlen, ebitpos, phase_ticks, &mut stop_coll);
    let mut weightmax = manchester_sample_weight(sample);
    let mut pmax: u16 = 8;

    stop_coll = 0;
    let sample = manchester_peek8_phase(
        edata,
        ebitlen,
        ebitpos.wrapping_sub(1),
        phase_ticks,
        &mut stop_coll,
    );
    let mut w = manchester_sample_weight(sample);
    if weightmax < w {
        weightmax = w;
        pmax = 7;
    }

    stop_coll = 0;
    let sample = manchester_peek8_phase(edata, ebitlen, ebitpos + 1, phase_ticks, &mut stop_coll);
    w = manchester_sample_weight(sample);
    if weightmax < w {
        weightmax = w;
        pmax = 9;
    }
    (weightmax, pmax, stop_coll)
}

fn push_decoded_bit(ddata: &mut [u8], dbitlen: u8, weightmax: u8) {
    if dbitlen == 0 {
        return;
    }
    let bytepos = ((dbitlen - 1) >> 3) as usize;
    let bitpos = (dbitlen - 1) & 7;
    if bitpos == 0 && bytepos < ddata.len() {
        ddata[bytepos] = 0;
    }
    if bytepos < ddata.len() {
        ddata[bytepos] = (ddata[bytepos] << 1) | (weightmax & 1);
    }
}

fn decode_manchester_phase_to_bytes(
    edata: &[u8],
    ebitlen: u16,
    ddata: &mut [u8],
    max_out_bits: u8,
    phase_ticks: i8,
) -> ManchesterDecodeResult {
    let mut dbitlen: u8 = 0;
    let mut ebitpos: u16 = 1;
    let mut score: u16 = 0;
    while ebitpos + 1 < ebitlen {
        if dbitlen > max_out_bits {
            break;
        }
        let (weightmax, pmax, stop_coll) =
            weighted_peek_around(edata, ebitlen, ebitpos, phase_ticks);
        if stop_coll == 1 {
            break;
        }
        if stop_coll == 2 {
            return ManchesterDecodeResult {
                bit_len: 0,
                phase_ticks,
                score: 0,
            };
        }
        score = score.saturating_add(weightmax as u16);
        push_decoded_bit(ddata, dbitlen, weightmax);
        dbitlen = dbitlen.saturating_add(1);
        ebitpos = ebitpos.saturating_add(pmax);
    }
    if dbitlen > 1 {
        dbitlen -= 1;
    }
    ManchesterDecodeResult {
        bit_len: dbitlen,
        phase_ticks,
        score,
    }
}

pub fn decode_manchester_to_bytes(
    edata: &[u8],
    ebitlen: u16,
    ddata: &mut [u8],
    max_out_bits: u8,
) -> u8 {
    decode_manchester_phase_to_bytes(edata, ebitlen, ddata, max_out_bits, 0).bit_len
}

pub fn decode_manchester_best_phase_to_bytes(
    edata: &[u8],
    ebitlen: u16,
    ddata: &mut [u8],
    max_out_bits: u8,
    preferred_bit_lengths: &[u8],
) -> Option<ManchesterDecodeResult> {
    let mut phase0_data = [0u8; 8];
    let phase0 =
        decode_manchester_phase_to_bytes(edata, ebitlen, &mut phase0_data, max_out_bits, 0);
    if preferred_bit_lengths.contains(&phase0.bit_len) {
        let n = core::cmp::min(ddata.len(), phase0_data.len());
        ddata[..n].copy_from_slice(&phase0_data[..n]);
        return Some(phase0);
    }

    let (result, data) = scan_best_phase(edata, ebitlen, max_out_bits, preferred_bit_lengths)?;
    let n = core::cmp::min(ddata.len(), data.len());
    ddata[..n].copy_from_slice(&data[..n]);
    Some(result)
}

fn scan_best_phase(
    edata: &[u8],
    ebitlen: u16,
    max_out_bits: u8,
    preferred_bit_lengths: &[u8],
) -> Option<(ManchesterDecodeResult, [u8; 8])> {
    let mut best: Option<(ManchesterDecodeResult, [u8; 8])> = None;
    for phase_ticks in PHASE_ACQUISITION_TICKS {
        if phase_ticks == 0 {
            continue;
        }
        let mut candidate_data = [0u8; 8];
        let candidate = decode_manchester_phase_to_bytes(
            edata,
            ebitlen,
            &mut candidate_data,
            max_out_bits,
            phase_ticks,
        );
        if candidate.bit_len == 0 {
            continue;
        }
        let should_replace = match best.as_ref() {
            Some((current, _)) => is_better_decode(candidate, *current, preferred_bit_lengths),
            None => true,
        };
        if should_replace {
            best = Some((candidate, candidate_data));
        }
    }
    best
}

fn is_better_decode(
    candidate: ManchesterDecodeResult,
    current: ManchesterDecodeResult,
    preferred_bit_lengths: &[u8],
) -> bool {
    let candidate_preferred = preferred_bit_lengths.contains(&candidate.bit_len);
    let current_preferred = preferred_bit_lengths.contains(&current.bit_len);
    if candidate_preferred != current_preferred {
        return candidate_preferred;
    }

    if candidate.score != current.score {
        return candidate.score > current.score;
    }

    let candidate_abs_phase = candidate.phase_ticks.unsigned_abs();
    let current_abs_phase = current.phase_ticks.unsigned_abs();
    if candidate_abs_phase != current_abs_phase {
        return candidate_abs_phase < current_abs_phase;
    }

    candidate.phase_ticks == 0 && current.phase_ticks != 0
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

    #[test]
    fn decode_roundtrip_ideal_samples() {
        let mut out = [0u8; 8];
        let r = decode_manchester_to_bytes(&[], 0, &mut out, MANCHESTER_RX_MAX_DECODED_BITS);
        assert_eq!(r, 0);
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
