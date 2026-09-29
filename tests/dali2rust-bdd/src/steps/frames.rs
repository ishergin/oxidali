use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

fn short_address(short: u8) -> DaliAddress {
    DaliAddress::short(short).expect("valid short address")
}

pub(crate) fn standard_frame(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

pub(crate) fn extended_frame(short: u8, command: ExtendedCommand) -> u16 {
    DaliCommand::Extended {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

pub(crate) fn special_frame(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

pub(crate) fn dt8_raw_query_frame(short: u8, opcode: u8) -> u16 {
    ForwardFrame::new(short_address(short).encode_address_byte() | 0x01, opcode).raw()
}

// IEC 62386-102 §11.6.2
pub(crate) fn extended_version_query_frame(short: u8) -> u16 {
    const QUERY_EXTENDED_VERSION_NUMBER: u16 = 0xFF;
    (u16::from((short << 1) | 1) << 8) | QUERY_EXTENDED_VERSION_NUMBER
}
