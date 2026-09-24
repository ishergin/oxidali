use crate::dali::frame::ForwardFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialCommand {
    Terminate,
    Dtr0(u8),
    Initialise(u8),
    Randomise,
    Compare,
    Withdraw,
    Ping,

    SearchAddrH(u8),
    SearchAddrM(u8),
    SearchAddrL(u8),

    ProgramShortAddress(u8),
    VerifyShortAddress(u8),
    QueryShortAddress,

    PhysicalSelection,

    EnableDeviceType(u8),
    Dtr1(u8),
    Dtr2(u8),
    WriteMemoryLocation(u8),
    WriteMemoryLocationNoReply(u8),
}

impl SpecialCommand {
    pub const fn wire_bytes(self) -> (u8, u8) {
        match self {
            Self::Terminate => (0xA1, 0x00),
            Self::Dtr0(data) => (0xA3, data),
            Self::Initialise(data) => (0xA5, data),
            Self::Randomise => (0xA7, 0x00),
            Self::Compare => (0xA9, 0x00),
            Self::Withdraw => (0xAB, 0x00),
            Self::Ping => (0xAD, 0x00),
            Self::SearchAddrH(data) => (0xB1, data),
            Self::SearchAddrM(data) => (0xB3, data),
            Self::SearchAddrL(data) => (0xB5, data),
            Self::ProgramShortAddress(addr) => (0xB7, addr),
            Self::VerifyShortAddress(addr) => (0xB9, addr),
            Self::QueryShortAddress => (0xBB, 0x00),
            Self::PhysicalSelection => (0xBD, 0x00),
            Self::EnableDeviceType(dt) => (0xC1, dt),
            Self::Dtr1(data) => (0xC3, data),
            Self::Dtr2(data) => (0xC5, data),
            Self::WriteMemoryLocation(data) => (0xC7, data),
            Self::WriteMemoryLocationNoReply(data) => (0xC9, data),
        }
    }

    pub fn to_forward_frame(&self) -> ForwardFrame {
        let (addr, cmd) = self.wire_bytes();
        ForwardFrame::new(addr, cmd)
    }

    pub const fn expects_backward(self) -> bool {
        matches!(
            self,
            Self::Compare
                | Self::VerifyShortAddress(_)
                | Self::QueryShortAddress
                | Self::WriteMemoryLocation(_)
        )
    }

    pub const fn requires_repeat(self) -> bool {
        matches!(self, Self::Initialise(_) | Self::Randomise)
    }
}

// IEC 62386-102 §11.7.13
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryShortAddressAnswer {
    Address(u8),
    Unaddressed,
    Multiple,
    None,
}

impl QueryShortAddressAnswer {
    pub fn decode(byte: Option<u8>, violation: bool) -> Self {
        if violation {
            return Self::Multiple;
        }
        match byte {
            Some(SHORT_ADDRESS_ANSWER_MASK) => Self::Unaddressed,
            Some(raw) if raw & 0x01 != 0 => Self::Address((raw >> 1) & SHORT_ADDRESS_VALUE_MASK),
            _ => Self::None,
        }
    }

    pub fn address(self) -> Option<u8> {
        match self {
            Self::Address(a) => Some(a),
            _ => None,
        }
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Address(_) => "address",
            Self::Unaddressed => "unaddressed",
            Self::Multiple => "multiple",
            Self::None => "none",
        }
    }
}

const SHORT_ADDRESS_ANSWER_MASK: u8 = 0xFF;
const SHORT_ADDRESS_VALUE_MASK: u8 = 0x3F;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_bytes_encode_commissioning_and_memory_commands() {
        assert_eq!(SpecialCommand::Terminate.wire_bytes(), (0xA1, 0x00));
        assert_eq!(SpecialCommand::SearchAddrM(0x34).wire_bytes(), (0xB3, 0x34));
        assert_eq!(
            SpecialCommand::WriteMemoryLocationNoReply(0x77).wire_bytes(),
            (0xC9, 0x77)
        );
    }

    #[test]
    fn expects_backward_matches_query_like_special_commands() {
        assert!(SpecialCommand::Compare.expects_backward());
        assert!(SpecialCommand::VerifyShortAddress(0x55).expects_backward());
        assert!(SpecialCommand::WriteMemoryLocation(0x12).expects_backward());
        assert!(!SpecialCommand::Dtr0(0x12).expects_backward());
        assert!(!SpecialCommand::EnableDeviceType(8).expects_backward());
    }

    #[test]
    fn repeat_only_applies_to_initialise_and_randomise() {
        assert!(SpecialCommand::Initialise(0xFF).requires_repeat());
        assert!(SpecialCommand::Randomise.requires_repeat());
        assert!(!SpecialCommand::Terminate.requires_repeat());
        assert!(!SpecialCommand::ProgramShortAddress(0x7F).requires_repeat());
    }

    #[test]
    fn mask_is_unaddressed_and_never_address_sixty_three() {
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0xFF), false),
            QueryShortAddressAnswer::Unaddressed
        );
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0xFF), false).address(),
            None,
            "102 §11.7.13 spends MASK on 'this gear has no short address'"
        );
    }

    #[test]
    fn an_address_answer_decodes_its_six_bits() {
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0x23), false),
            QueryShortAddressAnswer::Address(17)
        );
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0x7F), false),
            QueryShortAddressAnswer::Address(63)
        );
    }

    #[test]
    fn a_violation_outranks_any_byte_and_reads_as_multiple() {
        assert_eq!(
            QueryShortAddressAnswer::decode(None, true),
            QueryShortAddressAnswer::Multiple
        );
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0x23), true),
            QueryShortAddressAnswer::Multiple,
            "a violating frame carries no content, so the byte cannot win"
        );
    }

    #[test]
    fn silence_and_a_zero_byte_are_both_none() {
        assert_eq!(
            QueryShortAddressAnswer::decode(None, false),
            QueryShortAddressAnswer::None
        );
        assert_eq!(
            QueryShortAddressAnswer::decode(Some(0x00), false),
            QueryShortAddressAnswer::None
        );
    }

    #[test]
    fn every_answer_has_a_distinct_wire_name() {
        let names = [
            QueryShortAddressAnswer::Address(0).wire_name(),
            QueryShortAddressAnswer::Unaddressed.wire_name(),
            QueryShortAddressAnswer::Multiple.wire_name(),
            QueryShortAddressAnswer::None.wire_name(),
        ];
        let mut sorted = names.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "two answers share a REST name");
    }
}
