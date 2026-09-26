use crate::dali::devices::dt6_led::Dt6Command;
use crate::dali::devices::dt8_color::Dt8Command;
use crate::dali::devices::{DeviceCommandMetadata, DeviceType};
use crate::dali::frame::ForwardFrame;
use crate::dali::net::address::DaliAddress;

// IEC 62386-102 §11.6.2
pub const QUERY_EXTENDED_VERSION_NUMBER_OPCODE: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedCommand {
    Dt6(Dt6Command),
    Dt8(Dt8Command),
    ExtendedVersion { device_type: u8 },
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
            Self::ExtendedVersion { .. } => ForwardFrame::new(
                address.encode_address_byte() | 0x01,
                QUERY_EXTENDED_VERSION_NUMBER_OPCODE,
            ),
        }
    }

    pub fn is_query(&self) -> bool {
        match self {
            Self::Dt6(c) => c.is_query(),
            Self::Dt8(c) => c.is_query(),
            Self::ExtendedVersion { .. } => true,
        }
    }

    pub fn requires_repeat(&self) -> bool {
        match self {
            Self::Dt6(c) => c.requires_repeat(),
            Self::Dt8(c) => c.requires_repeat(),
            Self::ExtendedVersion { .. } => false,
        }
    }

    // IEC 62386-102 §9.18
    pub fn enable_device_type(&self) -> u8 {
        let md = match self {
            Self::Dt6(c) => c.metadata(),
            Self::Dt8(c) => c.metadata(),
            Self::ExtendedVersion { device_type } => return *device_type,
        };
        md.device_type.unwrap_or(DeviceType::Led).code()
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
        assert_eq!(dt8.enable_device_type(), DeviceType::Color.code());
    }

    #[test]
    fn from_opcode_maps_dt6_before_dt8_range() {
        let cmd = ExtendedCommand::from_opcode(0xED).expect("DT6 opcode");
        assert!(matches!(
            cmd,
            ExtendedCommand::Dt6(Dt6Command::QueryGearType)
        ));
        assert!(cmd.is_query());
        assert_eq!(cmd.enable_device_type(), DeviceType::Led.code());
    }

    #[test]
    fn an_extended_version_query_goes_behind_the_type_it_names() {
        const DIAGNOSTICS: u8 = 52;
        let cmd = ExtendedCommand::ExtendedVersion { device_type: DIAGNOSTICS };
        let frame = cmd.to_forward_frame(&DaliAddress::short(3).unwrap());
        assert_eq!(frame.address_byte(), 0x07);
        assert_eq!(frame.command_byte(), QUERY_EXTENDED_VERSION_NUMBER_OPCODE);
        assert!(cmd.is_query() && !cmd.requires_repeat());
        assert_eq!(
            cmd.enable_device_type(),
            DIAGNOSTICS,
            "IEC 62386-102 §9.18: the prelude names the type whose version is asked"
        );
    }
}
