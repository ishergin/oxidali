use dali2rust_dali_phy::{RxCompletedEvent, PHY_TICK_US, RX_IDLE_LINE_HIGH_TICKS};

// IEC 62386-101 Table 21
pub const HALF_BIT_NS: u64 = 416_667;

// IEC 62386-101 §7.3, §7.4.4
const BACKWARD_HALF_BITS: u64 = 18;

const TICK_NS: u64 = PHY_TICK_US as u64 * 1_000;
const TICKS_PER_SAMPLE_BYTE: usize = 8;
const CAPTURE_TICKS: usize = RxCompletedEvent::MAX_SAMPLES * TICKS_PER_SAMPLE_BYTE;
const IN_WINDOW_PRE_IDLE_TICKS: u8 = 70;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlappingAnswer {
    pub value: u8,
    pub start_ns: u64,
}

fn half_bit_is_dominant(value: u8, half_bit: u64) -> bool {
    let bit = half_bit / 2;
    let first_half = half_bit % 2 == 0;
    let one = bit == 0 || (value >> (8 - bit)) & 1 == 1;
    one == first_half
}

fn answer_is_dominant(answer: &OverlappingAnswer, at_ns: u64) -> bool {
    let Some(since) = at_ns.checked_sub(answer.start_ns) else {
        return false;
    };
    let half_bit = since / HALF_BIT_NS;
    half_bit < BACKWARD_HALF_BITS && half_bit_is_dominant(answer.value, half_bit)
}

fn line_is_dominant(answers: &[OverlappingAnswer], at_ns: u64) -> bool {
    answers.iter().any(|a| answer_is_dominant(a, at_ns))
}

fn last_dominant_ns(answers: &[OverlappingAnswer]) -> u64 {
    answers
        .iter()
        .map(|a| a.start_ns + BACKWARD_HALF_BITS * HALF_BIT_NS)
        .max()
        .unwrap_or(0)
}

fn first_dominant_tick(answers: &[OverlappingAnswer], sample_phase_ns: u64) -> Option<u64> {
    let end = last_dominant_ns(answers);
    (0..)
        .map(|k: u64| (k, sample_phase_ns + k * TICK_NS))
        .take_while(|&(_, at)| at <= end)
        .find(|&(_, at)| line_is_dominant(answers, at))
        .map(|(k, _)| k)
}

fn set_recessive(event: &mut RxCompletedEvent, tick: usize) {
    let byte = tick / TICKS_PER_SAMPLE_BYTE;
    let shift = TICKS_PER_SAMPLE_BYTE - 1 - tick % TICKS_PER_SAMPLE_BYTE;
    if let Some(slot) = event.samples.get_mut(byte) {
        *slot |= 1 << shift;
    }
}

// IEC 62386-101 §8.2.1, §8.2.5
pub fn overlapping_backward_capture(
    answers: &[OverlappingAnswer],
    sample_phase_ns: u64,
) -> RxCompletedEvent {
    let mut event = RxCompletedEvent::default();
    event.pre_idle_ticks = IN_WINDOW_PRE_IDLE_TICKS;
    let Some(first) = first_dominant_tick(answers, sample_phase_ns) else {
        return event;
    };
    let mut recessive_run = 0usize;
    let mut recorded = 0usize;
    while recorded < CAPTURE_TICKS && recessive_run < usize::from(RX_IDLE_LINE_HIGH_TICKS) {
        let at = sample_phase_ns + (first + recorded as u64) * TICK_NS;
        if line_is_dominant(answers, at) {
            recessive_run = 0;
        } else {
            recessive_run += 1;
            set_recessive(&mut event, recorded);
        }
        recorded += 1;
    }
    event.sample_count = recorded.div_ceil(TICKS_PER_SAMPLE_BYTE) as u16;
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rx_decode::RxDecode;

    #[test]
    fn a_lone_answer_decodes_to_itself_at_every_sampling_phase() {
        for value in [0x00u8, 0x5A, 0xA5, 0xFF, 0x13] {
            for phase in (0..TICK_NS).step_by(8_000) {
                let answer = OverlappingAnswer { value, start_ns: 0 };
                let event = overlapping_backward_capture(&[answer], phase);
                assert_eq!(
                    event.decode_backward_byte(),
                    Some(value),
                    "0x{value:02x} sampled at phase {phase} ns"
                );
            }
        }
    }

    const SURVEYED_ANSWERS: [u8; 8] = [0x00, 0x04, 0x20, 0x24, 0x80, 0x5A, 0xA5, 0xFF];
    const OFFSET_STEP_US: usize = 4;
    const PHASE_STEP_NS: usize = 26_000;

    #[derive(Default)]
    struct Tally {
        total: u32,
        clean: u32,
        foreign: u32,
    }

    fn survey(same_value: bool, offsets_us: core::ops::Range<u64>) -> Tally {
        let mut tally = Tally::default();
        for &a in &SURVEYED_ANSWERS {
            for &b in SURVEYED_ANSWERS.iter().filter(|&&b| (a == b) == same_value) {
                for offset_us in offsets_us.clone().step_by(OFFSET_STEP_US) {
                    for phase in (0..TICK_NS).step_by(PHASE_STEP_NS) {
                        let answers = [
                            OverlappingAnswer { value: a, start_ns: 0 },
                            OverlappingAnswer { value: b, start_ns: offset_us * 1_000 },
                        ];
                        let decoded = overlapping_backward_capture(&answers, phase)
                            .decode_backward_byte();
                        tally.total += 1;
                        tally.clean += u32::from(decoded.is_some());
                        tally.foreign += u32::from(decoded.is_some_and(|v| v != a && v != b));
                    }
                }
            }
        }
        tally
    }

    fn percent(part: u32, total: u32) -> u32 {
        part * 100 / total.max(1)
    }

    #[test]
    fn answers_under_0_4_ms_apart_often_read_as_one_clean_byte() {
        let agreeing = survey(true, 0..420);
        assert!(
            percent(agreeing.clean, agreeing.total) >= 75,
            "agreeing answers this close merge into their common value"
        );
        let differing = survey(false, 0..420);
        let clean = percent(differing.clean, differing.total);
        let foreign = percent(differing.foreign, differing.total);
        assert!(
            (50..=70).contains(&clean) && (10..=25).contains(&foreign),
            "differing answers under 0,4 ms apart: {clean} % clean, {foreign} % a value \
             neither sent — the measurement documented in 09 §Reading answers"
        );
    }

    #[test]
    fn answers_a_millisecond_or_more_apart_almost_always_violate() {
        for same_value in [true, false] {
            let far = survey(same_value, 1_000..5_001);
            assert!(
                percent(far.clean, far.total) < 1,
                "{} of {} merges this far apart still decoded clean",
                far.clean,
                far.total
            );
        }
    }
}
