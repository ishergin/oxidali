use crate::dali::devices::dt6_led::Dt6Command;
use crate::dali::devices::dt8_color::Dt8Command;
use crate::dali::devices::{DeviceCommandMetadata, DeviceType};
use crate::dali::frame::ForwardFrame;
use crate::dali::net::address::DaliAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedCommand {
    Dt6(Dt6Command),
    Dt8(Dt8Command),
}

impl ExtendedCommand {
    pub fn from_opcode(opcode: u8) -> Option<Self> {
        Dt6Command::from_opcode(opcode)
            .map(Self::Dt6)
            .or_else(|| Dt8Command::from_opcode(opcode).map(Self::Dt8))
    }

    pub fn to_forward_frame(&self, address: &DaliAddress) -> ForwardFrame {
        match self {
            Self::Dt6(c) => c.to_forward_frame(address),
            Self::Dt8(c) => c.to_forward_frame(address),
        }
    }

    pub fn is_query(&self) -> bool {
        match self {
            Self::Dt6(c) => c.is_query(),
            Self::Dt8(c) => c.is_query(),
        }
    }

    pub fn requires_repeat(&self) -> bool {
        match self {
            Self::Dt6(c) => c.requires_repeat(),
            Self::Dt8(c) => c.requires_repeat(),
        }
    }

    pub fn enable_device_type(&self) -> DeviceType {
        let md = match self {
            Self::Dt6(c) => c.metadata(),
            Self::Dt8(c) => c.metadata(),
        };
        md.device_type.unwrap_or(DeviceType::Led)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::devices::dt6_led::Dt6Command;
    use crate::dali::devices::dt8_color::Dt8Command;

    #[test]
    fn from_opcode_resolves_the_shared_query_block_as_dt6() {
        let cmd = ExtendedCommand::from_opcode(0xFA).expect("shared opcode");
        assert!(matches!(cmd, ExtendedCommand::Dt6(_)));

        let dt8 = ExtendedCommand::Dt8(Dt8Command::QueryColourValue);
        assert_eq!(dt8.to_forward_frame(&DaliAddress::short(1).unwrap()).command_byte(), 0xFA);
        assert!(dt8.is_query());
        assert_eq!(dt8.enable_device_type(), DeviceType::Color);
    }

    #[test]
    fn from_opcode_maps_dt6_before_dt8_range() {
        let cmd = ExtendedCommand::from_opcode(0xED).expect("DT6 opcode");
        assert!(matches!(
            cmd,
            ExtendedCommand::Dt6(Dt6Command::QueryGearType)
        ));
        assert!(cmd.is_query());
        assert_eq!(cmd.enable_device_type(), DeviceType::Led);
    }
}
