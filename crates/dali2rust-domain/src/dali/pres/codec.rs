use crate::dali::net::address::decode_wire_address;
use crate::dali::pres::extended::ExtendedCommand;
use crate::dali::pres::special::SpecialCommand;
use crate::dali::pres::standard::StandardCommand;
use crate::dali::pres::{DaliCommand, DecodeError};

pub fn dali_command_from_wire(
    wire_address: u8,
    command: u8,
    _repeat_count: u8,
) -> Result<DaliCommand, DecodeError> {
    if let Some(special) = try_decode_special(wire_address, command) {
        return Ok(DaliCommand::Special(special));
    }

    let address = decode_wire_address(wire_address).map_err(|_| DecodeError)?;

    let is_dapc = (wire_address & 0x01) == 0;

    if is_dapc {
        return Ok(DaliCommand::Standard {
            address,
            command: StandardCommand::DirectArcPower { level: command },
        });
    }

    if let Some(extended) = try_decode_extended(command) {
        return Ok(DaliCommand::Extended {
            address,
            command: extended,
        });
    }

    let standard = decode_standard_command(command)?;
    Ok(DaliCommand::Standard {
        address,
        command: standard,
    })
}

fn try_decode_special(wire_address: u8, command: u8) -> Option<SpecialCommand> {
    match wire_address {
        0xA1 if command == 0x00 => Some(SpecialCommand::Terminate),
        0xA3 => Some(SpecialCommand::Dtr0(command)),
        0xA5 => Some(SpecialCommand::Initialise(command)),
        0xA7 if command == 0x00 => Some(SpecialCommand::Randomise),
        0xA9 if command == 0x00 => Some(SpecialCommand::Compare),
        0xAB if command == 0x00 => Some(SpecialCommand::Withdraw),
        0xAD if command == 0x00 => Some(SpecialCommand::Ping),
        0xB1 => Some(SpecialCommand::SearchAddrH(command)),
        0xB3 => Some(SpecialCommand::SearchAddrM(command)),
        0xB5 => Some(SpecialCommand::SearchAddrL(command)),
        0xB7 => Some(SpecialCommand::ProgramShortAddress(command)),
        0xB9 => Some(SpecialCommand::VerifyShortAddress(command)),
        0xBB if command == 0x00 => Some(SpecialCommand::QueryShortAddress),
        0xBD if command == 0x00 => Some(SpecialCommand::PhysicalSelection),
        0xC1 => Some(SpecialCommand::EnableDeviceType(command)),
        0xC3 => Some(SpecialCommand::Dtr1(command)),
        0xC5 => Some(SpecialCommand::Dtr2(command)),
        0xC7 => Some(SpecialCommand::WriteMemoryLocation(command)),
        0xC9 => Some(SpecialCommand::WriteMemoryLocationNoReply(command)),
        _ => None,
    }
}

fn decode_standard_command(command: u8) -> Result<StandardCommand, DecodeError> {
    if let Some(cmd) = decode_arc_power_control(command) {
        return Ok(cmd);
    }
    if let Some(cmd) = decode_scene_and_group_command(command) {
        return Ok(cmd);
    }
    if let Some(cmd) = decode_configuration_command(command) {
        return Ok(cmd);
    }
    if let Some(cmd) = decode_query_command(command) {
        return Ok(cmd);
    }
    Err(DecodeError)
}

fn decode_arc_power_control(command: u8) -> Option<StandardCommand> {
    match command {
        0x00 => Some(StandardCommand::Off),
        0x01 => Some(StandardCommand::Up),
        0x02 => Some(StandardCommand::Down),
        0x03 => Some(StandardCommand::StepUp),
        0x04 => Some(StandardCommand::StepDown),
        0x05 => Some(StandardCommand::RecallMaxLevel),
        0x06 => Some(StandardCommand::RecallMinLevel),
        0x07 => Some(StandardCommand::StepDownAndOff),
        0x08 => Some(StandardCommand::OnAndStepUp),
        0x09 => Some(StandardCommand::EnableDapcSequence),
        0x0A => Some(StandardCommand::GoToLastActiveLevel),
        _ => None,
    }
}

fn decode_scene_and_group_command(command: u8) -> Option<StandardCommand> {
    match command {
        0x10..=0x1F => Some(StandardCommand::GoToScene {
            scene: command & 0x0F,
        }),
        0x40..=0x4F => Some(StandardCommand::SetScene {
            scene: command & 0x0F,
        }),
        0x50..=0x5F => Some(StandardCommand::RemoveScene {
            scene: command & 0x0F,
        }),
        0x60..=0x6F => Some(StandardCommand::AddToGroup {
            group: command & 0x0F,
        }),
        0x70..=0x7F => Some(StandardCommand::RemoveFromGroup {
            group: command & 0x0F,
        }),
        _ => None,
    }
}

fn decode_configuration_command(command: u8) -> Option<StandardCommand> {
    match command {
        0x20 => Some(StandardCommand::Reset),
        0x21 => Some(StandardCommand::StoreActualLevelInDtr0),
        0x22 => Some(StandardCommand::SavePersistentVariables),
        0x23 => Some(StandardCommand::SetOperatingMode),
        0x24 => Some(StandardCommand::ResetMemoryBank),
        0x25 => Some(StandardCommand::IdentifyDevice),
        0x2A => Some(StandardCommand::SetMaxLevel),
        0x2B => Some(StandardCommand::SetMinLevel),
        0x2C => Some(StandardCommand::SetSystemFailureLevel),
        0x2D => Some(StandardCommand::SetPowerOnLevel),
        0x2E => Some(StandardCommand::SetFadeTime),
        0x2F => Some(StandardCommand::SetFadeRate),
        0x30 => Some(StandardCommand::SetExtendedFadeTime),
        0x81 => Some(StandardCommand::EnableWriteMemory),
        0x80 => Some(StandardCommand::SetShortAddress),
        _ => None,
    }
}

fn decode_query_command(command: u8) -> Option<StandardCommand> {
    match command {
        0x90 => Some(StandardCommand::QueryStatus),
        0x91 => Some(StandardCommand::QueryControlGearPresent),
        0x92 => Some(StandardCommand::QueryLampFailure),
        0x93 => Some(StandardCommand::QueryLampPowerOn),
        0x94 => Some(StandardCommand::QueryLimitError),
        0x95 => Some(StandardCommand::QueryResetState),
        0x96 => Some(StandardCommand::QueryMissingShortAddress),
        0x97 => Some(StandardCommand::QueryVersionNumber),
        0x98 => Some(StandardCommand::QueryContentDtr0),
        0x99 => Some(StandardCommand::QueryDeviceType),
        0x9A => Some(StandardCommand::QueryPhysicalMinimum),
        0x9B => Some(StandardCommand::QueryPowerFailure),
        0x9C => Some(StandardCommand::QueryContentDtr1),
        0x9D => Some(StandardCommand::QueryContentDtr2),
        0x9E => Some(StandardCommand::QueryOperatingMode),
        0x9F => Some(StandardCommand::QueryLightSourceType),
        0xA0 => Some(StandardCommand::QueryActualLevel),
        0xA1 => Some(StandardCommand::QueryMaxLevel),
        0xA2 => Some(StandardCommand::QueryMinLevel),
        0xA3 => Some(StandardCommand::QueryPowerOnLevel),
        0xA4 => Some(StandardCommand::QuerySystemFailureLevel),
        0xA5 => Some(StandardCommand::QueryFadeTimeFadeRate),
        0xA6 => Some(StandardCommand::QueryManufacturerSpecificMode),
        0xA7 => Some(StandardCommand::QueryNextDeviceType),
        0xAA => Some(StandardCommand::QueryControlGearFailure),
        0xA8 => Some(StandardCommand::QueryExtendedFadeTime),
        0xB0..=0xBF => Some(StandardCommand::QuerySceneLevel {
            scene: command & 0x0F,
        }),
        0xC0 => Some(StandardCommand::QueryGroups0To7),
        0xC1 => Some(StandardCommand::QueryGroups8To15),
        0xC2 => Some(StandardCommand::QueryRandomAddressH),
        0xC3 => Some(StandardCommand::QueryRandomAddressM),
        0xC4 => Some(StandardCommand::QueryRandomAddressL),
        _ => None,
    }
}

fn try_decode_extended(opcode: u8) -> Option<ExtendedCommand> {
    ExtendedCommand::from_opcode(opcode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::devices::dt6_led::Dt6Command;
    use crate::dali::types::DaliAddress;

    #[test]
    fn every_special_command_survives_encode_then_decode() {
        let cases = [
            SpecialCommand::Terminate,
            SpecialCommand::Dtr0(0x5A),
            SpecialCommand::Initialise(0xFF),
            SpecialCommand::Randomise,
            SpecialCommand::Compare,
            SpecialCommand::Withdraw,
            SpecialCommand::Ping,
            SpecialCommand::SearchAddrH(0x12),
            SpecialCommand::SearchAddrM(0x34),
            SpecialCommand::SearchAddrL(0x56),
            SpecialCommand::ProgramShortAddress(0x0B),
            SpecialCommand::VerifyShortAddress(0x0B),
            SpecialCommand::QueryShortAddress,
            SpecialCommand::PhysicalSelection,
            SpecialCommand::EnableDeviceType(8),
            SpecialCommand::Dtr1(0x5A),
            SpecialCommand::Dtr2(0x5A),
            SpecialCommand::WriteMemoryLocation(0x77),
            SpecialCommand::WriteMemoryLocationNoReply(0x77),
        ];
        for expected in cases {
            let (wire_address, command) = expected.wire_bytes();
            let decoded = dali_command_from_wire(wire_address, command, 1)
                .unwrap_or_else(|e| panic!("{expected:?} encodes to ({wire_address:#04X}, {command:#04X}), which does not decode: {e:?}"));
            assert_eq!(
                decoded,
                DaliCommand::Special(expected),
                "{expected:?} encodes to ({wire_address:#04X}, {command:#04X})"
            );
        }
    }

    #[test]
    fn wire_decode_direct_arc() {
        let result = dali_command_from_wire(0x02, 0xFE, 1).unwrap();
        match result {
            DaliCommand::Standard { address, command } => {
                assert_eq!(address, DaliAddress::short(1).unwrap());
                assert_eq!(command, StandardCommand::DirectArcPower { level: 0xFE });
            }
            _ => panic!("expected standard, got {:?}", result),
        }
    }

    #[test]
    fn wire_decode_off_broadcast() {
        let result = dali_command_from_wire(0xFF, 0x00, 1).unwrap();
        match result {
            DaliCommand::Standard { address, command } => {
                assert_eq!(address, DaliAddress::Broadcast);
                assert_eq!(command, StandardCommand::Off);
            }
            _ => panic!("expected standard"),
        }
    }

    #[test]
    fn wire_decode_query_status() {
        let result = dali_command_from_wire(0x0B, 0x90, 1).unwrap();
        match result {
            DaliCommand::Standard { address, command } => {
                assert_eq!(address, DaliAddress::short(5).unwrap());
                assert_eq!(command, StandardCommand::QueryStatus);
            }
            _ => panic!("expected standard"),
        }
    }

    #[test]
    fn wire_decode_set_scene_repeat_param() {
        let result = dali_command_from_wire(0x07, 0x4A, 1).unwrap();
        match result {
            DaliCommand::Standard { address, command } => {
                assert_eq!(address, DaliAddress::short(3).unwrap());
                assert_eq!(command, StandardCommand::SetScene { scene: 10 });
            }
            _ => panic!("expected standard"),
        }
    }

    #[test]
    fn repeat_count_does_not_override_scene_parameter() {
        let result = dali_command_from_wire(0x07, 0x40, 10).unwrap();
        match result {
            DaliCommand::Standard { command, .. } => {
                assert_eq!(command, StandardCommand::SetScene { scene: 0 });
            }
            _ => panic!("expected standard"),
        }
    }

    #[test]
    fn wire_decode_special_dtr0() {
        let result = dali_command_from_wire(0xA3, 0x42, 1).unwrap();
        assert_eq!(result, DaliCommand::Special(SpecialCommand::Dtr0(0x42)));
    }

    #[test]
    fn wire_decode_special_initialise() {
        let result = dali_command_from_wire(0xA5, 0x00, 1).unwrap();
        assert_eq!(
            result,
            DaliCommand::Special(SpecialCommand::Initialise(0x00))
        );
    }

    #[test]
    fn wire_decode_special_compare() {
        let result = dali_command_from_wire(0xA9, 0x00, 1).unwrap();
        assert_eq!(result, DaliCommand::Special(SpecialCommand::Compare));
    }

    #[test]
    fn wire_decode_extended_query_gear_type() {
        let result = dali_command_from_wire(0x0B, 0xED, 1).unwrap();
        match result {
            DaliCommand::Extended { command, .. } => {
                assert_eq!(command, ExtendedCommand::Dt6(Dt6Command::QueryGearType));
            }
            _ => panic!("expected extended, got {:?}", result),
        }
    }

    #[test]
    fn wire_decode_rejected_reserved() {
        assert!(dali_command_from_wire(0x01, 0x0B, 1).is_err());
    }

    #[test]
    fn decode_all_query_opcodes() {
        let query_opcodes = [
            (0x90, StandardCommand::QueryStatus),
            (0x91, StandardCommand::QueryControlGearPresent),
            (0x92, StandardCommand::QueryLampFailure),
            (0x93, StandardCommand::QueryLampPowerOn),
            (0x94, StandardCommand::QueryLimitError),
            (0x95, StandardCommand::QueryResetState),
            (0x96, StandardCommand::QueryMissingShortAddress),
            (0x97, StandardCommand::QueryVersionNumber),
            (0x98, StandardCommand::QueryContentDtr0),
            (0x99, StandardCommand::QueryDeviceType),
            (0x9A, StandardCommand::QueryPhysicalMinimum),
            (0x9B, StandardCommand::QueryPowerFailure),
            (0x9C, StandardCommand::QueryContentDtr1),
            (0x9D, StandardCommand::QueryContentDtr2),
            (0x9E, StandardCommand::QueryOperatingMode),
            (0x9F, StandardCommand::QueryLightSourceType),
            (0xA0, StandardCommand::QueryActualLevel),
            (0xA1, StandardCommand::QueryMaxLevel),
            (0xA2, StandardCommand::QueryMinLevel),
            (0xA3, StandardCommand::QueryPowerOnLevel),
            (0xA4, StandardCommand::QuerySystemFailureLevel),
            (0xA5, StandardCommand::QueryFadeTimeFadeRate),
            (0xA7, StandardCommand::QueryNextDeviceType),
            (0xA8, StandardCommand::QueryExtendedFadeTime),
            (0xC2, StandardCommand::QueryRandomAddressH),
            (0xC3, StandardCommand::QueryRandomAddressM),
            (0xC4, StandardCommand::QueryRandomAddressL),
        ];
        for (opcode, expected) in query_opcodes {
            let result = dali_command_from_wire(0x01, opcode, 1).unwrap();
            match result {
                DaliCommand::Standard { command, .. } => {
                    assert_eq!(command, expected, "opcode 0x{opcode:02X}");
                }
                _ => panic!("opcode 0x{opcode:02X}: expected standard, got {result:?}"),
            }
        }
    }
}

#[cfg(test)]
mod issue58_missing_queries_tests {
    use super::*;
    use crate::dali::pres::opcode::{is_query_opcode, QUERY_OPCODES};

    #[test]
    fn both_missing_102_queries_decode_and_expect_an_answer() {
        for (opcode, expected) in [
            (0xA6u8, StandardCommand::QueryManufacturerSpecificMode),
            (0xAA, StandardCommand::QueryControlGearFailure),
        ] {
            let decoded = dali_command_from_wire(0x01, opcode, 0).expect("a 102 table command");
            let DaliCommand::Standard { command, .. } = decoded else {
                panic!("{opcode:#04X} decoded as {decoded:?}, not a standard command");
            };
            assert_eq!(command, expected);
            assert!(
                command.is_query(),
                "{opcode:#04X} answers, so it must open a backward window"
            );
            assert!(
                QUERY_OPCODES.contains(&opcode) && is_query_opcode(opcode),
                "{opcode:#04X} missing from the opcode table the sniffer reads"
            );
            assert_eq!(
                command.to_forward_frame(&crate::dali::net::address::DaliAddress::Short(0)).raw()
                    & 0x00FF,
                u16::from(opcode),
                "the opcode must survive the round trip it is decoded from"
            );
        }
    }
}
