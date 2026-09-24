use crate::dali::devices::{CommandMetadata, DeviceCommandMetadata, DeviceType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dt6Command {
    ReferenceSystemPower,
    EnableCurrentProtector,
    DisableCurrentProtector,
    SelectDimmingCurve,
    StoreDtrAsFastFadeTime,

    QueryGearType,
    QueryDimmingCurve,
    QueryPossibleOperatingMode,
    QueryFeatures,
    QueryFailureStatus,
    QueryShortCircuit,
    QueryOpenCircuit,
    QueryLoadDecrease,
    QueryLoadIncrease,
    QueryCurrentProtectorActive,
    QueryThermalShutdown,
    QueryThermalOverload,
    QueryReferenceRunning,
    QueryReferenceMeasurementFailed,
    QueryCurrentProtectorEnabled,
    QueryOperatingMode,
    QueryFastFadeTime,
    QueryMinFastFadeTime,
    QueryExtendedVersionNumber,
}

// IEC 62386-207 §11.3.4.2
pub const FAILURE_SHORT_CIRCUIT: u8 = 0x01;
pub const FAILURE_OPEN_CIRCUIT: u8 = 0x02;
pub const FAILURE_LOAD_DECREASE: u8 = 0x04;
pub const FAILURE_LOAD_INCREASE: u8 = 0x08;
pub const FAILURE_CURRENT_PROTECTOR_ACTIVE: u8 = 0x10;
pub const FAILURE_THERMAL_SHUT_DOWN: u8 = 0x20;
pub const FAILURE_THERMAL_OVERLOAD: u8 = 0x40;
pub const FAILURE_REFERENCE_MEASUREMENT_FAILED: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureStatusBits {
    pub short_circuit: bool,
    pub open_circuit: bool,
    pub load_decrease: bool,
    pub load_increase: bool,
    pub current_protector_active: bool,
    pub thermal_shut_down: bool,
    pub thermal_overload: bool,
    pub reference_measurement_failed: bool,
}

#[must_use]
pub const fn decode_failure_status(raw: u8) -> FailureStatusBits {
    FailureStatusBits {
        short_circuit: raw & FAILURE_SHORT_CIRCUIT != 0,
        open_circuit: raw & FAILURE_OPEN_CIRCUIT != 0,
        load_decrease: raw & FAILURE_LOAD_DECREASE != 0,
        load_increase: raw & FAILURE_LOAD_INCREASE != 0,
        current_protector_active: raw & FAILURE_CURRENT_PROTECTOR_ACTIVE != 0,
        thermal_shut_down: raw & FAILURE_THERMAL_SHUT_DOWN != 0,
        thermal_overload: raw & FAILURE_THERMAL_OVERLOAD != 0,
        reference_measurement_failed: raw & FAILURE_REFERENCE_MEASUREMENT_FAILED != 0,
    }
}

// IEC 62386-207 §11.3.4.1
pub const CONFIG_OPCODE_FIRST: u8 = 0xE0;
pub const CONFIG_OPCODE_LAST: u8 = 0xE4;

pub const fn opcode_requires_repeat(opcode: u8) -> bool {
    matches!(opcode, CONFIG_OPCODE_FIRST..=CONFIG_OPCODE_LAST)
}

impl Dt6Command {
    pub const fn opcode(&self) -> u8 {
        match self {
            Self::ReferenceSystemPower => 0xE0,
            Self::EnableCurrentProtector => 0xE1,
            Self::DisableCurrentProtector => 0xE2,
            Self::SelectDimmingCurve => 0xE3,
            Self::StoreDtrAsFastFadeTime => 0xE4,
            Self::QueryGearType => 0xED,
            Self::QueryDimmingCurve => 0xEE,
            Self::QueryPossibleOperatingMode => 0xEF,
            Self::QueryFeatures => 0xF0,
            Self::QueryFailureStatus => 0xF1,
            Self::QueryShortCircuit => 0xF2,
            Self::QueryOpenCircuit => 0xF3,
            Self::QueryLoadDecrease => 0xF4,
            Self::QueryLoadIncrease => 0xF5,
            Self::QueryCurrentProtectorActive => 0xF6,
            Self::QueryThermalShutdown => 0xF7,
            Self::QueryThermalOverload => 0xF8,
            Self::QueryReferenceRunning => 0xF9,
            Self::QueryReferenceMeasurementFailed => 0xFA,
            Self::QueryCurrentProtectorEnabled => 0xFB,
            Self::QueryOperatingMode => 0xFC,
            Self::QueryFastFadeTime => 0xFD,
            Self::QueryMinFastFadeTime => 0xFE,
            Self::QueryExtendedVersionNumber => 0xFF,
        }
    }

    pub fn from_opcode(opcode: u8) -> Option<Self> {
        match opcode {
            0xE0 => Some(Self::ReferenceSystemPower),
            0xE1 => Some(Self::EnableCurrentProtector),
            0xE2 => Some(Self::DisableCurrentProtector),
            0xE3 => Some(Self::SelectDimmingCurve),
            0xE4 => Some(Self::StoreDtrAsFastFadeTime),
            0xED => Some(Self::QueryGearType),
            0xEE => Some(Self::QueryDimmingCurve),
            0xEF => Some(Self::QueryPossibleOperatingMode),
            0xF0 => Some(Self::QueryFeatures),
            0xF1 => Some(Self::QueryFailureStatus),
            0xF2 => Some(Self::QueryShortCircuit),
            0xF3 => Some(Self::QueryOpenCircuit),
            0xF4 => Some(Self::QueryLoadDecrease),
            0xF5 => Some(Self::QueryLoadIncrease),
            0xF6 => Some(Self::QueryCurrentProtectorActive),
            0xF7 => Some(Self::QueryThermalShutdown),
            0xF8 => Some(Self::QueryThermalOverload),
            0xF9 => Some(Self::QueryReferenceRunning),
            0xFA => Some(Self::QueryReferenceMeasurementFailed),
            0xFB => Some(Self::QueryCurrentProtectorEnabled),
            0xFC => Some(Self::QueryOperatingMode),
            0xFD => Some(Self::QueryFastFadeTime),
            0xFE => Some(Self::QueryMinFastFadeTime),
            0xFF => Some(Self::QueryExtendedVersionNumber),
            _ => None,
        }
    }
}

impl DeviceCommandMetadata for Dt6Command {
    fn metadata(&self) -> CommandMetadata {
        CommandMetadata {
            device_type: Some(DeviceType::Led),
            opcode: self.opcode(),
            expects_backward: self.opcode() >= 0xED,
            requires_repeat: opcode_requires_repeat(self.opcode()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dt6_roundtrips_query_and_config_opcodes() {
        assert_eq!(
            Dt6Command::from_opcode(Dt6Command::ReferenceSystemPower.opcode()),
            Some(Dt6Command::ReferenceSystemPower)
        );
        assert_eq!(
            Dt6Command::from_opcode(Dt6Command::QueryFeatures.opcode()),
            Some(Dt6Command::QueryFeatures)
        );
        assert_eq!(Dt6Command::from_opcode(0xE5), None);
    }

    #[test]
    fn dt6_metadata_marks_queries_as_backward_expected() {
        let config = Dt6Command::SelectDimmingCurve.metadata();
        assert_eq!(config.device_type, Some(DeviceType::Led));
        assert!(!config.expects_backward);

        let query = Dt6Command::QueryFastFadeTime.metadata();
        assert_eq!(query.opcode, 0xFD);
        assert!(query.expects_backward);
        assert!(!query.requires_repeat);
    }

    #[test]
    fn dt6_configuration_commands_are_send_twice_and_nothing_else_is() {
        for opcode in 0x00..=0xFFu8 {
            assert_eq!(
                opcode_requires_repeat(opcode),
                (0xE0..=0xE4).contains(&opcode),
                "opcode {opcode:#04X} against IEC 62386-207 §11.3.4.1 (commands 224 to 228)"
            );
        }

        for config in [
            Dt6Command::ReferenceSystemPower,
            Dt6Command::EnableCurrentProtector,
            Dt6Command::DisableCurrentProtector,
            Dt6Command::SelectDimmingCurve,
            Dt6Command::StoreDtrAsFastFadeTime,
        ] {
            assert!(
                config.metadata().requires_repeat,
                "{config:?} is a configuration command"
            );
        }
        assert!(!Dt6Command::QueryGearType.metadata().requires_repeat);
        assert!(
            !Dt6Command::QueryExtendedVersionNumber
                .metadata()
                .requires_repeat
        );
    }

    #[test]
    fn every_failure_bit_belongs_to_the_query_that_asks_it() {
        for (opcode, mask) in [
            (Dt6Command::QueryShortCircuit, FAILURE_SHORT_CIRCUIT),
            (Dt6Command::QueryOpenCircuit, FAILURE_OPEN_CIRCUIT),
            (Dt6Command::QueryLoadDecrease, FAILURE_LOAD_DECREASE),
            (Dt6Command::QueryLoadIncrease, FAILURE_LOAD_INCREASE),
            (
                Dt6Command::QueryCurrentProtectorActive,
                FAILURE_CURRENT_PROTECTOR_ACTIVE,
            ),
            (Dt6Command::QueryThermalShutdown, FAILURE_THERMAL_SHUT_DOWN),
            (Dt6Command::QueryThermalOverload, FAILURE_THERMAL_OVERLOAD),
            (
                Dt6Command::QueryReferenceMeasurementFailed,
                FAILURE_REFERENCE_MEASUREMENT_FAILED,
            ),
        ] {
            let expected_bit = match opcode {
                Dt6Command::QueryReferenceMeasurementFailed => 7,
                other => other.opcode() - Dt6Command::QueryShortCircuit.opcode(),
            };
            assert_eq!(
                mask,
                1u8 << expected_bit,
                "{opcode:?} asks the condition of bit {expected_bit}"
            );
        }
    }

    #[test]
    fn a_failure_byte_decodes_bit_for_bit() {
        let none = decode_failure_status(0x00);
        assert!(!none.short_circuit && !none.thermal_shut_down);
        let all = decode_failure_status(0xFF);
        assert!(all.short_circuit && all.reference_measurement_failed);
        let mixed = decode_failure_status(0x21);
        assert!(mixed.short_circuit, "bit 0");
        assert!(mixed.thermal_shut_down, "bit 5");
        assert!(!mixed.open_circuit && !mixed.load_decrease && !mixed.thermal_overload);
    }
}
