use crate::fsm::DALI_RX_SAMPLE_BUF_LEN;

pub const MANCHESTER_RX_MAX_DECODED_BITS: u8 = 32;
pub const MANCHESTER_RX_MIN_DECODED_BITS: u8 = 3;
pub const PHASE_SCAN_COUNT: u8 = 4;
pub const DECODED_BYTES: usize = 8;

const WINDOW_LEN: usize = DALI_RX_SAMPLE_BUF_LEN + 2;
const WINDOW_BITS_PER_SAMPLE: u16 = 8;
const PEEK_IDLE: u8 = 0xFF;
const PEEK_HELD: u8 = 0x00;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeekStop {
    None,
    Idle,
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitLengths(u32);

impl BitLengths {
    pub const fn of(mut lengths: &[u8]) -> Self {
        let mut mask = 0u32;
        while let [first, rest @ ..] = lengths {
            if let Some(bit) = 1u32.checked_shl(*first as u32) {
                mask |= bit;
            }
            lengths = rest;
        }
        Self(mask)
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub const fn contains(self, length: u8) -> bool {
        match 1u32.checked_shl(length as u32) {
            Some(bit) => self.0 & bit != 0,
            None => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseDecode {
    pub bit_len: u8,
    pub phase_ticks: i8,
    pub score: u16,
    pub data: [u8; DECODED_BYTES],
}

impl PhaseDecode {
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    const fn failed(phase_ticks: i8) -> Self {
        Self { bit_len: 0, phase_ticks, score: 0, data: [0; DECODED_BYTES] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleWindow {
    bytes: [u8; WINDOW_LEN],
    len: u16,
    bits: u16,
}

impl SampleWindow {
    pub const EMPTY: Self = Self { bytes: [0; WINDOW_LEN], len: 0, bits: 0 };

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn of_capture(samples: &[u8; DALI_RX_SAMPLE_BUF_LEN], sample_count: u16) -> Self {
        let n = if usize::from(sample_count) < DALI_RX_SAMPLE_BUF_LEN {
            usize::from(sample_count)
        } else {
            DALI_RX_SAMPLE_BUF_LEN
        };
        let mut window = Self::EMPTY;
        let mut i = 0usize;
        while i < n {
            if let (Some(slot), Some(sample)) = (window.bytes.get_mut(i), samples.get(i)) {
                *slot = *sample;
            }
            i = i.saturating_add(1);
        }
        if let Some(pad) = window.bytes.get_mut(n) {
            *pad = PEEK_IDLE;
        }
        window.len = n.saturating_add(1) as u16;
        window.bits = (n as u16).saturating_mul(WINDOW_BITS_PER_SAMPLE);
        window
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn byte(&self, pos: u16) -> Option<u8> {
        if pos >= self.len {
            return None;
        }
        self.bytes.get(usize::from(pos)).copied()
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn sample_bit(&self, bitpos: i16) -> u8 {
        if bitpos < 0 {
            return self.byte(0).unwrap_or(PEEK_IDLE) >> 7;
        }
        let bitpos = bitpos as u16;
        if bitpos >= self.bits {
            return 1;
        }
        let byte = self.byte(bitpos >> 3).unwrap_or(PEEK_IDLE);
        (byte << (bitpos & 7)) >> 7
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn peek8(&self, bitpos: u16, phase_ticks: i8) -> (u8, PeekStop) {
        let shifted = (bitpos as i16).wrapping_add(i16::from(phase_ticks));
        let sample = if shifted >= 0 {
            let shifted = shifted as u16;
            let pos = shifted >> 3;
            let shift = shifted & 7;
            match (self.byte(pos), self.byte(pos.saturating_add(1))) {
                (None, _) => PEEK_IDLE,
                (Some(first), Some(next)) => {
                    ((u16::from(first) << shift) | (u16::from(next) >> 8u16.wrapping_sub(shift)))
                        as u8
                }
                (Some(first), None) => (u16::from(first) << shift) as u8,
            }
        } else {
            let mut sample = 0u8;
            let mut offset = 0i16;
            while offset < 8 {
                sample = (sample << 1) | self.sample_bit(shifted.wrapping_add(offset));
                offset = offset.wrapping_add(1);
            }
            sample
        };
        let stop = match sample {
            PEEK_IDLE => PeekStop::Idle,
            PEEK_HELD => PeekStop::Held,
            _ => PeekStop::None,
        };
        (sample, stop)
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    fn weighted_peek_around(&self, bitpos: u16, phase_ticks: i8) -> (u8, u16, PeekStop) {
        let (centre, _) = self.peek8(bitpos, phase_ticks);
        let mut weightmax = sample_weight(centre);
        let mut pmax: u16 = 8;
        let (early, _) = self.peek8(bitpos.wrapping_sub(1), phase_ticks);
        let early = sample_weight(early);
        if weightmax < early {
            weightmax = early;
            pmax = 7;
        }
        let (late, stop) = self.peek8(bitpos.saturating_add(1), phase_ticks);
        let late = sample_weight(late);
        if weightmax < late {
            weightmax = late;
            pmax = 9;
        }
        (weightmax, pmax, stop)
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn decode_phase(&self, max_out_bits: u8, phase_ticks: i8) -> PhaseDecode {
        let mut out = PhaseDecode::failed(phase_ticks);
        let mut bitpos: u16 = 1;
        while bitpos.saturating_add(1) < self.bits {
            if out.bit_len > max_out_bits {
                break;
            }
            let (weightmax, pmax, stop) = self.weighted_peek_around(bitpos, phase_ticks);
            match stop {
                PeekStop::Idle => break,
                PeekStop::Held => return PhaseDecode::failed(phase_ticks),
                PeekStop::None => {}
            }
            out.score = out.score.saturating_add(u16::from(weightmax));
            push_decoded_bit(&mut out.data, out.bit_len, weightmax);
            out.bit_len = out.bit_len.saturating_add(1);
            bitpos = bitpos.saturating_add(pmax);
        }
        if out.bit_len > 1 {
            out.bit_len = out.bit_len.saturating_sub(1);
        }
        out
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
const fn bit_weight(sample: u8, bit: u8, weight: i16) -> i16 {
    let set = match sample.checked_shr(bit as u32) {
        Some(shifted) => shifted & 1 != 0,
        None => false,
    };
    if set {
        weight
    } else {
        weight.wrapping_neg()
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub const fn sample_weight(sample: u8) -> u8 {
    let first = bit_weight(sample, 7, 1)
        .wrapping_add(bit_weight(sample, 6, 2))
        .wrapping_add(bit_weight(sample, 5, 2))
        .wrapping_add(bit_weight(sample, 4, 1));
    let second = bit_weight(sample, 3, 1)
        .wrapping_add(bit_weight(sample, 2, 2))
        .wrapping_add(bit_weight(sample, 1, 2))
        .wrapping_add(bit_weight(sample, 0, 1));
    let w = first.wrapping_sub(second).wrapping_mul(2);
    if w < 0 {
        w.wrapping_neg().wrapping_add(1) as u8
    } else {
        w as u8
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
fn push_decoded_bit(data: &mut [u8; DECODED_BYTES], bit_len: u8, weightmax: u8) {
    if bit_len == 0 {
        return;
    }
    let index = bit_len.wrapping_sub(1);
    let Some(byte) = data.get_mut(usize::from(index >> 3)) else {
        return;
    };
    if index & 7 == 0 {
        *byte = 0;
    }
    *byte = (*byte << 1) | (weightmax & 1);
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
const fn scan_phase(index: u8) -> i8 {
    let offset = ((index & 3) as i8).wrapping_sub(2);
    if offset >= 0 {
        offset.wrapping_add(1)
    } else {
        offset
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
pub fn is_better_decode(candidate: &PhaseDecode, current: &PhaseDecode, preferred: BitLengths) -> bool {
    let candidate_preferred = preferred.contains(candidate.bit_len);
    if candidate_preferred != preferred.contains(current.bit_len) {
        return candidate_preferred;
    }
    if candidate.score != current.score {
        return candidate.score > current.score;
    }
    let candidate_abs = candidate.phase_ticks.unsigned_abs();
    let current_abs = current.phase_ticks.unsigned_abs();
    if candidate_abs != current_abs {
        return candidate_abs < current_abs;
    }
    candidate.phase_ticks == 0 && current.phase_ticks != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsrDecode {
    NotDecoded,
    Decoded(Option<PhaseDecode>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseSearch {
    Searching,
    Done(Option<PhaseDecode>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BestPhaseSearch {
    preferred: BitLengths,
    step: u8,
    best: Option<PhaseDecode>,
}

impl BestPhaseSearch {
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub const fn new(preferred: BitLengths) -> Self {
        Self { preferred, step: 0, best: None }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn step(&mut self, window: &SampleWindow) -> PhaseSearch {
        if self.step == 0 {
            self.step = 1;
            let first = window.decode_phase(MANCHESTER_RX_MAX_DECODED_BITS, 0);
            if self.preferred.contains(first.bit_len) {
                return PhaseSearch::Done(accepted(Some(first)));
            }
            return PhaseSearch::Searching;
        }
        if self.step > PHASE_SCAN_COUNT {
            return PhaseSearch::Done(accepted(self.best));
        }
        let phase = scan_phase(self.step.wrapping_sub(1));
        let candidate = window.decode_phase(MANCHESTER_RX_MAX_DECODED_BITS, phase);
        self.step = self.step.saturating_add(1);
        if candidate.bit_len != 0 {
            let replace = match &self.best {
                Some(current) => is_better_decode(&candidate, current, self.preferred),
                None => true,
            };
            if replace {
                self.best = Some(candidate);
            }
        }
        if self.step > PHASE_SCAN_COUNT {
            return PhaseSearch::Done(accepted(self.best));
        }
        PhaseSearch::Searching
    }

    pub fn run(mut self, window: &SampleWindow) -> Option<PhaseDecode> {
        loop {
            if let PhaseSearch::Done(found) = self.step(window) {
                return found;
            }
        }
    }
}

#[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
const fn accepted(found: Option<PhaseDecode>) -> Option<PhaseDecode> {
    match found {
        Some(decode) if decode.bit_len >= MANCHESTER_RX_MIN_DECODED_BITS => Some(decode),
        _ => None,
    }
}

pub const SNIFFED_BIT_LENGTHS: BitLengths = BitLengths::of(&[8, 16, 24]);
pub const FORWARD24_BITS: u8 = 24;

#[cfg(test)]
mod tests {
    use super::*;

    const SNIFFED: BitLengths = BitLengths::of(&[8, 16, 24]);

    fn window_of(samples: &[u8]) -> SampleWindow {
        let mut buf = [0u8; DALI_RX_SAMPLE_BUF_LEN];
        buf[..samples.len()].copy_from_slice(samples);
        SampleWindow::of_capture(&buf, samples.len() as u16)
    }

    #[test]
    fn the_preferred_lengths_are_a_set() {
        assert!(SNIFFED.contains(8) && SNIFFED.contains(16) && SNIFFED.contains(24));
        assert!(!SNIFFED.contains(0) && !SNIFFED.contains(23) && !SNIFFED.contains(200));
    }

    #[test]
    fn the_scan_visits_the_four_phases_off_the_centre() {
        let phases: Vec<i8> = (0..PHASE_SCAN_COUNT).map(scan_phase).collect();
        assert_eq!(phases, [-2, -1, 1, 2]);
    }

    #[test]
    fn a_balanced_half_bit_pair_outweighs_a_flat_run() {
        assert!(sample_weight(0b1111_0000) > sample_weight(0b1111_1111));
        assert_eq!(sample_weight(0b1111_0000), sample_weight(0b0000_1111) - 1);
    }

    #[test]
    fn an_empty_capture_decodes_to_nothing() {
        assert_eq!(BestPhaseSearch::new(SNIFFED).run(&window_of(&[])), None);
    }

    #[test]
    fn a_held_line_fails_the_phase_it_was_found_in() {
        let decode = window_of(&[0x0F, 0x00, 0x00, 0x00]).decode_phase(32, 0);
        assert_eq!((decode.bit_len, decode.score), (0, 0));
    }

    #[test]
    fn the_search_takes_at_most_one_phase_per_step() {
        let window = window_of(&[0x0F; 27]);
        let mut search = BestPhaseSearch::new(SNIFFED);
        let mut steps = 0;
        while search.step(&window) == PhaseSearch::Searching {
            steps += 1;
            assert!(steps <= PHASE_SCAN_COUNT, "the search must end after the scan");
        }
    }
}
