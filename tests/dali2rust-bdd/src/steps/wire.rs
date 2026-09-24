use dali2rust_adapters::dali::transport::mock::BusFrameSource;

pub fn frame_index(frames: &[u16], frame: u16) -> usize {
    frames
        .iter()
        .position(|candidate| *candidate == frame)
        .unwrap_or_else(|| panic!("expected frame 0x{frame:04X} in trace {frames:?}"))
}

pub fn assert_frame_before(frames: &[u16], first: u16, second: u16) {
    let first_index = frame_index(frames, first);
    let second_index = frame_index(frames, second);
    assert!(
        first_index < second_index,
        "frame 0x{first:04X} should be sent before 0x{second:04X}; trace: {frames:?}"
    );
}

pub fn assert_nothing_between_frames(trace: &[(BusFrameSource, u16)], first: u16, second: u16) {
    let frames: Vec<u16> = trace.iter().map(|(_, frame)| *frame).collect();
    let first_index = frame_index(&frames, first);
    let next = frames.get(first_index + 1).copied().unwrap_or_else(|| {
        panic!("frame 0x{first:04X} is the last on the bus; expected 0x{second:04X} after it")
    });
    assert_eq!(
        next, second,
        "0x{second:04X} must follow 0x{first:04X} with nothing in between; \
         found 0x{next:04X} instead. Bus trace: {trace:?}"
    );
}

const TABLE_22_BANDS_US: [(u32, u32, u8); 5] = [
    (13_500, 14_700, 1),
    (14_900, 16_100, 2),
    (16_300, 17_700, 3),
    (17_900, 19_300, 4),
    (19_500, 21_100, 5),
];

const PRIORITY_5_MAX_US: u32 = 21_100;

const TX_ARM_LEAD_US: u32 = 3 * 104;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClaim {
    Priority(u8),
    BusRelease,
}

pub fn frame_claim(index: usize, requested_us: u32) -> FrameClaim {
    let settle_us = requested_us + TX_ARM_LEAD_US;
    if settle_us > PRIORITY_5_MAX_US {
        return FrameClaim::BusRelease;
    }
    TABLE_22_BANDS_US
        .iter()
        .find(|(min, max, _)| settle_us >= *min && settle_us <= *max)
        .map(|(_, _, priority)| FrameClaim::Priority(*priority))
        .unwrap_or_else(|| {
            panic!(
                "forward frame {index} was sent with {settle_us} µs of settling, which falls in no \
                 IEC 62386-101 Table 22 band ({TABLE_22_BANDS_US:?}) and is not a §9.2 bus release \
                 (> {PRIORITY_5_MAX_US} µs). Either the frame took a path that records no priority \
                 (send_forward_frame writes 0), or it landed in one of the 200 µs guards between \
                 two bands, which announces no priority at all."
            )
        })
}

pub fn priority_of_settle_us(index: usize, settle_us: u32) -> u8 {
    match frame_claim(index, settle_us) {
        FrameClaim::Priority(priority) => priority,
        FrameClaim::BusRelease => panic!(
            "forward frame {index} carried a §9.2 bus release ({settle_us} µs), not a priority"
        ),
    }
}

pub const RELEASE: u8 = 0;

pub fn priorities_of(settle_us: &[u32]) -> Vec<u8> {
    settle_us
        .iter()
        .enumerate()
        .map(|(index, us)| match frame_claim(index, *us) {
            FrameClaim::Priority(priority) => priority,
            FrameClaim::BusRelease => RELEASE,
        })
        .collect()
}
