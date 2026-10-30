#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardFrame(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackwardFrame(pub u8);

impl ForwardFrame {
    pub const DATA_BITS: u8 = 16;

    pub fn new(address_byte: u8, command_byte: u8) -> Self {
        Self(((address_byte as u16) << 8) | (command_byte as u16))
    }

    pub fn address_byte(&self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn command_byte(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub fn raw(&self) -> u16 {
        self.0
    }

    pub fn is_direct_arc_power(&self) -> bool {
        (self.address_byte() & 0x01) == 0
    }
}

impl BackwardFrame {
    pub fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn raw(&self) -> u8 {
        self.0
    }

    pub fn is_nack(&self) -> bool {
        false
    }

    pub fn is_valid(&self) -> bool {
        true
    }
}

// IEC 62386-101 Table 16
const HALF_BITS_PER_SECOND: u32 = 2_400;
const HALF_BITS_PER_BIT: u32 = 2;
const US_PER_SECOND: u32 = 1_000_000;
const US_PER_MS: u32 = 1_000;
// IEC 62386-101 Table 20
pub const SEND_TWICE_SEPARATE_US: u32 = 105_000;

#[must_use]
pub const fn forward_frame_us(data_bits: u8) -> u32 {
    (data_bits as u32 + 1) * HALF_BITS_PER_BIT * US_PER_SECOND / HALF_BITS_PER_SECOND
}

// IEC 62386-101 §9.3
#[must_use]
pub const fn send_twice_span_ms(data_bits: u8) -> u32 {
    (SEND_TWICE_SEPARATE_US + forward_frame_us(data_bits)).div_ceil(US_PER_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    BackwardTimeout,
    BackwardNack,
    Collision,
    BusBusy,
    TransportError,
    Preempted,
    Held(HoldCause),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldCause {
    Passive,
    Quiescent,
}

impl HoldCause {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Passive => "controller_passive",
            Self::Quiescent => "controller_quiescent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_spans_its_settling_limit_plus_the_second_frame() {
        assert_eq!(forward_frame_us(24), 20_833);
        assert_eq!(forward_frame_us(16), 14_166);
        assert_eq!(send_twice_span_ms(24), 126);
        assert_eq!(send_twice_span_ms(16), 120);
    }

    #[test]
    fn forward_frame_new() {
        let f = ForwardFrame::new(0x02, 0xFE);
        assert_eq!(f.raw(), 0x02FE);
        assert_eq!(f.address_byte(), 0x02);
        assert_eq!(f.command_byte(), 0xFE);
    }

    #[test]
    fn forward_frame_direct_arc() {
        let f = ForwardFrame::new(0x00, 0xFE);
        assert!(f.is_direct_arc_power());
    }

    #[test]
    fn forward_frame_not_direct_arc() {
        let f = ForwardFrame::new(0x01, 0x05);
        assert!(!f.is_direct_arc_power());
    }

    #[test]
    fn backward_frame_valid() {
        let b = BackwardFrame::new(0x80);
        assert!(b.is_valid());
        assert!(!b.is_nack());
        assert_eq!(b.raw(), 0x80);
    }

    #[test]
    fn backward_frame_ff_is_valid_data() {
        let b = BackwardFrame::new(0xFF);
        assert!(b.is_valid());
        assert!(!b.is_nack());
    }

    #[test]
    fn forward_frame_roundtrip() {
        let addr = 0x7E;
        let cmd = 0x05;
        let f = ForwardFrame::new(addr, cmd);
        assert_eq!(f.address_byte(), addr);
        assert_eq!(f.command_byte(), cmd);
        let f2 = ForwardFrame(f.raw());
        assert_eq!(f2.address_byte(), addr);
        assert_eq!(f2.command_byte(), cmd);
    }

    #[test]
    fn frame_error_variants_are_distinct() {
        assert_ne!(FrameError::BackwardTimeout, FrameError::BackwardNack);
        assert_ne!(FrameError::BackwardTimeout, FrameError::Collision);
        assert_ne!(FrameError::BackwardTimeout, FrameError::BusBusy);
        assert_ne!(FrameError::BackwardTimeout, FrameError::TransportError);
        assert_ne!(FrameError::Collision, FrameError::BusBusy);
    }

    #[test]
    fn frame_error_debug_includes_variant_name() {
        let s = format!("{:?}", FrameError::Collision);
        assert!(s.contains("Collision"));
        let s = format!("{:?}", FrameError::BusBusy);
        assert!(s.contains("BusBusy"));
        let s = format!("{:?}", FrameError::TransportError);
        assert!(s.contains("TransportError"));
    }
}
