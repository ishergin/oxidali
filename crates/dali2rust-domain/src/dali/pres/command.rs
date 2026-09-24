use crate::dali::frame::{BackwardFrame, ForwardFrame};
use crate::dali::pres::extended::ExtendedCommand;
use crate::dali::pres::special::SpecialCommand;
use crate::dali::pres::standard::StandardCommand;
use crate::dali::types::DaliAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DaliCommand {
    Standard {
        address: DaliAddress,
        command: StandardCommand,
    },
    Extended {
        address: DaliAddress,
        command: ExtendedCommand,
    },
    Special(SpecialCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

impl DaliCommand {
    pub fn address(&self) -> Option<&DaliAddress> {
        match self {
            Self::Standard { address, .. } => Some(address),
            Self::Extended { address, .. } => Some(address),
            Self::Special(_) => None,
        }
    }

    pub fn to_forward_frame(&self) -> ForwardFrame {
        match self {
            Self::Standard { address, command } => command.to_forward_frame(address),
            Self::Extended { address, command } => command.to_forward_frame(address),
            Self::Special(cmd) => cmd.to_forward_frame(),
        }
    }

    pub fn is_query(&self) -> bool {
        match self {
            Self::Standard { command, .. } => command.is_query(),
            Self::Extended { command, .. } => command.is_query(),
            Self::Special(cmd) => cmd.expects_backward(),
        }
    }

    pub fn requires_repeat(&self) -> bool {
        match self {
            Self::Standard { command, .. } => command.requires_repeat(),
            Self::Extended { command, .. } => command.requires_repeat(),
            Self::Special(cmd) => cmd.requires_repeat(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaliResponse {
    Answer(u8),
    NoAnswer,
    Violation,
}

impl DaliResponse {
    pub fn from_backward(frame: Option<BackwardFrame>) -> Self {
        match frame {
            Some(f) => Self::Answer(f.raw()),
            None => Self::NoAnswer,
        }
    }

    pub fn value(self) -> Option<u8> {
        match self {
            Self::Answer(value) => Some(value),
            Self::NoAnswer | Self::Violation => None,
        }
    }

    pub fn is_yes(self) -> bool {
        match self {
            Self::Answer(_) | Self::Violation => true,
            Self::NoAnswer => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_arc_power_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(1).unwrap(),
            command: StandardCommand::DirectArcPower { level: 254 },
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0x02);
        assert_eq!(f.command_byte(), 0xFE);
    }

    #[test]
    fn off_command_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::Off,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0xFF);
        assert_eq!(f.command_byte(), 0x00);
    }

    #[test]
    fn go_to_scene_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::GoToScene { scene: 5 },
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0x01);
        assert_eq!(f.command_byte(), 0x15);
    }

    #[test]
    fn query_status_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(5).unwrap(),
            command: StandardCommand::QueryStatus,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0x0B);
        assert_eq!(f.command_byte(), 0x90);
    }

    #[test]
    fn query_physical_minimum_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryPhysicalMinimum,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x9A);
    }

    #[test]
    fn query_version_number_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryVersionNumber,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x97);
    }

    #[test]
    fn query_content_dtr0_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryContentDtr0,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x98);
    }

    #[test]
    fn query_device_type_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryDeviceType,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x99);
    }

    #[test]
    fn query_operating_mode_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryOperatingMode,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x9E);
    }

    #[test]
    fn query_random_address_bytes_use_golden_opcodes() {
        let address = DaliAddress::short(0).unwrap();
        let high = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryRandomAddressH,
        }
        .to_forward_frame();
        let middle = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryRandomAddressM,
        }
        .to_forward_frame();
        let low = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryRandomAddressL,
        }
        .to_forward_frame();
        assert_eq!(high.command_byte(), 0xC2);
        assert_eq!(middle.command_byte(), 0xC3);
        assert_eq!(low.command_byte(), 0xC4);
    }

    #[test]
    fn query_power_failure_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryPowerFailure,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x9B);
    }

    #[test]
    fn query_limit_error_correct_opcode() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::QueryLimitError,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.command_byte(), 0x94);
    }

    #[test]
    fn set_scene_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(3).unwrap(),
            command: StandardCommand::SetScene { scene: 10 },
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0x07);
        assert_eq!(f.command_byte(), 0x4A);
    }

    #[test]
    fn add_to_group_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::short(0).unwrap(),
            command: StandardCommand::AddToGroup { group: 3 },
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0x01);
        assert_eq!(f.command_byte(), 0x63);
    }

    #[test]
    fn set_short_address_frame() {
        let cmd = DaliCommand::Standard {
            address: DaliAddress::Broadcast,
            command: StandardCommand::SetShortAddress,
        };
        let f = cmd.to_forward_frame();
        assert_eq!(f.address_byte(), 0xFF);
        assert_eq!(f.command_byte(), 0x80);
    }

    #[test]
    fn response_from_backward_valid() {
        let resp = DaliResponse::from_backward(Some(BackwardFrame::new(0x42)));
        assert_eq!(resp, DaliResponse::Answer(0x42));
    }

    #[test]
    fn response_from_backward_none() {
        let resp = DaliResponse::from_backward(None);
        assert_eq!(resp, DaliResponse::NoAnswer);
    }

    #[test]
    fn response_from_backward_ff_is_answer() {
        let resp = DaliResponse::from_backward(Some(BackwardFrame::new(0xFF)));
        assert_eq!(resp, DaliResponse::Answer(0xFF));
    }

    #[test]
    fn write_memory_location_expects_backward() {
        let cmd = DaliCommand::Special(SpecialCommand::WriteMemoryLocation(0x55));
        assert!(cmd.is_query());

        let no_reply = DaliCommand::Special(SpecialCommand::WriteMemoryLocationNoReply(0x55));
        assert!(!no_reply.is_query());
    }

    #[test]
    fn dtr_special_command_is_not_query_even_if_data_byte_matches_query_opcode() {
        let cmd = DaliCommand::Special(SpecialCommand::Dtr0(0xFE));
        assert!(!cmd.is_query());
    }

    #[test]
    fn standard_is_query() {
        assert!(StandardCommand::QueryStatus.is_query());
        assert!(StandardCommand::QuerySceneLevel { scene: 15 }.is_query());
        assert!(StandardCommand::QueryGroups0To7.is_query());
        assert!(StandardCommand::QueryGroups8To15.is_query());
        assert!(!StandardCommand::Off.is_query());
        assert!(!StandardCommand::DirectArcPower { level: 100 }.is_query());
    }

    #[test]
    fn config_requires_repeat() {
        assert!(StandardCommand::Reset.requires_repeat());
        assert!(StandardCommand::SetScene { scene: 0 }.requires_repeat());
        assert!(!StandardCommand::Off.requires_repeat());
        assert!(!StandardCommand::QueryStatus.requires_repeat());
        assert!(!StandardCommand::QuerySceneLevel { scene: 0 }.requires_repeat());
        assert!(!StandardCommand::QueryGroups0To7.requires_repeat());
        assert!(!StandardCommand::QueryGroups8To15.requires_repeat());
    }

    #[test]
    fn scene_and_group_queries_are_typed_queries() {
        let address = DaliAddress::short(3).unwrap();
        let scene = DaliCommand::Standard {
            address,
            command: StandardCommand::QuerySceneLevel { scene: 10 },
        };
        assert!(scene.is_query());
        let scene_frame = scene.to_forward_frame();
        assert_eq!(scene_frame.address_byte(), 0x07);
        assert_eq!(scene_frame.command_byte(), 0xBA);

        let groups_0_7 = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryGroups0To7,
        };
        assert!(groups_0_7.is_query());
        let groups_0_7_frame = groups_0_7.to_forward_frame();
        assert_eq!(groups_0_7_frame.command_byte(), 0xC0);

        let groups_8_15 = DaliCommand::Standard {
            address,
            command: StandardCommand::QueryGroups8To15,
        };
        assert!(groups_8_15.is_query());
        let groups_8_15_frame = groups_8_15.to_forward_frame();
        assert_eq!(groups_8_15_frame.command_byte(), 0xC1);
    }

    #[test]
    fn extended_dt8_query_is_typed_query() {
        let cmd = DaliCommand::Extended {
            address: DaliAddress::short(1).unwrap(),
            command: ExtendedCommand::Dt8(
                crate::dali::devices::dt8_color::Dt8Command::QueryColourValue,
            ),
        };

        assert!(cmd.is_query());
        let frame = cmd.to_forward_frame();
        assert_eq!(frame.address_byte(), 0x03);
        assert_eq!(frame.command_byte(), 0xFA);
    }
}
