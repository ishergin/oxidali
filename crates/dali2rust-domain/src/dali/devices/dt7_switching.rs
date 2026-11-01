use crate::dali::devices::{CommandMetadata, DeviceCommandMetadata, DeviceType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dt7Command {
    ReferenceSystemPower,
    StoreDtrAsThreshold(Threshold),
    StoreDtrAsErrorHoldOffTime,

    QueryFeatures,
    QuerySwitchStatus,
    QueryThreshold(Threshold),
    QueryErrorHoldOffTime,
    QueryGearType,
    QueryReferenceRunning,
    QueryReferenceMeasurementFailed,
    QueryExtendedVersionNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    UpSwitchOn,
    UpSwitchOff,
    DownSwitchOn,
    DownSwitchOff,
}

// IEC 62386-208 §11.3.4.2
pub const FEATURE_LOAD_ERROR_QUERYABLE: u8 = 0x01;
pub const FEATURE_ADJUSTABLE_THRESHOLDS: u8 = 0x08;
pub const FEATURE_ADJUSTABLE_HOLD_OFF: u8 = 0x10;
pub const FEATURE_REFERENCE_SYSTEM_POWER: u8 = 0x40;
pub const FEATURE_PHYSICAL_SELECTION: u8 = 0x80;
pub const SWITCH_STATUS_REFERENCE_FAILED: u8 = 0x80;

// IEC 62386-208 §11.3.4.1
pub const CONFIG_OPCODE_FIRST: u8 = 0xE0;
pub const CONFIG_OPCODE_LAST: u8 = 0xEF;
const STORE_THRESHOLD_FIRST: u8 = 0xE1;
const QUERY_THRESHOLD_FIRST: u8 = 0xF2;
// IEC 62386-208 Table 2
pub const THRESHOLD_MASK: u8 = 0xFF;

pub const fn opcode_requires_repeat(opcode: u8) -> bool {
    matches!(opcode, CONFIG_OPCODE_FIRST..=CONFIG_OPCODE_LAST)
}

impl Threshold {
    pub const ALL: [Self; 4] = [
        Self::UpSwitchOn,
        Self::UpSwitchOff,
        Self::DownSwitchOn,
        Self::DownSwitchOff,
    ];

    pub const fn index(self) -> u8 {
        self as u8
    }

    const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::UpSwitchOn),
            1 => Some(Self::UpSwitchOff),
            2 => Some(Self::DownSwitchOn),
            3 => Some(Self::DownSwitchOff),
            _ => None,
        }
    }

    // IEC 62386-208 Table 2
    pub const fn factory(self) -> u8 {
        match self {
            Self::UpSwitchOn => 1,
            Self::UpSwitchOff | Self::DownSwitchOn => THRESHOLD_MASK,
            Self::DownSwitchOff => 0,
        }
    }

    // IEC 62386-208 §11.3.4, Table 2
    pub const fn within_range(self, dtr: u8) -> u8 {
        match self {
            Self::UpSwitchOn | Self::UpSwitchOff if dtr == 0 => 1,
            _ => dtr,
        }
    }
}

impl Dt7Command {
    pub const fn opcode(&self) -> u8 {
        match self {
            Self::ReferenceSystemPower => 0xE0,
            Self::StoreDtrAsThreshold(t) => STORE_THRESHOLD_FIRST + t.index(),
            Self::StoreDtrAsErrorHoldOffTime => 0xE5,
            Self::QueryFeatures => 0xF0,
            Self::QuerySwitchStatus => 0xF1,
            Self::QueryThreshold(t) => QUERY_THRESHOLD_FIRST + t.index(),
            Self::QueryErrorHoldOffTime => 0xF6,
            Self::QueryGearType => 0xF7,
            Self::QueryReferenceRunning => 0xF9,
            Self::QueryReferenceMeasurementFailed => 0xFA,
            Self::QueryExtendedVersionNumber => 0xFF,
        }
    }

    pub fn from_opcode(opcode: u8) -> Option<Self> {
        match opcode {
            0xE0 => Some(Self::ReferenceSystemPower),
            0xE1..=0xE4 => Threshold::from_index(opcode - STORE_THRESHOLD_FIRST).map(Self::StoreDtrAsThreshold),
            0xE5 => Some(Self::StoreDtrAsErrorHoldOffTime),
            0xF0 => Some(Self::QueryFeatures),
            0xF1 => Some(Self::QuerySwitchStatus),
            0xF2..=0xF5 => Threshold::from_index(opcode - QUERY_THRESHOLD_FIRST).map(Self::QueryThreshold),
            0xF6 => Some(Self::QueryErrorHoldOffTime),
            0xF7 => Some(Self::QueryGearType),
            0xF9 => Some(Self::QueryReferenceRunning),
            0xFA => Some(Self::QueryReferenceMeasurementFailed),
            0xFF => Some(Self::QueryExtendedVersionNumber),
            _ => None,
        }
    }
}

impl DeviceCommandMetadata for Dt7Command {
    fn metadata(&self) -> CommandMetadata {
        CommandMetadata {
            device_type: Some(DeviceType::Switching),
            opcode: self.opcode(),
            expects_backward: self.opcode() >= Self::QueryFeatures.opcode(),
            requires_repeat: opcode_requires_repeat(self.opcode()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT7_OPCODES: [u8; 17] = [
        0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF9,
        0xFA, 0xFF,
    ];

    #[test]
    fn every_part_208_opcode_round_trips_and_no_other_decodes() {
        for opcode in 0x00..=0xFFu8 {
            let decoded = Dt7Command::from_opcode(opcode);
            assert_eq!(
                decoded.is_some(),
                DT7_OPCODES.contains(&opcode),
                "opcode {opcode:#04X} against IEC 62386-208 §11.3.4"
            );
            if let Some(command) = decoded {
                assert_eq!(command.opcode(), opcode, "{command:?}");
            }
        }
    }

    #[test]
    fn the_command_table_names_each_threshold_and_the_hold_off() {
        assert_eq!(Dt7Command::StoreDtrAsThreshold(Threshold::UpSwitchOn).opcode(), 0xE1);
        assert_eq!(Dt7Command::StoreDtrAsThreshold(Threshold::DownSwitchOff).opcode(), 0xE4);
        assert_eq!(Dt7Command::StoreDtrAsErrorHoldOffTime.opcode(), 0xE5, "command 229");
        assert_eq!(Dt7Command::QueryThreshold(Threshold::UpSwitchOff).opcode(), 0xF3);
        assert_eq!(Dt7Command::QueryThreshold(Threshold::DownSwitchOn).opcode(), 0xF4);
        assert_eq!(Dt7Command::QueryErrorHoldOffTime.opcode(), 0xF6, "command 246");
        assert_eq!(Dt7Command::QueryReferenceRunning.opcode(), 0xF9, "command 249");
        assert_eq!(Dt7Command::QueryReferenceMeasurementFailed.opcode(), 0xFA, "command 250");
    }

    #[test]
    fn configuration_commands_are_send_twice_and_only_queries_expect_an_answer() {
        for opcode in 0x00..=0xFFu8 {
            assert_eq!(
                opcode_requires_repeat(opcode),
                (0xE0..=0xEF).contains(&opcode),
                "opcode {opcode:#04X} against IEC 62386-208 §11.3.4.1 (commands 224 to 239)"
            );
        }
        for opcode in DT7_OPCODES {
            let md = Dt7Command::from_opcode(opcode).expect("a Part 208 opcode").metadata();
            assert_eq!(md.device_type, Some(DeviceType::Switching));
            assert_eq!(md.expects_backward, opcode >= 0xF0, "{opcode:#04X}");
            assert_eq!(md.requires_repeat, opcode <= 0xE5, "{opcode:#04X}");
        }
    }

    #[test]
    fn thresholds_start_at_their_factory_values_and_an_up_threshold_never_holds_zero() {
        let factory = Threshold::ALL.map(Threshold::factory);
        assert_eq!(factory, [1, THRESHOLD_MASK, THRESHOLD_MASK, 0], "IEC 62386-208 Table 2");
        assert_eq!(Threshold::UpSwitchOn.within_range(0), 1);
        assert_eq!(Threshold::UpSwitchOff.within_range(0), 1);
        assert_eq!(Threshold::DownSwitchOn.within_range(0), 0);
        assert_eq!(Threshold::DownSwitchOff.within_range(0), 0);
        for threshold in Threshold::ALL {
            assert_eq!(threshold.within_range(THRESHOLD_MASK), THRESHOLD_MASK, "{threshold:?}");
            assert_eq!(threshold.within_range(0x80), 0x80, "{threshold:?}");
        }
    }
}
