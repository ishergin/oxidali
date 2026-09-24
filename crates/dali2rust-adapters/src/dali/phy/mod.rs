use dali2rust_dali_codec::rx_decode::SniffedFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedFrame {
    Forward16(u16),
    Backward8(u8),
    Forward24([u8; 3]),
    DecodeFailed,
}

impl DecodedFrame {
    pub fn from_sniffed(frame: SniffedFrame) -> Self {
        match frame {
            SniffedFrame::Forward16(v) => Self::Forward16(v),
            SniffedFrame::Backward8(v) => Self::Backward8(v),
            SniffedFrame::Forward24(b) => Self::Forward24(b),
            SniffedFrame::UnsupportedLength(_) | SniffedFrame::DecodeFailed => Self::DecodeFailed,
        }
    }

    pub const fn bit_len(&self) -> u8 {
        match self {
            DecodedFrame::Forward16(_) => 16,
            DecodedFrame::Backward8(_) => 8,
            DecodedFrame::Forward24(_) => 24,
            DecodedFrame::DecodeFailed => 0,
        }
    }

    pub const fn is_forward(&self) -> bool {
        matches!(
            self,
            DecodedFrame::Forward16(_) | DecodedFrame::Forward24(_)
        )
    }

    pub const fn is_backward(&self) -> bool {
        matches!(self, DecodedFrame::Backward8(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_frame_bit_len() {
        assert_eq!(DecodedFrame::Forward16(0).bit_len(), 16);
        assert_eq!(DecodedFrame::Backward8(0).bit_len(), 8);
        assert_eq!(DecodedFrame::Forward24([0; 3]).bit_len(), 24);
        assert_eq!(DecodedFrame::DecodeFailed.bit_len(), 0);
    }

    #[test]
    fn decoded_frame_is_forward() {
        assert!(DecodedFrame::Forward16(0x1234).is_forward());
        assert!(DecodedFrame::Forward24([0; 3]).is_forward());
        assert!(!DecodedFrame::Backward8(0x55).is_forward());
        assert!(!DecodedFrame::DecodeFailed.is_forward());
    }

    #[test]
    fn decoded_frame_is_backward() {
        assert!(DecodedFrame::Backward8(0xFF).is_backward());
        assert!(!DecodedFrame::Forward16(0).is_backward());
        assert!(!DecodedFrame::Forward24([0; 3]).is_backward());
        assert!(!DecodedFrame::DecodeFailed.is_backward());
    }

    #[test]
    fn from_sniffed_maps_supported_frames() {
        assert_eq!(
            DecodedFrame::from_sniffed(SniffedFrame::Forward16(0x1234)),
            DecodedFrame::Forward16(0x1234)
        );
        assert_eq!(
            DecodedFrame::from_sniffed(SniffedFrame::UnsupportedLength(20)),
            DecodedFrame::DecodeFailed
        );
    }
}
