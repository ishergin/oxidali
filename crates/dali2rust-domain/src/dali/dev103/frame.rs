use crate::dali::dev103::address::{Device103Address, InstanceAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardFrame24 {
    bytes: [u8; 3],
}

impl ForwardFrame24 {
    #[must_use]
    pub const fn command(address: Device103Address, instance: InstanceAddress, opcode: u8) -> Self {
        Self {
            bytes: [address.encode(), instance.encode(), opcode],
        }
    }

    #[must_use]
    pub const fn special(selector: u8, data: u8) -> Self {
        Self {
            bytes: [
                Device103Address::Special(0).encode(),
                selector,
                data,
            ],
        }
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 3]) -> Self {
        Self { bytes }
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 3] {
        self.bytes
    }

    #[must_use]
    pub const fn is_command(self) -> bool {
        self.bytes[0] & 0x01 == 0x01
    }

    #[must_use]
    pub const fn address(self) -> Option<Device103Address> {
        Device103Address::decode(self.bytes[0])
    }

    #[must_use]
    pub const fn instance(self) -> Option<InstanceAddress> {
        InstanceAddress::decode(self.bytes[1])
    }

    #[must_use]
    pub const fn opcode(self) -> u8 {
        self.bytes[2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_frame_sets_bit_16_and_an_event_frame_does_not() {
        let cmd = ForwardFrame24::command(
            Device103Address::Short(3),
            InstanceAddress::Number(0),
            0x80,
        );
        assert!(cmd.is_command());
        let mut event_bytes = cmd.as_bytes();
        event_bytes[0] &= !0x01;
        assert!(!ForwardFrame24::from_bytes(event_bytes).is_command());
    }

    #[test]
    fn a_device_command_carries_0xfe_in_the_instance_byte() {
        let frame = ForwardFrame24::command(
            Device103Address::Short(7),
            InstanceAddress::Device,
            0x30,
        );
        assert_eq!(frame.as_bytes(), [0b0000_1111, 0xFE, 0x30]);
        assert_eq!(frame.instance(), Some(InstanceAddress::Device));
    }

    #[test]
    fn special_frames_start_at_0xc1() {
        assert_eq!(ForwardFrame24::special(0x00, 0x00).as_bytes(), [0xC1, 0x00, 0x00]);
        assert_eq!(ForwardFrame24::special(0x01, 0xFF).as_bytes(), [0xC1, 0x01, 0xFF]);
    }

    #[test]
    fn raw_bytes_round_trip() {
        let bytes = [0x81u8, 0xC3, 0x1F];
        assert_eq!(ForwardFrame24::from_bytes(bytes).as_bytes(), bytes);
    }
}
