use core::fmt;

pub const MAX_CLIENT_FRAME_BYTES: usize = 512;

const FIN_BIT: u8 = 0x80;
const RSV_BITS: u8 = 0x70;
const OPCODE_BITS: u8 = 0x0f;
const MASK_BIT: u8 = 0x80;
const LEN7_BITS: u8 = 0x7f;
const LEN_IS_16_BIT: u8 = 126;
const LEN_IS_64_BIT: u8 = 127;
const MASK_KEY_BYTES: usize = 4;
const MAX_CONTROL_PAYLOAD: usize = 125;
const MAX_SHORT_LEN: usize = 125;
const EXTENDED_LEN_BYTES: usize = 2;

pub const PRELUDE_BYTES: usize = 2;
pub const MAX_TAIL_BYTES: usize = EXTENDED_LEN_BYTES + MASK_KEY_BYTES;
pub const MAX_SERVER_HEADER_BYTES: usize = PRELUDE_BYTES + EXTENDED_LEN_BYTES;

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xa;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    const fn code(self) -> u8 {
        match self {
            Self::Continuation => OPCODE_CONTINUATION,
            Self::Text => OPCODE_TEXT,
            Self::Binary => OPCODE_BINARY,
            Self::Close => OPCODE_CLOSE,
            Self::Ping => OPCODE_PING,
            Self::Pong => OPCODE_PONG,
        }
    }

    const fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            OPCODE_CONTINUATION => Some(Self::Continuation),
            OPCODE_TEXT => Some(Self::Text),
            OPCODE_BINARY => Some(Self::Binary),
            OPCODE_CLOSE => Some(Self::Close),
            OPCODE_PING => Some(Self::Ping),
            OPCODE_PONG => Some(Self::Pong),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    Unmasked,
    ReservedBits,
    ReservedOpcode,
    ControlTooLong,
    ControlFragmented,
    CloseTooShort,
    Fragmented,
    LengthUnsupported,
    LengthNotMinimal,
    TailLength,
    TooLarge,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unmasked => "a client frame must be masked",
            Self::ReservedBits => "a reserved bit is set",
            Self::ReservedOpcode => "the opcode is reserved",
            Self::ControlTooLong => "a control frame carries more than 125 bytes",
            Self::ControlFragmented => "a control frame is fragmented",
            Self::CloseTooShort => "a close frame carries one byte, which is neither empty nor a code",
            Self::Fragmented => "a fragmented data frame is not accepted",
            Self::LengthUnsupported => "a 64-bit length is not accepted",
            Self::LengthNotMinimal => "an extended length encodes a value that fits the short form",
            Self::TailLength => "the header tail does not match its length marker",
            Self::TooLarge => "the payload is over the frame ceiling",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub opcode: Opcode,
    pub is_final: bool,
    pub payload_len: usize,
    pub mask: [u8; MASK_KEY_BYTES],
}

pub fn header_tail_len(prelude: [u8; PRELUDE_BYTES]) -> Result<usize, FrameError> {
    if prelude[0] & RSV_BITS != 0 {
        return Err(FrameError::ReservedBits);
    }
    if Opcode::from_code(prelude[0] & OPCODE_BITS).is_none() {
        return Err(FrameError::ReservedOpcode);
    }
    if prelude[1] & MASK_BIT == 0 {
        return Err(FrameError::Unmasked);
    }
    match prelude[1] & LEN7_BITS {
        LEN_IS_64_BIT => Err(FrameError::LengthUnsupported),
        LEN_IS_16_BIT => Ok(EXTENDED_LEN_BYTES + MASK_KEY_BYTES),
        _ => Ok(MASK_KEY_BYTES),
    }
}

pub fn parse_header(
    prelude: [u8; PRELUDE_BYTES],
    tail: &[u8],
) -> Result<FrameHeader, FrameError> {
    let tail_len = header_tail_len(prelude)?;
    if tail.len() != tail_len {
        return Err(FrameError::TailLength);
    }
    let len7 = prelude[1] & LEN7_BITS;
    let (payload_len, mask) = split_tail(len7, tail);
    if len7 == LEN_IS_16_BIT && payload_len <= MAX_SHORT_LEN {
        return Err(FrameError::LengthNotMinimal);
    }
    let opcode = Opcode::from_code(prelude[0] & OPCODE_BITS).ok_or(FrameError::ReservedOpcode)?;
    let is_final = prelude[0] & FIN_BIT != 0;
    validate(opcode, is_final, payload_len)?;
    Ok(FrameHeader {
        opcode,
        is_final,
        payload_len,
        mask,
    })
}

fn split_tail(len7: u8, tail: &[u8]) -> (usize, [u8; MASK_KEY_BYTES]) {
    let (payload_len, key) = if len7 == LEN_IS_16_BIT {
        (
            usize::from(u16::from_be_bytes([tail[0], tail[1]])),
            &tail[EXTENDED_LEN_BYTES..],
        )
    } else {
        (usize::from(len7), tail)
    };
    let mut mask = [0u8; MASK_KEY_BYTES];
    mask.copy_from_slice(key);
    (payload_len, mask)
}

fn validate(opcode: Opcode, is_final: bool, payload_len: usize) -> Result<(), FrameError> {
    if opcode.is_control() {
        if payload_len > MAX_CONTROL_PAYLOAD {
            return Err(FrameError::ControlTooLong);
        }
        if !is_final {
            return Err(FrameError::ControlFragmented);
        }
        if matches!(opcode, Opcode::Close) && payload_len == 1 {
            return Err(FrameError::CloseTooShort);
        }
    } else if !is_final || matches!(opcode, Opcode::Continuation) {
        return Err(FrameError::Fragmented);
    }
    if payload_len > MAX_CLIENT_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(())
}

pub fn unmask(mask: [u8; MASK_KEY_BYTES], payload: &mut [u8]) {
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % MASK_KEY_BYTES];
    }
}

pub fn server_header(
    opcode: Opcode,
    payload_len: usize,
    out: &mut [u8; MAX_SERVER_HEADER_BYTES],
) -> Option<usize> {
    if matches!(opcode, Opcode::Continuation) {
        return None;
    }
    if opcode.is_control() && payload_len > MAX_CONTROL_PAYLOAD {
        return None;
    }
    out[0] = FIN_BIT | opcode.code();
    if payload_len <= MAX_SHORT_LEN {
        out[1] = payload_len as u8;
        return Some(PRELUDE_BYTES);
    }
    let len = u16::try_from(payload_len).ok()?;
    out[1] = LEN_IS_16_BIT;
    out[PRELUDE_BYTES..].copy_from_slice(&len.to_be_bytes());
    Some(MAX_SERVER_HEADER_BYTES)
}

pub fn close_frame(payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut header = [0u8; MAX_SERVER_HEADER_BYTES];
    let header_len = server_header(Opcode::Close, payload.len(), &mut header)?;
    let total = header_len.checked_add(payload.len())?;
    if out.len() < total {
        return None;
    }
    out[..header_len].copy_from_slice(&header[..header_len]);
    out[header_len..total].copy_from_slice(payload);
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; MASK_KEY_BYTES] = [0xa1, 0xb2, 0xc3, 0xd4];

    fn prelude(first: u8, second: u8) -> [u8; PRELUDE_BYTES] {
        [first, second]
    }

    fn masked_short(opcode: u8, len: u8) -> [u8; PRELUDE_BYTES] {
        prelude(FIN_BIT | opcode, MASK_BIT | len)
    }

    #[test]
    fn a_short_masked_text_frame_parses_to_its_length_and_key() {
        let header = parse_header(masked_short(OPCODE_TEXT, 5), &KEY).expect("a valid frame");
        assert_eq!(header.opcode, Opcode::Text);
        assert!(header.is_final);
        assert_eq!(header.payload_len, 5);
        assert_eq!(header.mask, KEY);
    }

    #[test]
    fn a_sixteen_bit_length_is_read_from_the_tail_before_the_key() {
        let tail = [0x01, 0x40, KEY[0], KEY[1], KEY[2], KEY[3]];
        let header = parse_header(masked_short(OPCODE_TEXT, LEN_IS_16_BIT), &tail)
            .expect("a 320-byte frame is under the ceiling");
        assert_eq!(header.payload_len, 320);
        assert_eq!(header.mask, KEY);
    }

    #[test]
    fn the_tail_length_follows_the_length_marker() {
        assert_eq!(
            header_tail_len(masked_short(OPCODE_TEXT, 7)).expect("a short frame"),
            MASK_KEY_BYTES
        );
        assert_eq!(
            header_tail_len(masked_short(OPCODE_TEXT, LEN_IS_16_BIT)).expect("an extended frame"),
            2 + MASK_KEY_BYTES
        );
    }

    #[test]
    fn an_unmasked_client_frame_is_refused() {
        let unmasked = prelude(FIN_BIT | OPCODE_TEXT, 5);
        assert_eq!(header_tail_len(unmasked), Err(FrameError::Unmasked));
        assert_eq!(parse_header(unmasked, &KEY), Err(FrameError::Unmasked));
    }

    #[test]
    fn a_reserved_bit_is_refused() {
        let reserved = prelude(FIN_BIT | 0x10 | OPCODE_TEXT, MASK_BIT | 5);
        assert_eq!(header_tail_len(reserved), Err(FrameError::ReservedBits));
    }

    #[test]
    fn a_reserved_opcode_is_refused_rather_than_panicking() {
        for code in [0x3, 0x4, 0x5, 0x6, 0x7, 0xb, 0xc, 0xd, 0xe, 0xf] {
            let frame = masked_short(code, 1);
            assert_eq!(
                header_tail_len(frame),
                Err(FrameError::ReservedOpcode),
                "opcode {code:#x} was accepted"
            );
        }
    }

    #[test]
    fn a_sixty_four_bit_length_is_refused_before_its_tail_is_read() {
        assert_eq!(
            header_tail_len(masked_short(OPCODE_TEXT, LEN_IS_64_BIT)),
            Err(FrameError::LengthUnsupported)
        );
    }

    #[test]
    fn a_control_frame_over_125_bytes_is_refused() {
        let tail = [0x00, 0x7e, KEY[0], KEY[1], KEY[2], KEY[3]];
        assert_eq!(
            parse_header(masked_short(OPCODE_PING, LEN_IS_16_BIT), &tail),
            Err(FrameError::ControlTooLong)
        );
    }

    #[test]
    fn a_fragmented_control_frame_is_refused() {
        let fragmented = prelude(OPCODE_CLOSE, MASK_BIT | 2);
        assert_eq!(
            parse_header(fragmented, &KEY),
            Err(FrameError::ControlFragmented)
        );
    }

    #[test]
    fn a_non_final_data_frame_is_refused_rather_than_taken_for_a_whole_one() {
        let started = prelude(OPCODE_TEXT, MASK_BIT | 3);
        assert_eq!(parse_header(started, &KEY), Err(FrameError::Fragmented));
        let binary = prelude(OPCODE_BINARY, MASK_BIT | 3);
        assert_eq!(parse_header(binary, &KEY), Err(FrameError::Fragmented));
    }

    #[test]
    fn a_continuation_frame_is_refused_since_no_fragment_is_ever_started() {
        for first in [OPCODE_CONTINUATION, FIN_BIT | OPCODE_CONTINUATION] {
            assert_eq!(
                parse_header(prelude(first, MASK_BIT | 3), &KEY),
                Err(FrameError::Fragmented),
                "a continuation frame was accepted with first byte {first:#x}"
            );
        }
    }

    #[test]
    fn a_final_data_frame_is_still_accepted() {
        assert_eq!(
            parse_header(masked_short(OPCODE_TEXT, 3), &KEY)
                .expect("a final text frame")
                .opcode,
            Opcode::Text
        );
        assert_eq!(
            parse_header(masked_short(OPCODE_BINARY, 3), &KEY)
                .expect("a final binary frame")
                .opcode,
            Opcode::Binary
        );
    }

    #[test]
    fn a_payload_over_the_ceiling_is_refused() {
        let over = MAX_CLIENT_FRAME_BYTES as u16 + 1;
        let bytes = over.to_be_bytes();
        let tail = [bytes[0], bytes[1], KEY[0], KEY[1], KEY[2], KEY[3]];
        assert_eq!(
            parse_header(masked_short(OPCODE_TEXT, LEN_IS_16_BIT), &tail),
            Err(FrameError::TooLarge)
        );
    }

    #[test]
    fn the_ceiling_itself_is_accepted() {
        let bytes = (MAX_CLIENT_FRAME_BYTES as u16).to_be_bytes();
        let tail = [bytes[0], bytes[1], KEY[0], KEY[1], KEY[2], KEY[3]];
        let header = parse_header(masked_short(OPCODE_TEXT, LEN_IS_16_BIT), &tail)
            .expect("the ceiling is a legal length");
        assert_eq!(header.payload_len, MAX_CLIENT_FRAME_BYTES);
    }

    #[test]
    fn a_tail_of_the_wrong_length_is_refused_rather_than_indexed() {
        assert_eq!(
            parse_header(masked_short(OPCODE_TEXT, LEN_IS_16_BIT), &KEY),
            Err(FrameError::TailLength),
            "an extended frame needs six tail bytes, not four"
        );
        assert_eq!(
            parse_header(masked_short(OPCODE_TEXT, 4), &KEY[..3]),
            Err(FrameError::TailLength)
        );
    }

    #[test]
    fn unmasking_is_its_own_inverse() {
        let plain = b"subscribe to the lamps".to_vec();
        let mut wire = plain.clone();
        unmask(KEY, &mut wire);
        assert_ne!(wire, plain, "the mask left the payload untouched");
        unmask(KEY, &mut wire);
        assert_eq!(wire, plain);
    }

    #[test]
    fn unmasking_walks_the_key_by_payload_offset() {
        let mut one = [0u8; 5];
        unmask(KEY, &mut one);
        assert_eq!(one, [KEY[0], KEY[1], KEY[2], KEY[3], KEY[0]]);
    }

    #[test]
    fn a_server_frame_carries_no_mask_and_grows_its_header_by_length() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        assert_eq!(server_header(Opcode::Text, 5, &mut out), Some(2));
        assert_eq!(out[0], FIN_BIT | OPCODE_TEXT);
        assert_eq!(out[1] & MASK_BIT, 0, "a server frame must not be masked");
        assert_eq!(out[1], 5);

        assert_eq!(server_header(Opcode::Text, 320, &mut out), Some(4));
        assert_eq!(out[1], LEN_IS_16_BIT);
        assert_eq!(u16::from_be_bytes([out[2], out[3]]), 320);
    }

    #[test]
    fn the_server_header_switches_marker_exactly_at_126() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        assert_eq!(server_header(Opcode::Text, 125, &mut out), Some(2));
        assert_eq!(out[1], 125);
        assert_eq!(server_header(Opcode::Text, 126, &mut out), Some(4));
        assert_eq!(out[1], LEN_IS_16_BIT);
    }

    #[test]
    fn a_payload_past_a_sixteen_bit_length_is_refused_rather_than_truncated() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        assert_eq!(
            server_header(Opcode::Text, usize::from(u16::MAX), &mut out),
            Some(4)
        );
        assert_eq!(
            server_header(Opcode::Text, usize::from(u16::MAX) + 1, &mut out),
            None,
            "a frame needing a 64-bit length must fail, not wrap its header"
        );
    }

    #[test]
    fn a_whole_close_frame_is_its_header_and_its_code() {
        let mut out = [0u8; 8];
        let len = close_frame(&1008u16.to_be_bytes(), &mut out).expect("a close frame");
        assert_eq!(&out[..len], &[0x88, 0x02, 0x03, 0xf0]);
    }

    #[test]
    fn a_close_frame_that_does_not_fit_is_refused_rather_than_truncated() {
        let mut out = [0u8; 3];
        assert_eq!(close_frame(&1008u16.to_be_bytes(), &mut out), None);
    }

    #[test]
    fn a_close_frame_with_no_payload_is_two_bytes() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        assert_eq!(server_header(Opcode::Close, 0, &mut out), Some(2));
        assert_eq!(out[0], FIN_BIT | OPCODE_CLOSE);
        assert_eq!(out[1], 0);
    }

    #[test]
    fn every_error_says_something_of_its_own() {
        let all = [
            FrameError::Unmasked,
            FrameError::ReservedBits,
            FrameError::ReservedOpcode,
            FrameError::ControlTooLong,
            FrameError::ControlFragmented,
            FrameError::CloseTooShort,
            FrameError::Fragmented,
            FrameError::LengthUnsupported,
            FrameError::LengthNotMinimal,
            FrameError::TailLength,
            FrameError::TooLarge,
        ];
        let mut seen: Vec<String> = Vec::new();
        for error in all {
            let said = error.to_string();
            assert!(!said.is_empty(), "{error:?} says nothing");
            assert!(!seen.contains(&said), "{error:?} repeats another message");
            seen.push(said);
        }
    }

    #[test]
    fn a_control_frame_of_exactly_125_bytes_is_accepted() {
        let header = parse_header(masked_short(OPCODE_PING, MAX_CONTROL_PAYLOAD as u8), &KEY)
            .expect("125 is the control ceiling, not past it");
        assert_eq!(header.payload_len, MAX_CONTROL_PAYLOAD);
    }

    #[test]
    fn every_reserved_bit_is_refused_not_only_the_first() {
        for bit in [0x10, 0x20, 0x40] {
            assert_eq!(
                header_tail_len(prelude(FIN_BIT | bit | OPCODE_TEXT, MASK_BIT | 5)),
                Err(FrameError::ReservedBits),
                "reserved bit {bit:#x} was accepted"
            );
        }
    }

    #[test]
    fn a_pong_counts_as_a_control_frame() {
        let tail = [0x00, 0x7e, KEY[0], KEY[1], KEY[2], KEY[3]];
        assert_eq!(
            parse_header(masked_short(OPCODE_PONG, LEN_IS_16_BIT), &tail),
            Err(FrameError::ControlTooLong),
            "an oversized pong must be refused like any control frame"
        );
    }

    #[test]
    fn a_close_frame_carrying_one_byte_is_refused() {
        assert_eq!(
            parse_header(masked_short(OPCODE_CLOSE, 1), &KEY),
            Err(FrameError::CloseTooShort)
        );
        assert!(
            parse_header(masked_short(OPCODE_CLOSE, 0), &KEY).is_ok(),
            "an empty close is legal"
        );
        assert!(
            parse_header(masked_short(OPCODE_CLOSE, 2), &KEY).is_ok(),
            "a close with a code is legal"
        );
    }

    #[test]
    fn an_extended_length_that_fits_the_short_form_is_refused() {
        for small in [0u16, 5, MAX_SHORT_LEN as u16] {
            let bytes = small.to_be_bytes();
            let tail = [bytes[0], bytes[1], KEY[0], KEY[1], KEY[2], KEY[3]];
            assert_eq!(
                parse_header(masked_short(OPCODE_TEXT, LEN_IS_16_BIT), &tail),
                Err(FrameError::LengthNotMinimal),
                "a 126 marker carrying {small} is not the minimal encoding"
            );
        }
    }

    #[test]
    fn a_tail_longer_than_its_marker_is_refused_as_well_as_a_shorter_one() {
        let long = [KEY[0], KEY[1], KEY[2], KEY[3], 0x00];
        assert_eq!(
            parse_header(masked_short(OPCODE_TEXT, 4), &long),
            Err(FrameError::TailLength),
            "a short frame takes four tail bytes, not five"
        );
    }

    #[test]
    fn the_encoder_spells_each_opcode_it_is_asked_for() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        for (opcode, code) in [
            (Opcode::Text, OPCODE_TEXT),
            (Opcode::Binary, OPCODE_BINARY),
            (Opcode::Ping, OPCODE_PING),
            (Opcode::Pong, OPCODE_PONG),
            (Opcode::Close, OPCODE_CLOSE),
        ] {
            assert_eq!(server_header(opcode, 1, &mut out), Some(PRELUDE_BYTES));
            assert_eq!(
                out[0],
                FIN_BIT | code,
                "{opcode:?} was written as {:#x}",
                out[0]
            );
        }
    }

    #[test]
    fn the_encoder_refuses_an_oversized_control_frame_and_a_continuation() {
        let mut out = [0u8; MAX_SERVER_HEADER_BYTES];
        for control in [Opcode::Close, Opcode::Ping, Opcode::Pong] {
            assert_eq!(
                server_header(control, MAX_CONTROL_PAYLOAD + 1, &mut out),
                None,
                "{control:?} may not carry more than 125 bytes"
            );
            assert_eq!(
                server_header(control, MAX_CONTROL_PAYLOAD, &mut out),
                Some(PRELUDE_BYTES)
            );
        }
        assert_eq!(
            server_header(Opcode::Continuation, 1, &mut out),
            None,
            "nothing here sends fragments, so a continuation must not be encodable"
        );
    }
}
