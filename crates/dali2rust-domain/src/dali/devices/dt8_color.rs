use crate::dali::devices::{CommandMetadata, DeviceCommandMetadata, DeviceType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dt8Command {
    SetTemporaryXCoordinate,
    SetTemporaryYCoordinate,
    Activate,
    XCoordinateStepUp,
    XCoordinateStepDown,
    YCoordinateStepUp,
    YCoordinateStepDown,
    SetTemporaryColourTemperature,
    ColourTemperatureStepCooler,
    ColourTemperatureStepWarmer,
    SetTemporaryPrimaryNDimLevel,
    SetTemporaryRgbDimLevel,
    SetTemporaryWafDimLevel,
    SetTemporaryRgbwafControl,
    CopyReportToTemporary,
    StoreColourTemperatureTcLimit,
    StoreGearFeaturesStatus,
    QueryGearFeaturesStatus,
    QueryColourStatus,
    QueryColourTypeFeatures,
    QueryColourValue,
    QueryRgbwafControl,
}

impl Dt8Command {
    pub const fn is_query(&self) -> bool {
        matches!(
            self,
            Self::QueryGearFeaturesStatus
                | Self::QueryColourStatus
                | Self::QueryColourTypeFeatures
                | Self::QueryColourValue
                | Self::QueryRgbwafControl
        )
    }
}

// IEC 62386-209 §11.3.4.2
pub const CONFIG_OPCODE_FIRST: u8 = 0xEF;
pub const CONFIG_OPCODE_LAST: u8 = 0xF6;

pub const fn opcode_requires_repeat(opcode: u8) -> bool {
    matches!(opcode, CONFIG_OPCODE_FIRST..=CONFIG_OPCODE_LAST)
}

pub const COLOUR_VALUE_X: u8 = 0;
pub const COLOUR_VALUE_Y: u8 = 1;
pub const COLOUR_VALUE_TC: u8 = 2;
pub const COLOUR_VALUE_RED: u8 = 9;
pub const COLOUR_VALUE_GREEN: u8 = 10;
pub const COLOUR_VALUE_BLUE: u8 = 11;
pub const COLOUR_VALUE_WHITE: u8 = 12;
pub const COLOUR_VALUE_AMBER: u8 = 13;
pub const COLOUR_VALUE_FREECOLOUR: u8 = 14;
pub const COLOUR_VALUE_RGBWAF_CONTROL: u8 = 15;
pub const COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL: u8 = 207;
pub const COLOUR_VALUE_TC_COOLEST: u8 = 128;
pub const COLOUR_VALUE_TC_PHYSICAL_COOLEST: u8 = 129;
pub const COLOUR_VALUE_TC_WARMEST: u8 = 130;
pub const COLOUR_VALUE_TC_PHYSICAL_WARMEST: u8 = 131;
pub const COLOUR_VALUE_NUMBER_OF_PRIMARIES: u8 = 82;
pub const COLOUR_VALUE_TEMPORARY_X: u8 = 192;
pub const COLOUR_VALUE_TEMPORARY_Y: u8 = 193;
pub const COLOUR_VALUE_TEMPORARY_TC: u8 = 194;
pub const COLOUR_VALUE_TEMPORARY_COLOUR_TYPE: u8 = 208;
pub const COLOUR_VALUE_REPORT_X: u8 = 224;
pub const COLOUR_VALUE_REPORT_Y: u8 = 225;
pub const COLOUR_VALUE_REPORT_TC: u8 = 226;
pub const COLOUR_VALUE_REPORT_RED: u8 = 233;
pub const COLOUR_VALUE_REPORT_GREEN: u8 = 234;
pub const COLOUR_VALUE_REPORT_BLUE: u8 = 235;
pub const COLOUR_VALUE_REPORT_WHITE: u8 = 236;
pub const COLOUR_VALUE_REPORT_AMBER: u8 = 237;
pub const COLOUR_VALUE_REPORT_FREECOLOUR: u8 = 238;
pub const COLOUR_VALUE_REPORT_RGBWAF_CONTROL: u8 = 239;
pub const COLOUR_VALUE_REPORT_COLOUR_TYPE: u8 = 240;

pub const COLOUR_TYPE_BYTE_XY: u8 = 0x10;
pub const COLOUR_TYPE_BYTE_TC: u8 = 0x20;
pub const COLOUR_TYPE_BYTE_PRIMARY_N: u8 = 0x40;
pub const COLOUR_TYPE_BYTE_RGBWAF: u8 = 0x80;
pub const COLOUR_TYPE_BYTE_MASK: u8 = 0xFF;

// IEC 62386-209 Table 9
pub const TC_LIMIT_SELECTOR_COOLEST: u8 = 0;
pub const TC_LIMIT_SELECTOR_WARMEST: u8 = 1;
pub const TC_LIMIT_SELECTOR_PHYSICAL_COOLEST: u8 = 2;
pub const TC_LIMIT_SELECTOR_PHYSICAL_WARMEST: u8 = 3;

// IEC 62386-209 §9.9, Table 8
pub const fn colour_value_is_wide(value_id: u8) -> bool {
    matches!(value_id, 0..=8 | 64..=81 | 128..=131 | 192..=200 | 224..=232)
}

// IEC 62386-209 Table 11
pub const fn colour_value_is_defined(value_id: u8) -> bool {
    colour_value_is_wide(value_id) || matches!(value_id, 9..=15 | 82 | 201..=208 | 233..=240)
}

pub const COLOUR_VALUE_MASK: u16 = 0xFFFF;

pub const COLOUR_VALUE_LEVEL_MASK: u8 = 0xFF;

pub const fn srgb_channel_to_dim_level(srgb: u8) -> u8 {
    SRGB_TO_DIM_LEVEL[srgb as usize]
}

pub const fn dim_level_to_srgb_channel(dim: u8) -> u8 {
    DIM_LEVEL_TO_SRGB[dim as usize]
}

const SRGB_TO_DIM_LEVEL: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 11, 12, 12, 13, 13, 13, 14, 14, 15,
    15, 16, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27, 27, 28, 28, 29, 30,
    30, 31, 32, 32, 33, 34, 34, 35, 36, 37, 37, 38, 39, 40, 40, 41, 42, 43, 44, 44, 45, 46, 47, 48, 49, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 79, 80,
    81, 82, 83, 84, 86, 87, 88, 89, 91, 92, 93, 94, 96, 97, 98, 99, 101, 102, 103, 105, 106, 108, 109, 110, 112,
    113, 114, 116, 117, 119, 120, 122, 123, 125, 126, 128, 129, 131, 132, 134, 135, 137, 139, 140, 142, 143, 145,
    147, 148, 150, 152, 153, 155, 157, 158, 160, 162, 164, 165, 167, 169, 171, 173, 174, 176, 178, 180, 182, 184,
    186, 187, 189, 191, 193, 195, 197, 199, 201, 203, 205, 207, 209, 211, 213, 215, 217, 219, 221, 223, 226, 228,
    230, 232, 234, 236, 238, 241, 243, 245, 247, 249, 252, 254,
];

const DIM_LEVEL_TO_SRGB: [u8; 256] = [
    0, 1, 18, 26, 32, 37, 41, 45, 48, 52, 55, 58, 61, 63, 66, 68, 70, 73, 75, 77, 79, 81, 83, 84, 86, 88, 90, 91,
    93, 95, 96, 98, 99, 101, 102, 104, 105, 106, 108, 109, 110, 112, 113, 114, 115, 117, 118, 119, 120, 121, 123,
    124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145,
    146, 147, 148, 149, 150, 150, 151, 152, 153, 154, 155, 156, 156, 157, 158, 159, 160, 160, 161, 162, 163, 164,
    164, 165, 166, 167, 168, 168, 169, 170, 171, 171, 172, 173, 173, 174, 175, 176, 176, 177, 178, 179, 179, 180,
    181, 181, 182, 183, 183, 184, 185, 185, 186, 187, 187, 188, 189, 189, 190, 191, 191, 192, 193, 193, 194, 194,
    195, 196, 196, 197, 198, 198, 199, 199, 200, 201, 201, 202, 202, 203, 204, 204, 205, 205, 206, 207, 207, 208,
    208, 209, 209, 210, 211, 211, 212, 212, 213, 213, 214, 214, 215, 216, 216, 217, 217, 218, 218, 219, 219, 220,
    220, 221, 221, 222, 223, 223, 224, 224, 225, 225, 226, 226, 227, 227, 228, 228, 229, 229, 230, 230, 231, 231,
    232, 232, 233, 233, 234, 234, 235, 235, 236, 236, 237, 237, 238, 238, 239, 239, 240, 240, 241, 241, 242, 242,
    242, 243, 243, 244, 244, 245, 245, 246, 246, 247, 247, 248, 248, 249, 249, 249, 250, 250, 251, 251, 252, 252,
    253, 253, 254, 254, 254, 255, 255,
];

pub const COLOUR_STATUS_XY_OUT_OF_RANGE: u8 = 0x01;
pub const COLOUR_STATUS_TC_OUT_OF_RANGE: u8 = 0x02;
pub const COLOUR_STATUS_XY_ACTIVE: u8 = 0x10;
pub const COLOUR_STATUS_TC_ACTIVE: u8 = 0x20;
pub const COLOUR_STATUS_PRIMARY_N_ACTIVE: u8 = 0x40;
pub const COLOUR_STATUS_RGBWAF_ACTIVE: u8 = 0x80;

// IEC 62386-209 Table 8
pub const GEAR_FEATURES_AUTOMATIC_ACTIVATION: u8 = 0x01;
pub const GEAR_FEATURES_RESERVED_MASK: u8 = 0x3E;
pub const GEAR_FEATURES_STORE_RESERVED_MASK: u8 = 0xFE;
pub const GEAR_FEATURES_AUTO_CALIBRATION: u8 = 0x40;
pub const GEAR_FEATURES_AUTO_CALIBRATION_RECOVERY: u8 = 0x80;
pub const GEAR_FEATURES_POWER_UP_DEFAULT: u8 = GEAR_FEATURES_AUTOMATIC_ACTIVATION;

pub const fn gear_features_store_operand(automatic_activation: bool) -> u8 {
    if automatic_activation {
        GEAR_FEATURES_AUTOMATIC_ACTIVATION
    } else {
        0x00
    }
}

pub const fn gear_features_automatic_activation(features: u8) -> bool {
    features & GEAR_FEATURES_AUTOMATIC_ACTIVATION != 0
}

pub const RGBWAF_CONTROL_TYPE_MASK: u8 = 0xC0;
pub const RGBWAF_CONTROL_CHANNEL_CONTROL: u8 = 0x00;
pub const RGBWAF_CONTROL_COLOUR_CONTROL: u8 = 0x40;
pub const RGBWAF_CONTROL_NORMALISED: u8 = 0x80;
pub const RGBWAF_CONTROL_RED: u8 = 0x01;
pub const RGBWAF_CONTROL_GREEN: u8 = 0x02;
pub const RGBWAF_CONTROL_BLUE: u8 = 0x04;
pub const RGBWAF_CONTROL_WHITE: u8 = 0x08;
pub const RGBWAF_CONTROL_AMBER: u8 = 0x10;
pub const RGBWAF_CONTROL_FREECOLOUR: u8 = 0x20;
pub const RGBWAF_CONTROL_RGB_CHANNELS: u8 =
    RGBWAF_CONTROL_RED | RGBWAF_CONTROL_GREEN | RGBWAF_CONTROL_BLUE;
pub const RGBWAF_CONTROL_ALL_CHANNELS: u8 = 0x3F;
pub const RGBWAF_CONTROL_MASK: u8 = 0xFF;

// IEC 62386-209 Table 8
pub const RGBWAF_CONTROL_POWER_UP_DEFAULT: u8 = 0x3F;

// IEC 62386-209 §9.1
pub const fn rgbwaf_control_operand() -> u8 {
    RGBWAF_CONTROL_NORMALISED
}

// IEC 62386-209 §9.1
pub const fn rgbwaf_control_drives(answer: u8, driven: u8) -> bool {
    answer & driven == 0
}

pub const fn rgbwaf_control_is_target(answer: u8, driven: u8) -> bool {
    answer & RGBWAF_CONTROL_TYPE_MASK == RGBWAF_CONTROL_NORMALISED && answer & driven == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    XyCoordinate = 0,
    ColorTemperature = 1,
    PrimaryN = 2,
    Rgbwaf = 3,
}

impl Dt8Command {
    pub const fn opcode(&self) -> u8 {
        match self {
            Self::SetTemporaryXCoordinate => 0xE0,
            Self::SetTemporaryYCoordinate => 0xE1,
            Self::Activate => 0xE2,
            Self::XCoordinateStepUp => 0xE3,
            Self::XCoordinateStepDown => 0xE4,
            Self::YCoordinateStepUp => 0xE5,
            Self::YCoordinateStepDown => 0xE6,
            Self::SetTemporaryColourTemperature => 0xE7,
            Self::ColourTemperatureStepCooler => 0xE8,
            Self::ColourTemperatureStepWarmer => 0xE9,
            Self::SetTemporaryPrimaryNDimLevel => 0xEA,
            Self::SetTemporaryRgbDimLevel => 0xEB,
            Self::SetTemporaryWafDimLevel => 0xEC,
            Self::SetTemporaryRgbwafControl => 0xED,
            Self::CopyReportToTemporary => 0xEE,
            Self::StoreColourTemperatureTcLimit => 0xF2,
            Self::StoreGearFeaturesStatus => 0xF3,
            Self::QueryGearFeaturesStatus => 0xF7,
            Self::QueryColourStatus => 0xF8,
            Self::QueryColourTypeFeatures => 0xF9,
            Self::QueryColourValue => 0xFA,
            Self::QueryRgbwafControl => 0xFB,
        }
    }

    pub fn from_opcode(opcode: u8) -> Option<Self> {
        match opcode {
            0xE0 => Some(Self::SetTemporaryXCoordinate),
            0xE1 => Some(Self::SetTemporaryYCoordinate),
            0xE2 => Some(Self::Activate),
            0xE3 => Some(Self::XCoordinateStepUp),
            0xE4 => Some(Self::XCoordinateStepDown),
            0xE5 => Some(Self::YCoordinateStepUp),
            0xE6 => Some(Self::YCoordinateStepDown),
            0xE7 => Some(Self::SetTemporaryColourTemperature),
            0xE8 => Some(Self::ColourTemperatureStepCooler),
            0xE9 => Some(Self::ColourTemperatureStepWarmer),
            0xEA => Some(Self::SetTemporaryPrimaryNDimLevel),
            0xEB => Some(Self::SetTemporaryRgbDimLevel),
            0xEC => Some(Self::SetTemporaryWafDimLevel),
            0xED => Some(Self::SetTemporaryRgbwafControl),
            0xEE => Some(Self::CopyReportToTemporary),
            0xF2 => Some(Self::StoreColourTemperatureTcLimit),
            0xF3 => Some(Self::StoreGearFeaturesStatus),
            0xF7 => Some(Self::QueryGearFeaturesStatus),
            0xF8 => Some(Self::QueryColourStatus),
            0xF9 => Some(Self::QueryColourTypeFeatures),
            0xFA => Some(Self::QueryColourValue),
            0xFB => Some(Self::QueryRgbwafControl),
            _ => None,
        }
    }
}

impl DeviceCommandMetadata for Dt8Command {
    fn metadata(&self) -> CommandMetadata {
        CommandMetadata {
            device_type: Some(DeviceType::Color),
            opcode: self.opcode(),
            expects_backward: self.is_query(),
            requires_repeat: opcode_requires_repeat(self.opcode()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dt8_from_opcode_decodes_all_public_variants() {
        assert_eq!(
            Dt8Command::from_opcode(0xF8),
            Some(Dt8Command::QueryColourStatus)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xF9),
            Some(Dt8Command::QueryColourTypeFeatures)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xFA),
            Some(Dt8Command::QueryColourValue)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xF3),
            Some(Dt8Command::StoreGearFeaturesStatus)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xF7),
            Some(Dt8Command::QueryGearFeaturesStatus)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xFB),
            Some(Dt8Command::QueryRgbwafControl)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xEE),
            Some(Dt8Command::CopyReportToTemporary)
        );
        assert_eq!(
            Dt8Command::from_opcode(0xF2),
            Some(Dt8Command::StoreColourTemperatureTcLimit)
        );
        assert_eq!(Dt8Command::from_opcode(0xF0), None);
        assert_eq!(Dt8Command::from_opcode(0xF1), None);
        assert_eq!(Dt8Command::from_opcode(0xF5), None);
        assert_eq!(Dt8Command::from_opcode(0xF6), None);
        assert_eq!(Dt8Command::from_opcode(0xFC), None);
    }

    #[test]
    fn dt8_step_commands_are_single_frame_controls() {
        for (cmd, opcode) in [
            (Dt8Command::XCoordinateStepUp, 0xE3u8),
            (Dt8Command::XCoordinateStepDown, 0xE4),
            (Dt8Command::YCoordinateStepUp, 0xE5),
            (Dt8Command::YCoordinateStepDown, 0xE6),
            (Dt8Command::ColourTemperatureStepCooler, 0xE8),
            (Dt8Command::ColourTemperatureStepWarmer, 0xE9),
            (Dt8Command::CopyReportToTemporary, 0xEE),
        ] {
            let meta = cmd.metadata();
            assert_eq!(meta.opcode, opcode);
            assert_eq!(Dt8Command::from_opcode(opcode), Some(cmd));
            assert!(!meta.requires_repeat, "{cmd:?} is a control command");
            assert!(!meta.expects_backward, "{cmd:?} carries no backward frame");
            assert!(!cmd.is_query());
        }
    }

    #[test]
    fn store_tc_limit_is_a_send_twice_configuration_command() {
        let store = Dt8Command::StoreColourTemperatureTcLimit.metadata();
        assert_eq!(store.opcode, 0xF2);
        assert!(store.requires_repeat, "242 is a configuration command");
        assert!(!store.expects_backward, "242 carries no backward frame");
        assert_eq!(store.device_type, Some(DeviceType::Color));

        assert_eq!(TC_LIMIT_SELECTOR_COOLEST, 0);
        assert_eq!(TC_LIMIT_SELECTOR_WARMEST, 1);
        assert_eq!(TC_LIMIT_SELECTOR_PHYSICAL_COOLEST, 2);
        assert_eq!(TC_LIMIT_SELECTOR_PHYSICAL_WARMEST, 3);
    }

    #[test]
    fn every_table_11_identifier_has_a_width_and_nothing_else_answers() {
        let wide: &[u8] = &[
            COLOUR_VALUE_X,
            COLOUR_VALUE_Y,
            COLOUR_VALUE_TC,
            3, 4, 5, 6, 7, 8,
            64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80,
            81,
            COLOUR_VALUE_TC_COOLEST,
            COLOUR_VALUE_TC_PHYSICAL_COOLEST,
            COLOUR_VALUE_TC_WARMEST,
            COLOUR_VALUE_TC_PHYSICAL_WARMEST,
            COLOUR_VALUE_TEMPORARY_X,
            COLOUR_VALUE_TEMPORARY_Y,
            COLOUR_VALUE_TEMPORARY_TC,
            195, 196, 197, 198, 199, 200,
            COLOUR_VALUE_REPORT_X,
            COLOUR_VALUE_REPORT_Y,
            COLOUR_VALUE_REPORT_TC,
            227, 228, 229, 230, 231, 232,
        ];
        let narrow: &[u8] = &[
            COLOUR_VALUE_RED,
            COLOUR_VALUE_GREEN,
            COLOUR_VALUE_BLUE,
            COLOUR_VALUE_WHITE,
            COLOUR_VALUE_AMBER,
            COLOUR_VALUE_FREECOLOUR,
            COLOUR_VALUE_RGBWAF_CONTROL,
            COLOUR_VALUE_NUMBER_OF_PRIMARIES,
            201, 202, 203, 204, 205, 206,
            COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL,
            COLOUR_VALUE_TEMPORARY_COLOUR_TYPE,
            COLOUR_VALUE_REPORT_RED,
            COLOUR_VALUE_REPORT_GREEN,
            COLOUR_VALUE_REPORT_BLUE,
            COLOUR_VALUE_REPORT_WHITE,
            COLOUR_VALUE_REPORT_AMBER,
            COLOUR_VALUE_REPORT_FREECOLOUR,
            COLOUR_VALUE_REPORT_RGBWAF_CONTROL,
            COLOUR_VALUE_REPORT_COLOUR_TYPE,
        ];
        for id in 0..=255u8 {
            let expect_wide = wide.contains(&id);
            let expect_defined = expect_wide || narrow.contains(&id);
            assert_eq!(
                colour_value_is_wide(id),
                expect_wide,
                "identifier {id} width against Table 8"
            );
            assert_eq!(
                colour_value_is_defined(id),
                expect_defined,
                "identifier {id} presence in Table 11"
            );
        }
        assert_eq!(wide.len(), 49, "Table 11 sixteen-bit rows");
        assert_eq!(narrow.len(), 24, "Table 11 one-byte rows");
    }

    #[test]
    fn rgbwaf_commands_are_control_not_configuration() {
        let waf = Dt8Command::SetTemporaryWafDimLevel.metadata();
        assert_eq!(waf.opcode, 0xEC);
        assert!(!waf.requires_repeat, "236 is a control command");
        assert!(!waf.expects_backward);

        let control = Dt8Command::SetTemporaryRgbwafControl.metadata();
        assert_eq!(control.opcode, 0xED);
        assert!(!control.requires_repeat, "237 is a control command");
        assert!(!control.expects_backward, "237 carries no backward frame");

        let query = Dt8Command::QueryRgbwafControl.metadata();
        assert_eq!(query.opcode, 0xFB);
        assert!(!query.requires_repeat);
        assert!(query.expects_backward, "251 answers");
    }

    #[test]
    fn a_colour_that_appears_is_not_the_same_as_the_state_we_ask_for() {
        assert_eq!(rgbwaf_control_operand(), 0x80);

        assert!(rgbwaf_control_drives(0x80, RGBWAF_CONTROL_RGB_CHANNELS));
        assert!(rgbwaf_control_is_target(0x80, RGBWAF_CONTROL_RGB_CHANNELS));

        assert!(rgbwaf_control_drives(0xC0, RGBWAF_CONTROL_RGB_CHANNELS));
        assert!(!rgbwaf_control_is_target(0xC0, RGBWAF_CONTROL_RGB_CHANNELS));

        assert!(!rgbwaf_control_drives(
            RGBWAF_CONTROL_POWER_UP_DEFAULT,
            RGBWAF_CONTROL_RGB_CHANNELS
        ));

        assert!(rgbwaf_control_drives(0xB8, RGBWAF_CONTROL_RGB_CHANNELS));
        assert!(!rgbwaf_control_drives(0xB8, RGBWAF_CONTROL_ALL_CHANNELS));
    }

    #[test]
    fn gear_features_store_repeats_and_the_query_answers() {
        let store = Dt8Command::StoreGearFeaturesStatus.metadata();
        assert_eq!(store.opcode, 0xF3);
        assert!(store.requires_repeat, "243 is a configuration command");
        assert!(!store.expects_backward, "243 carries no backward frame");

        let query = Dt8Command::QueryGearFeaturesStatus.metadata();
        assert_eq!(query.opcode, 0xF7);
        assert!(!query.requires_repeat, "247 is a query");
        assert!(query.expects_backward);
    }

    #[test]
    fn a_store_operand_never_carries_a_queried_feature_bit() {
        assert_eq!(gear_features_store_operand(true), 0x01);
        assert_eq!(gear_features_store_operand(false), 0x00);

        let answered = GEAR_FEATURES_AUTO_CALIBRATION
            | GEAR_FEATURES_AUTO_CALIBRATION_RECOVERY
            | GEAR_FEATURES_AUTOMATIC_ACTIVATION;
        assert!(gear_features_automatic_activation(answered));
        assert_eq!(
            gear_features_store_operand(gear_features_automatic_activation(answered)),
            GEAR_FEATURES_AUTOMATIC_ACTIVATION,
            "a round trip through the read byte must not widen the operand"
        );
        assert_eq!(answered & GEAR_FEATURES_RESERVED_MASK, 0);
        assert!(!gear_features_automatic_activation(
            GEAR_FEATURES_AUTO_CALIBRATION
        ));
    }

    #[test]
    fn dt8_temporary_set_opcodes_are_writes() {
        for set_opcode in [0xE0u8, 0xE1, 0xE2, 0xE7, 0xEA, 0xEB] {
            let cmd = Dt8Command::from_opcode(set_opcode)
                .unwrap_or_else(|| panic!("opcode {set_opcode:#04X} should decode"));
            assert!(
                !cmd.is_query(),
                "opcode {set_opcode:#04X} is a temporary-colour set, not a query"
            );
            assert!(!cmd.metadata().expects_backward);
        }
    }

    #[test]
    fn dt8_queries_all_expect_a_backward_frame() {
        let query = Dt8Command::QueryColourValue.metadata();
        assert_eq!(query.device_type, Some(DeviceType::Color));
        assert_eq!(query.opcode, 0xFA);
        assert!(query.expects_backward);
        assert!(!query.requires_repeat);
    }

    #[test]
    fn dt8_configuration_commands_are_send_twice_and_nothing_else_is() {
        for opcode in 0x00..=0xFFu8 {
            assert_eq!(
                opcode_requires_repeat(opcode),
                (0xEF..=0xF6).contains(&opcode),
                "opcode {opcode:#04X} against IEC 62386-209 §11.3.4.2 (commands 239 to 246)"
            );
        }
        assert!(!opcode_requires_repeat(0xEE), "238 is a control command");
        assert!(!opcode_requires_repeat(0xF7), "247 is a query");

        for control in [
            Dt8Command::SetTemporaryXCoordinate,
            Dt8Command::SetTemporaryYCoordinate,
            Dt8Command::Activate,
            Dt8Command::SetTemporaryColourTemperature,
            Dt8Command::SetTemporaryPrimaryNDimLevel,
            Dt8Command::SetTemporaryRgbDimLevel,
        ] {
            assert!(
                !control.metadata().requires_repeat,
                "{control:?} is a control command, not a configuration one"
            );
        }
    }

    fn srgb_eotf(code: u8) -> f64 {
        let c = f64::from(code) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    #[test]
    fn the_encoder_table_is_the_srgb_curve_it_claims_to_be() {
        for code in 0..=255u8 {
            let want = if code == 0 {
                0
            } else {
                ((srgb_eotf(code) * 254.0).round() as u8).max(1)
            };
            assert_eq!(
                srgb_channel_to_dim_level(code),
                want,
                "sRGB {code} against the piecewise EOTF scaled to 0..254"
            );
        }
    }

    #[test]
    fn both_directions_are_monotone_and_pin_their_endpoints() {
        for code in 0..255u8 {
            assert!(
                srgb_channel_to_dim_level(code) <= srgb_channel_to_dim_level(code + 1),
                "encoder must not fall at sRGB {code}"
            );
        }
        for dim in 0..254u8 {
            assert!(
                dim_level_to_srgb_channel(dim) <= dim_level_to_srgb_channel(dim + 1),
                "decoder must not fall at dim level {dim}"
            );
        }
        assert_eq!(srgb_channel_to_dim_level(0), 0);
        assert_eq!(dim_level_to_srgb_channel(0), 0);
        assert_eq!(srgb_channel_to_dim_level(255), 254);
        assert_eq!(dim_level_to_srgb_channel(254), 255);
        assert_eq!(dim_level_to_srgb_channel(COLOUR_VALUE_LEVEL_MASK), COLOUR_VALUE_LEVEL_MASK);
    }

    #[test]
    fn a_little_colour_is_never_silently_off() {
        for code in 1..=255u8 {
            assert!(
                srgb_channel_to_dim_level(code) >= 1,
                "sRGB {code} must not encode to darkness"
            );
        }
        assert_eq!(srgb_channel_to_dim_level(0), 0);
    }

    #[test]
    fn the_decoder_is_a_right_inverse_on_the_image_of_the_encoder() {
        for code in 0..=255u8 {
            let wire = srgb_channel_to_dim_level(code);
            assert_eq!(
                srgb_channel_to_dim_level(dim_level_to_srgb_channel(wire)),
                wire,
                "dim level {wire} (from sRGB {code}) must decode to one of its own preimages"
            );
        }
        for dim in 0..=254u8 {
            let round = i16::from(srgb_channel_to_dim_level(dim_level_to_srgb_channel(dim)));
            assert!(
                (round - i16::from(dim)).abs() <= 1,
                "dim level {dim} is off-image; the decoder owes the nearest attainable, got {round}"
            );
        }
    }

    #[test]
    fn above_the_knee_the_round_trip_is_exact_in_product_space() {
        for code in 123..=255u8 {
            assert_eq!(
                dim_level_to_srgb_channel(srgb_channel_to_dim_level(code)),
                code,
                "sRGB {code} is above the knee and must survive a round trip"
            );
        }
    }

    #[test]
    fn the_bench_measurement_of_2026_08_20_is_pinned() {
        assert_eq!(srgb_channel_to_dim_level(120), 48);
        assert_eq!(srgb_channel_to_dim_level(0), 0);
        assert_eq!(
            [
                srgb_channel_to_dim_level(255),
                srgb_channel_to_dim_level(180),
                srgb_channel_to_dim_level(90)
            ],
            [254, 116, 26]
        );
        assert_eq!(
            [
                dim_level_to_srgb_channel(254),
                dim_level_to_srgb_channel(116),
                dim_level_to_srgb_channel(26)
            ],
            [255, 180, 90]
        );
    }
}
