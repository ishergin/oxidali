use crate::dali::frame::ForwardFrame;
use crate::dali::types::DaliAddress;

pub mod dt6_led;
pub mod dt8_color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum DeviceType {
    Fluorescent = 0,
    Emergency = 1,
    Discharge = 2,
    LowVoltageHalogen = 3,
    Incandescent = 4,
    DcConverter = 5,
    Led = 6,
    Switching = 7,
    Color = 8,
}

impl DeviceType {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Fluorescent),
            1 => Some(Self::Emergency),
            2 => Some(Self::Discharge),
            3 => Some(Self::LowVoltageHalogen),
            4 => Some(Self::Incandescent),
            5 => Some(Self::DcConverter),
            6 => Some(Self::Led),
            7 => Some(Self::Switching),
            8 => Some(Self::Color),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMetadata {
    pub device_type: Option<DeviceType>,
    pub opcode: u8,
    pub expects_backward: bool,
    pub requires_repeat: bool,
}

pub trait DeviceCommandMetadata {
    fn metadata(&self) -> CommandMetadata;

    fn opcode(&self) -> u8 {
        self.metadata().opcode
    }

    fn expects_backward(&self) -> bool {
        self.metadata().expects_backward
    }

    fn requires_repeat(&self) -> bool {
        self.metadata().requires_repeat
    }

    fn is_query(&self) -> bool {
        self.expects_backward()
    }

    fn to_forward_frame(&self, address: &DaliAddress) -> ForwardFrame {
        let addr_byte = address.encode_address_byte() | 0x01;
        ForwardFrame::new(addr_byte, self.opcode())
    }
}
