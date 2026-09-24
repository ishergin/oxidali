use dali2rust_dali_phy::{RxCompletedEvent, LINE_HELD_TICKS};

use super::codec::{
    decode_manchester_best_phase_to_bytes, ManchesterDecodeResult,
    MANCHESTER_RX_MAX_DECODED_BITS, MANCHESTER_RX_MIN_DECODED_BITS,
};

#[inline]
fn pad_rx_tmp_for_manchester_peek(tmp: &mut [u8], n: usize) {
    if n < tmp.len() {
        tmp[n] = 0xFF;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SniffedFrame {
    Forward16(u16),
    Backward8(u8),
    Forward24([u8; 3]),
    UnsupportedLength(u8),
    DecodeFailed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SniffedDecode {
    pub frame: SniffedFrame,
    pub phase_ticks: i8,
    pub score: u16,
}

const BACKWARD_FRAME_BIT_LENGTHS: [u8; 1] = [8];
const SNIFFED_FRAME_BIT_LENGTHS: [u8; 3] = [8, 16, 24];

pub trait RxDecode {
    fn decode_backward_byte(&self) -> Option<u8>;
    fn decode_sniffed(&self) -> SniffedFrame;
    fn decode_sniffed_with_phase(&self) -> SniffedDecode;
    fn longest_dominant_run_ticks(&self) -> u16;
    fn is_line_held(&self) -> bool;
}

fn try_decode(
    ev: &RxCompletedEvent,
    preferred_bit_lengths: &[u8],
) -> Option<(ManchesterDecodeResult, [u8; 8])> {
    if ev.sample_count == 0 {
        return None;
    }
    let n = core::cmp::min(ev.sample_count as usize, RxCompletedEvent::MAX_SAMPLES);
    let mut tmp = [0u8; RxCompletedEvent::MAX_SAMPLES + 2];
    tmp[..n].copy_from_slice(&ev.samples[..n]);
    pad_rx_tmp_for_manchester_peek(&mut tmp, n);
    let ebitlen = (n as u16).saturating_mul(8);
    let edata_len = (n + 1).min(tmp.len());
    let mut out = [0u8; 8];
    let result = decode_manchester_best_phase_to_bytes(
        &tmp[..edata_len],
        ebitlen,
        &mut out,
        MANCHESTER_RX_MAX_DECODED_BITS,
        preferred_bit_lengths,
    );
    let result = result?;
    if result.bit_len < MANCHESTER_RX_MIN_DECODED_BITS {
        return None;
    }
    Some((result, out))
}

impl RxDecode for RxCompletedEvent {
    fn decode_backward_byte(&self) -> Option<u8> {
        let (result, out) = try_decode(self, &BACKWARD_FRAME_BIT_LENGTHS)?;
        if result.bit_len == 8 {
            Some(out[0])
        } else {
            None
        }
    }

    fn decode_sniffed(&self) -> SniffedFrame {
        self.decode_sniffed_with_phase().frame
    }

    fn decode_sniffed_with_phase(&self) -> SniffedDecode {
        let Some((result, out)) = try_decode(self, &SNIFFED_FRAME_BIT_LENGTHS) else {
            return SniffedDecode {
                frame: SniffedFrame::DecodeFailed,
                phase_ticks: 0,
                score: 0,
            };
        };
        SniffedDecode {
            frame: sniffed_from_decoded(result.bit_len, out),
            phase_ticks: result.phase_ticks,
            score: result.score,
        }
    }

    fn longest_dominant_run_ticks(&self) -> u16 {
        let n = core::cmp::min(self.sample_count as usize, RxCompletedEvent::MAX_SAMPLES);
        let mut best: u16 = 0;
        let mut run: u16 = 0;
        for byte in &self.samples[..n] {
            for shift in (0..8).rev() {
                if (byte >> shift) & 1 == 0 {
                    run = run.saturating_add(1);
                    if run > best {
                        best = run;
                    }
                } else {
                    run = 0;
                }
            }
        }
        best
    }

    fn is_line_held(&self) -> bool {
        self.longest_dominant_run_ticks() >= LINE_HELD_TICKS
    }
}

fn sniffed_from_decoded(dlen: u8, out: [u8; 8]) -> SniffedFrame {
    match dlen {
        8 => SniffedFrame::Backward8(out[0]),
        16 => SniffedFrame::Forward16(((out[0] as u16) << 8) | (out[1] as u16)),
        24 => SniffedFrame::Forward24([out[0], out[1], out[2]]),
        other => SniffedFrame::UnsupportedLength(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::HalfBitBuffer;

    fn halfbit_buffer_to_samples(hb: &HalfBitBuffer) -> Vec<u8> {
        let total_bits = hb.length as usize;
        assert!(total_bits / 8 < 9, "too many bits for 9-byte buffer");
        let mut samples = Vec::new();
        for p in (0..total_bits).step_by(2) {
            let byte_idx = p >> 3;
            let bit_offset = p & 7;
            let shift = 6 - bit_offset;
            let pair = (hb.data[byte_idx] >> shift) & 0x03;
            let sample = match pair {
                0b10 => 0x0F,
                0b01 => 0xF0,
                0b00 => 0xFF,
                0b11 => 0x00,
                _ => unreachable!(),
            };
            samples.push(sample);
        }
        samples
    }

    fn copy_phase_shifted_samples(input: &[u8], output: &mut [u8], phase_ticks: i8) {
        debug_assert_eq!(input.len(), output.len());
        let total_bits = input.len().saturating_mul(8);
        if total_bits == 0 {
            return;
        }

        let first_bit = read_sample_bit(input, 0);
        for bit in 0..total_bits {
            let src = bit as isize + phase_ticks as isize;
            let value = if src < 0 {
                first_bit
            } else if src as usize >= total_bits {
                1
            } else {
                read_sample_bit(input, src as usize)
            };
            write_sample_bit(output, bit, value);
        }
    }

    fn read_sample_bit(bytes: &[u8], bit: usize) -> u8 {
        let byte = bytes[bit / 8];
        let shift = 7 - (bit & 7);
        (byte >> shift) & 1
    }

    fn write_sample_bit(bytes: &mut [u8], bit: usize, value: u8) {
        let byte = bit / 8;
        let mask = 1u8 << (7 - (bit & 7));
        if value != 0 {
            bytes[byte] |= mask;
        } else {
            bytes[byte] &= !mask;
        }
    }

    pub fn capture_from_hex(hex: &str) -> RxCompletedEvent {
        let bytes: Vec<u8> = hex
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap_or_else(|_| panic!("not a hex byte: {h:?}")))
            .collect();
        assert!(
            bytes.len() <= RxCompletedEvent::MAX_SAMPLES,
            "a capture holds at most {} samples, got {}",
            RxCompletedEvent::MAX_SAMPLES,
            bytes.len()
        );
        event_from(&bytes)
    }

    const BENCH_CAPTURES: [&str; 2] = [
        concat!(env!("CARGO_MANIFEST_DIR"),
                "/../../tools/hil/corpus/20260915-213611/primary/wire/captures.log"),
        concat!(env!("CARGO_MANIFEST_DIR"),
                "/../../tools/hil/corpus/20260915-213611/peer/wire/captures.log"),
    ];

    struct DumpedCapture {
        detail: String,
        phase: i8,
        score: u16,
        hex: String,
    }

    fn parse_dump(line: &str) -> Option<DumpedCapture> {
        let (head, hex) = line.split_once("capture=[")?;
        let detail = head.split("DALI sniff: ").nth(1)?.split(": samples=").next()?;
        Some(DumpedCapture {
            detail: detail.to_string(),
            phase: field(head, "phase=")?.parse().ok()?,
            score: field(head, "score=")?.parse().ok()?,
            hex: clean_hex(hex)?,
        })
    }

    fn clean_hex(hex: &str) -> Option<String> {
        let payload = hex.trim_end().trim_end_matches(']');
        payload
            .split_whitespace()
            .all(|token| token.len() == 2 && u8::from_str_radix(token, 16).is_ok())
            .then(|| payload.to_string())
    }

    fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        line.split(key).nth(1)?.split_whitespace().next()
    }

    #[test]
    fn the_bench_captures_replay_to_the_verdicts_the_bench_reached() {
        let mut replayed = 0usize;
        let mut calibrations = 0usize;
        let mut unparseable = 0usize;
        for path in BENCH_CAPTURES {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("bench corpus missing at {path}: {e}"));
            for line in text.lines() {
                if !line.contains("capture=[") {
                    continue;
                }
                let Some(dump) = parse_dump(line) else {
                    unparseable += 1;
                    continue;
                };
                let decoded = capture_from_hex(&dump.hex).decode_sniffed_with_phase();
                assert_eq!(decoded.phase_ticks, dump.phase, "phase, on {line}");
                assert_eq!(decoded.score, dump.score, "score, on {line}");
                check_verdict(&dump, decoded.frame, line);
                replayed += 1;
                calibrations += usize::from(dump.detail.starts_with("calibration"));
            }
        }
        assert!(replayed > 3_000, "only {replayed} captures replayed");
        assert!(calibrations > 100, "only {calibrations} calibration captures");
        assert!(
            unparseable * 100 < replayed,
            "{unparseable} unparseable capture lines against {replayed} replayed \
             — the log is interleaving far more than the corpus recorded"
        );
    }

    fn check_verdict(dump: &DumpedCapture, frame: SniffedFrame, line: &str) {
        if dump.detail.starts_with("calibration") {
            assert!(
                !matches!(frame, SniffedFrame::DecodeFailed | SniffedFrame::UnsupportedLength(_)),
                "a calibration capture decoded on the board and must here: {line}"
            );
        } else if let Some(bits) = dump.detail.strip_prefix("unsupported decoded length (bits=") {
            let bits: u8 = bits.trim_end_matches(')').parse().expect("bit count");
            assert_eq!(frame, SniffedFrame::UnsupportedLength(bits), "on {line}");
        } else {
            assert_eq!(frame, SniffedFrame::DecodeFailed, "on {line}");
        }
    }

    #[test]
    fn a_dumped_capture_replays_to_the_same_frame() {
        use crate::codec::encode_forward24_raw;

        let hb = encode_forward24_raw(0xFF, 0xFE, 0x3D);
        let samples = halfbit_buffer_to_samples(&hb);
        let dumped: Vec<String> = samples.iter().map(|b| format!("{b:02x}")).collect();
        let replayed = capture_from_hex(&dumped.join(" "));
        assert_eq!(replayed.sample_count as usize, samples.len());
        assert_eq!(
            replayed.decode_sniffed(),
            SniffedFrame::Forward24([0xFF, 0xFE, 0x3D]),
            "the arbitration probe must survive a dump and a replay"
        );
    }

    #[test]
    fn decode_sniffed_detects_forward16_exact_length() {
        use crate::codec::encode_forward16_raw;

        let hb = encode_forward16_raw(0x82A9);
        let sample_bytes = halfbit_buffer_to_samples(&hb);

        let mut event = RxCompletedEvent::default();
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);
        event.samples[..n].copy_from_slice(&sample_bytes[..n]);
        event.sample_count = n as u16;

        let decoded = event.decode_sniffed_with_phase();
        assert_eq!(decoded.frame, SniffedFrame::Forward16(0x82A9));
        assert_eq!(decoded.phase_ticks, 0);
    }

    #[test]
    fn decode_sniffed_detects_forward24() {
        use crate::codec::encode_forward24_raw;

        let hb = encode_forward24_raw(0xA1, 0x02, 0x03);
        let sample_bytes = halfbit_buffer_to_samples(&hb);

        let mut event = RxCompletedEvent::default();
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);
        event.samples[..n].copy_from_slice(&sample_bytes[..n]);
        event.sample_count = n as u16;

        let frame = event.decode_sniffed();
        match frame {
            SniffedFrame::Forward24([a, b, c]) => {
                assert_eq!(a, 0xA1);
                assert_eq!(b, 0x02);
                assert_eq!(c, 0x03);
            }
            other => panic!("expected Forward24, got {:?}", other),
        }
    }

    #[test]
    fn decode_sniffed_rejects_non_dali_bit_lengths() {
        use crate::codec::encode_to_half_bits;

        let hb = encode_to_half_bits(0xABCDE, 20);
        let sample_bytes = halfbit_buffer_to_samples(&hb);

        let mut event = RxCompletedEvent::default();
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);
        event.samples[..n].copy_from_slice(&sample_bytes[..n]);
        event.sample_count = n as u16;

        assert_eq!(event.decode_sniffed(), SniffedFrame::UnsupportedLength(20));
    }

    #[test]
    fn decode_sniffed_best_phase_recovers_shifted_forward16() {
        use crate::codec::encode_forward16_raw;

        let hb = encode_forward16_raw(0x8244);
        let sample_bytes = halfbit_buffer_to_samples(&hb);
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);

        let mut recovered_with_adjusted_phase = false;
        for shift in [-2, -1, 1, 2] {
            let mut shifted = [0u8; RxCompletedEvent::MAX_SAMPLES];
            copy_phase_shifted_samples(&sample_bytes[..n], &mut shifted[..n], shift);

            let mut event = RxCompletedEvent::default();
            event.samples[..n].copy_from_slice(&shifted[..n]);
            event.sample_count = n as u16;

            let decoded = event.decode_sniffed_with_phase();
            if decoded.frame == SniffedFrame::Forward16(0x8244) && decoded.phase_ticks != 0 {
                recovered_with_adjusted_phase = true;
                break;
            }
        }

        assert!(recovered_with_adjusted_phase);
    }

    #[test]
    fn decode_backward_byte_best_phase_recovers_shifted_backward8() {
        use crate::codec::encode_to_half_bits;

        let hb = encode_to_half_bits(0x5A, 8);
        let sample_bytes = halfbit_buffer_to_samples(&hb);
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);
        let mut shifted = [0u8; RxCompletedEvent::MAX_SAMPLES];
        copy_phase_shifted_samples(&sample_bytes[..n], &mut shifted[..n], -2);

        let mut event = RxCompletedEvent::default();
        event.samples[..n].copy_from_slice(&shifted[..n]);
        event.sample_count = n as u16;

        assert_eq!(event.decode_backward_byte(), Some(0x5A));
    }

    #[test]
    fn decode_backward_byte_does_not_accept_forward16() {
        use crate::codec::encode_forward16_raw;

        let hb = encode_forward16_raw(0x8244);
        let sample_bytes = halfbit_buffer_to_samples(&hb);

        let mut event = RxCompletedEvent::default();
        let n = sample_bytes.len().min(RxCompletedEvent::MAX_SAMPLES);
        event.samples[..n].copy_from_slice(&sample_bytes[..n]);
        event.sample_count = n as u16;

        assert_eq!(event.decode_backward_byte(), None);
    }

    fn event_from(samples: &[u8]) -> RxCompletedEvent {
        let mut event = RxCompletedEvent::default();
        let n = samples.len().min(RxCompletedEvent::MAX_SAMPLES);
        event.samples[..n].copy_from_slice(&samples[..n]);
        event.sample_count = n as u16;
        event
    }

    #[test]
    fn a_capture_with_no_mid_bit_transitions_is_not_a_frame() {
        for level in [0x00u8, 0xFF] {
            let event = event_from(&[level; 34]);
            let frame = event.decode_sniffed();
            assert!(
                !matches!(
                    frame,
                    SniffedFrame::Forward16(_)
                        | SniffedFrame::Forward24(_)
                        | SniffedFrame::Backward8(_)
                ),
                "a transition-free capture at level 0x{level:02X} decoded as a \
                 readable frame: {frame:?}"
            );
        }
    }

    #[test]
    fn a_glitch_shorter_than_any_frame_is_rejected() {
        for len in 0..4usize {
            let mut samples = vec![0u8; len];
            for (i, s) in samples.iter_mut().enumerate() {
                *s = if i % 2 == 0 { 0x0F } else { 0xF0 };
            }
            let frame = event_from(&samples).decode_sniffed();
            assert!(
                !matches!(
                    frame,
                    SniffedFrame::Forward16(_)
                        | SniffedFrame::Forward24(_)
                        | SniffedFrame::Backward8(_)
                ),
                "a {len}-sample glitch decoded as a frame: {frame:?}"
            );
        }
    }

    #[test]
    fn a_sample_count_past_the_buffer_is_clamped_not_believed() {
        use crate::codec::encode_forward16_raw;

        let hb = encode_forward16_raw(0x82A9);
        let samples = halfbit_buffer_to_samples(&hb);
        let mut event = event_from(&samples);
        event.sample_count = u16::MAX;
        let _ = event.decode_sniffed();
    }

    #[test]
    fn two_colliding_answers_do_not_decode_to_a_byte() {
        use crate::codec::merge_dominant;

        let merged = merge_dominant(&[0x5A, 0xA5], 8).expect("two answers");
        let samples = halfbit_buffer_to_samples(&merged);
        assert_eq!(
            event_from(&samples).decode_backward_byte(),
            None,
            "a collided backward window must not yield a byte — reading one \
             invents an answer neither gear sent"
        );
    }

    #[test]
    fn three_colliding_answers_do_not_decode_to_a_byte() {
        use crate::codec::merge_dominant;

        let merged = merge_dominant(&[0x0F, 0x33, 0x55], 8).expect("three answers");
        let samples = halfbit_buffer_to_samples(&merged);
        assert_eq!(event_from(&samples).decode_backward_byte(), None);
    }

    #[test]
    fn identical_colliding_answers_still_read_as_that_answer() {
        use crate::codec::merge_dominant;

        let merged = merge_dominant(&[0x5A, 0x5A, 0x5A], 8).expect("three answers");
        let samples = halfbit_buffer_to_samples(&merged);
        assert_eq!(event_from(&samples).decode_backward_byte(), Some(0x5A));
    }

    fn ticks_of(samples: &[u8]) -> Vec<u8> {
        let mut ticks = Vec::with_capacity(samples.len() * 8);
        for &byte in samples {
            for shift in (0..8).rev() {
                ticks.push((byte >> shift) & 1);
            }
        }
        ticks
    }

    fn capture_of_ticks(ticks: &[u8]) -> RxCompletedEvent {
        let samples: Vec<u8> = ticks
            .chunks_exact(8)
            .map(|c| c.iter().fold(0u8, |acc, &bit| (acc << 1) | bit))
            .collect();
        event_from(&samples)
    }

    fn capture_missing_ticks(samples: &[u8], at: usize, count: usize) -> RxCompletedEvent {
        let mut ticks = ticks_of(samples);
        ticks.drain(at..at + count);
        capture_of_ticks(&ticks)
    }

    fn nominal_samples(bits: u32, width: u8) -> Vec<u8> {
        use crate::codec::encode_to_half_bits;

        let mut samples = halfbit_buffer_to_samples(&encode_to_half_bits(bits, width));
        samples.extend_from_slice(&[0xFF, 0xFF]);
        samples
    }

    #[test]
    fn one_lost_tick_never_turns_a_frame_into_a_different_frame() {
        for (bits, width, want) in [
            (0x82A9, 16, SniffedFrame::Forward16(0x82A9)),
            (0xFFFE3D, 24, SniffedFrame::Forward24([0xFF, 0xFE, 0x3D])),
        ] {
            let samples = nominal_samples(bits, width);
            assert_eq!(event_from(&samples).decode_sniffed(), want, "the clean frame");
            for at in 0..ticks_of(&samples).len() - 1 {
                let got = capture_missing_ticks(&samples, at, 1).decode_sniffed();
                assert!(
                    got == want
                        || !matches!(
                            got,
                            SniffedFrame::Forward16(_)
                                | SniffedFrame::Forward24(_)
                                | SniffedFrame::Backward8(_)
                        ),
                    "losing tick {at} of {width}-bit 0x{bits:06X} fabricated {got:?}"
                );
            }
        }
    }

    const CAPTURES_THAT_ARE_NOT_FRAMES: [(&str, &str); 14] = [
        ("dapc short 0 level 6", "1f 1f 1f 1f 1f 8f 8f 8f 8f 8f 87 8f f8 f8 78 07 87 fe 0f f0 0f ff ff"),
        ("f16 0x007d", "1e 0e 1e 1e 1e 1e 1e 1e 1e 1e 1e f0 0f 0f 0f 0f f0 0f ff ff"),
        ("f16 0x01fd", "1f 1f 1f 1f 1f 1f 1f 0f 0f f0 f1 e0 1e 1e 1e 1f e0 1f ff ff"),
        ("f16 0x0ffc", "0f 0f 0f 0f 0f 0f 7f 1f 1f 1f 1f 1f 1f 1f 1f 1f f1 f1 ff 3e 3e 3e 3f e0 3f ff ff"),
        ("f16 0x1ffc", "0f 0f 0f 0f 0f 7f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f f0 f0 ff ff ff"),
        ("f16 0xff41", "1e 1e 1e 1e 1e 1e 1e 1e 1f e0 03 ff 0f 1f 1f 1f f0 1f ff ff"),
        ("probe as 0x080385", "0f 0f 0f 0f 0f 0f 7f 87 87 87 87 87 87 87 87 87 f8 78 ff c7 87 87 87 f8 07 ff ff"),
        ("probe as 0xeffebd", "1f 1f 1f 1f fe 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3f e3 8f 80 f8 f8 f8 ff 80 ff ff ff"),
        ("probe as 0xfc03dd", "0f 0f 0f 0f 0f 0f 0f f8 78 78 78 78 78 78 78 7f 87 87 80 78 1c 7c 7f c0 7f ff ff"),
        ("probe as 0xfffefd", "0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f 0f f0 3c 3c 03 c3 c3 c3 fc 03 ff ff"),
        ("backward as 0x00", "0f 1f 1f 1f 1f 1f 1f 1f 9f f1 ff ff"),
        ("backward as 0xbd", "1f 1f 01 f1 f1 f1 ff 01 ff ff"),
        ("backward as 0xfd", "1f e1 e1 e0 1e 1e 1e 1f e0 1f ff ff"),
        ("collision break, 1,25 ms dominant", "0f f0 f0 00 00 00 00 00 0f 0f 0f f8 f8 f8 f8 f8 0f ff ff"),
    ];

    #[test]
    fn real_bench_captures_that_are_not_frames_stay_refused() {
        for (what, hex) in CAPTURES_THAT_ARE_NOT_FRAMES {
            let frame = capture_from_hex(hex).decode_sniffed();
            assert!(
                !matches!(frame, SniffedFrame::Forward16(_)
                    | SniffedFrame::Forward24(_) | SniffedFrame::Backward8(_)),
                "{what}: a capture off the bench that is not a frame decoded as {frame:?}"
            );
        }
    }

    fn wire_half_bits(value: u32, width: u8) -> Vec<u8> {
        let mut levels = vec![0u8, 1];
        for shift in (0..width).rev() {
            let one = (value >> shift) & 1 == 1;
            levels.extend_from_slice(if one { &[0, 1] } else { &[1, 0] });
        }
        levels
    }

    fn capture_at_rate(levels: &[u8], half_us: f64, offset_us: f64) -> RxCompletedEvent {
        const TICK_US: f64 = 104.0;
        let span = levels.len() as f64 * half_us + TICK_US * 24.0;
        let mut ticks = Vec::new();
        let mut at = -offset_us;
        while at < span {
            let idx = if at < 0.0 { usize::MAX } else { (at / half_us) as usize };
            ticks.push(*levels.get(idx).unwrap_or(&1));
            at += TICK_US;
        }
        let first_dominant = ticks.iter().position(|&l| l == 0).unwrap_or(0);
        let mut ticks = ticks[first_dominant..].to_vec();
        while ticks.len() % 8 != 0 {
            ticks.push(1);
        }
        capture_of_ticks(&ticks)
    }

    #[test]
    fn the_receiver_covers_table_21_rate_tolerance_at_every_offset() {
        const NOMINAL_HALF_US: f64 = 416.6667;
        for (value, width, want) in [
            (0xFFFE3Du32, 24u8, SniffedFrame::Forward24([0xFF, 0xFE, 0x3D])),
            (0x82A9, 16, SniffedFrame::Forward16(0x82A9)),
        ] {
            let levels = wire_half_bits(value, width);
            for percent in [-6, -4, -2, 0, 2, 4, 6] {
                let half_us = NOMINAL_HALF_US * (1.0 + f64::from(percent) / 100.0);
                for offset in 0..13 {
                    let capture = capture_at_rate(&levels, half_us, 8.0 * f64::from(offset));
                    assert_eq!(
                        capture.decode_sniffed(),
                        want,
                        "{width}-bit frame at {percent:+} % half bit, sampling offset {offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_probe_one_tick_short_is_refused_and_the_tick_restores_it() {
        const SHORT: &str = "1f 1e 1f 1e 1f 1e 1f 1f 1e 1f 1f 1f 1f 1e 1e 1f \
             f1 f1 e0 1e 1f 1e 1f f0 7f ff ff";
        assert_eq!(
            capture_from_hex(SHORT).decode_sniffed(),
            SniffedFrame::UnsupportedLength(23),
            "the decoder refuses a probe a tick short — a DECISION, not a defect: \
             the width that would read it also reads the captures in \
             CAPTURES_THAT_ARE_NOT_FRAMES, and this assert is here so a change \
             to it is made on purpose"
        );

        let samples: Vec<u8> = SHORT
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).expect("hex"))
            .collect();
        let mut ticks = ticks_of(&samples);
        ticks.insert(179, 1);
        assert_eq!(
            capture_of_ticks(&ticks).decode_sniffed(),
            SniffedFrame::Forward24([0xFF, 0xFE, 0x3D]),
            "the probe the standby actually sent"
        );
    }

    fn held_line_capture(dominant_ticks: usize) -> RxCompletedEvent {
        let mut ticks = vec![0u8; dominant_ticks];
        ticks.extend(core::iter::repeat(1).take(16));
        while ticks.len() % 8 != 0 {
            ticks.push(1);
        }
        capture_of_ticks(&ticks)
    }

    #[test]
    fn a_held_line_measures_its_own_length_and_reads_as_held() {
        for dominant in [24usize, 48, 67, 120] {
            let event = held_line_capture(dominant);
            assert_eq!(
                event.longest_dominant_run_ticks(),
                dominant as u16,
                "a hold is one run and the whole capture is it"
            );
            assert!(event.is_line_held(), "{dominant} ticks dominant is a hold");
        }
    }

    #[test]
    fn no_backward_frame_ever_holds_the_line() {
        for byte in 0u32..=0xFF {
            let event = event_from(&nominal_samples(byte, 8));
            let run = event.longest_dominant_run_ticks();
            assert!(
                run <= 8,
                "byte 0x{byte:02x}: Manchester allows at most two adjacent \
                 dominant half-bits, and that is eight ticks — measured {run}"
            );
            assert!(!event.is_line_held(), "byte 0x{byte:02x} is a frame");
        }
    }

    #[test]
    fn no_forward_frame_ever_holds_the_line() {
        for (value, width) in [(0xFFFE3Du32, 24u8), (0x82A9, 16), (0x0000, 16), (0xFFFF, 16)] {
            let event = event_from(&nominal_samples(value, width));
            assert!(
                !event.is_line_held(),
                "0x{value:06x}/{width}: a forward frame is signalling, not a hold"
            );
        }
    }

    #[test]
    fn samples_past_the_capture_are_not_read() {
        let mut event = held_line_capture(30);
        let n = event.sample_count as usize;
        for slot in event.samples[n..].iter_mut() {
            *slot = 0x00;
        }
        assert_eq!(
            event.longest_dominant_run_ticks(),
            30,
            "the run stops at `sample_count`; the stale dominant bytes behind \
             it are the PREVIOUS capture and must not extend this one"
        );
    }

    fn merged_capture(a: u32, b: u32, width: u8, offset_us: f64) -> RxCompletedEvent {
        const TICK_US: f64 = 104.0;
        const HALF_US: f64 = 416.6667;
        let la = wire_half_bits(a, width);
        let lb = wire_half_bits(b, width);
        let span = (la.len() as f64) * HALF_US + offset_us + TICK_US * 24.0;
        let level_at = |levels: &[u8], t: f64| -> u8 {
            if t < 0.0 {
                1
            } else {
                *levels.get((t / HALF_US) as usize).unwrap_or(&1)
            }
        };
        let mut ticks = Vec::new();
        let mut at = 0.0f64;
        while at < span {
            let dominant = level_at(&la, at) == 0 || level_at(&lb, at - offset_us) == 0;
            ticks.push(u8::from(!dominant));
            at += TICK_US;
        }
        let first = ticks.iter().position(|&l| l == 0).unwrap_or(0);
        let mut ticks = ticks[first..].to_vec();
        while ticks.len() % 8 != 0 {
            ticks.push(1);
        }
        capture_of_ticks(&ticks)
    }

    #[test]
    fn two_answers_half_a_bit_apart_hold_the_line_as_a_hold_does() {
        let event = merged_capture(0xFF, 0xFF, 8, 416.6667);
        let run = event.longest_dominant_run_ticks();
        assert!(
            run >= LINE_HELD_TICKS,
            "measured {run} ticks — if this ever drops below the threshold the \
             comment above is wrong, not the test"
        );
        assert!(event.is_line_held());
    }

    #[test]
    fn an_overlap_that_leaves_recessive_gaps_is_not_a_hold() {
        for (a, b, offset) in [(0xFFu32, 0xFFu32, 0.0f64), (0x00, 0xFF, 1250.0)] {
            let event = merged_capture(a, b, 8, offset);
            assert!(
                !event.is_line_held(),
                "0x{a:02x}+0x{b:02x} at {offset} us: {} ticks",
                event.longest_dominant_run_ticks()
            );
        }
    }
}
