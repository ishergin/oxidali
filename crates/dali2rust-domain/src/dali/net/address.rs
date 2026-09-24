#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DaliAddress {
    Short(u8),
    Group(u8),
    Broadcast,
    BroadcastUnaddressed,
}

impl DaliAddress {
    pub fn short(addr: u8) -> Result<Self, DaliAddressError> {
        if addr > 63 {
            return Err(DaliAddressError::OutOfRange { max: 63, got: addr });
        }
        Ok(Self::Short(addr))
    }

    pub fn group(addr: u8) -> Result<Self, DaliAddressError> {
        if addr > 15 {
            return Err(DaliAddressError::OutOfRange { max: 15, got: addr });
        }
        Ok(Self::Group(addr))
    }

    pub fn encode_address_byte(&self) -> u8 {
        match self {
            Self::Short(a) => {
                debug_assert!(*a <= 63, "short address out of range: {}", a);
                (a & 0x3F) << 1
            }
            Self::Group(a) => {
                debug_assert!(*a <= 15, "group address out of range: {}", a);
                ((a & 0x0F) << 1) | 0x80
            }
            Self::Broadcast => 0xFE,
            Self::BroadcastUnaddressed => 0xFC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaliAddressError {
    OutOfRange { max: u8, got: u8 },
}

pub fn decode_wire_address(wire: u8) -> Result<DaliAddress, DaliAddressError> {
    match wire {
        0xFE | 0xFF => Ok(DaliAddress::Broadcast),
        0xFC | 0xFD => Ok(DaliAddress::BroadcastUnaddressed),
        w if (w & 0xE0) == 0x80 => {
            let group = (w >> 1) & 0x0F;
            DaliAddress::group(group)
        }
        w if (w & 0x80) == 0 => {
            let short = (w >> 1) & 0x3F;
            DaliAddress::short(short)
        }
        _ => Err(DaliAddressError::OutOfRange { max: 0, got: wire }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_address_valid() {
        assert_eq!(DaliAddress::short(0).unwrap(), DaliAddress::Short(0));
        assert_eq!(DaliAddress::short(63).unwrap(), DaliAddress::Short(63));
    }

    #[test]
    fn short_address_out_of_range() {
        assert_eq!(
            DaliAddress::short(64),
            Err(DaliAddressError::OutOfRange { max: 63, got: 64 })
        );
    }

    #[test]
    fn group_address_valid() {
        assert_eq!(DaliAddress::group(0).unwrap(), DaliAddress::Group(0));
        assert_eq!(DaliAddress::group(15).unwrap(), DaliAddress::Group(15));
    }

    #[test]
    fn group_address_out_of_range() {
        assert_eq!(
            DaliAddress::group(16),
            Err(DaliAddressError::OutOfRange { max: 15, got: 16 })
        );
    }

    #[test]
    fn encode_short_address() {
        assert_eq!(DaliAddress::short(0).unwrap().encode_address_byte(), 0x00);
        assert_eq!(DaliAddress::short(1).unwrap().encode_address_byte(), 0x02);
        assert_eq!(DaliAddress::short(63).unwrap().encode_address_byte(), 0x7E);
    }

    #[test]
    fn encode_group_address() {
        assert_eq!(DaliAddress::group(0).unwrap().encode_address_byte(), 0x80);
        assert_eq!(DaliAddress::group(1).unwrap().encode_address_byte(), 0x82);
        assert_eq!(DaliAddress::group(15).unwrap().encode_address_byte(), 0x9E);
    }

    #[test]
    fn encode_broadcast() {
        assert_eq!(DaliAddress::Broadcast.encode_address_byte(), 0xFE);
        assert_eq!(
            DaliAddress::BroadcastUnaddressed.encode_address_byte(),
            0xFC
        );
    }

    #[test]
    fn decode_wire_address_all_forms() {
        assert_eq!(
            decode_wire_address(0x00).unwrap(),
            DaliAddress::short(0).unwrap()
        );
        assert_eq!(
            decode_wire_address(0x7E).unwrap(),
            DaliAddress::short(63).unwrap()
        );
        assert_eq!(
            decode_wire_address(0x80).unwrap(),
            DaliAddress::group(0).unwrap()
        );
        assert_eq!(
            decode_wire_address(0x9E).unwrap(),
            DaliAddress::group(15).unwrap()
        );
        assert_eq!(decode_wire_address(0xFE).unwrap(), DaliAddress::Broadcast);
        assert_eq!(
            decode_wire_address(0xFC).unwrap(),
            DaliAddress::BroadcastUnaddressed
        );
        assert_eq!(
            decode_wire_address(0x01).unwrap(),
            DaliAddress::short(0).unwrap()
        );
        assert_eq!(
            decode_wire_address(0x0B).unwrap(),
            DaliAddress::short(5).unwrap()
        );
        assert!(decode_wire_address(0xA1).is_err());
        assert_eq!(decode_wire_address(0xFF).unwrap(), DaliAddress::Broadcast);
    }
}
