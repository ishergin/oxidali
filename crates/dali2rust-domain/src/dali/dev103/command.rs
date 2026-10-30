use crate::dali::dev103::address::{Device103Address, InstanceAddress};
use crate::dali::dev103::frame::ForwardFrame24;

macro_rules! opcode_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($variant:ident = $code:literal,)* }) => {
        $(#[$meta])*
        #[repr(u8)]
        $vis enum $name {
            $($variant = $code,)*
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }
    };
}

const FIRST_DEVICE_QUERY: u8 = 0x30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command103Metadata {
    pub opcode: u8,
    pub expects_backward: bool,
    pub send_twice: bool,
    pub uses_dtr0: bool,
}

opcode_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Device103Command {
        IdentifyDevice = 0x00,
        ResetPowerCycleSeen = 0x01,
        Reset = 0x10,
        ResetMemoryBank = 0x11,
        SetShortAddress = 0x14,
        EnableWriteMemory = 0x15,
        EnableApplicationController = 0x16,
        DisableApplicationController = 0x17,
        SetOperatingMode = 0x18,
        AddToDeviceGroups0To15 = 0x19,
        AddToDeviceGroups16To31 = 0x1A,
        RemoveFromDeviceGroups0To15 = 0x1B,
        RemoveFromDeviceGroups16To31 = 0x1C,
        StartQuiescentMode = 0x1D,
        StopQuiescentMode = 0x1E,
        EnablePowerCycleNotification = 0x1F,
        DisablePowerCycleNotification = 0x20,
        SavePersistentVariables = 0x21,
        QueryDeviceStatus = 0x30,
        QueryApplicationControllerError = 0x31,
        QueryInputDeviceError = 0x32,
        QueryMissingShortAddress = 0x33,
        QueryVersionNumber = 0x34,
        QueryNumberOfInstances = 0x35,
        QueryContentDtr0 = 0x36,
        QueryContentDtr1 = 0x37,
        QueryContentDtr2 = 0x38,
        QueryRandomAddressH = 0x39,
        QueryRandomAddressM = 0x3A,
        QueryRandomAddressL = 0x3B,
        ReadMemoryLocation = 0x3C,
        QueryApplicationControlEnabled = 0x3D,
        QueryOperatingMode = 0x3E,
        QueryManufacturerSpecificMode = 0x3F,
        QueryQuiescentMode = 0x40,
        QueryDeviceGroups0To7 = 0x41,
        QueryDeviceGroups8To15 = 0x42,
        QueryDeviceGroups16To23 = 0x43,
        QueryDeviceGroups24To31 = 0x44,
        QueryPowerCycleNotification = 0x45,
        QueryDeviceCapabilities = 0x46,
        QueryExtendedVersionNumber = 0x47,
        QueryResetState = 0x48,
    }
}

impl Device103Command {
    // IEC 62386-103 Table 21
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let opcode = self as u8;
        let expects_backward = opcode >= FIRST_DEVICE_QUERY;
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice: !expects_backward,
            uses_dtr0: matches!(
                self,
                Self::ResetMemoryBank
                    | Self::SetShortAddress
                    | Self::SetOperatingMode
                    | Self::ReadMemoryLocation
                    | Self::QueryExtendedVersionNumber
            ),
        }
    }

    #[must_use]
    pub fn from_opcode(opcode: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|command| *command as u8 == opcode)
    }

    #[must_use]
    pub const fn frame(self, address: Device103Address) -> ForwardFrame24 {
        ForwardFrame24::command(address, InstanceAddress::Device, self.metadata().opcode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instance103Command {
    SetEventPriority,
    EnableInstance,
    DisableInstance,
    SetPrimaryInstanceGroup,
    SetInstanceGroup1,
    SetInstanceGroup2,
    SetEventScheme,
    SetEventFilter,
    QueryInstanceType,
    QueryResolution,
    QueryInstanceError,
    QueryInstanceStatus,
    QueryEventPriority,
    QueryInstanceEnabled,
    QueryPrimaryInstanceGroup,
    QueryInstanceGroup1,
    QueryInstanceGroup2,
    QueryEventScheme,
    QueryInputValue,
    QueryInputValueLatch,
    QueryFeatureType,
    QueryNextFeatureType,
    QueryEventFilter0To7,
    QueryEventFilter8To15,
    QueryEventFilter16To23,
}

impl Instance103Command {
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let (opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::SetEventPriority => (0x61, false, true, true),
            Self::EnableInstance => (0x62, false, true, false),
            Self::DisableInstance => (0x63, false, true, false),
            Self::SetPrimaryInstanceGroup => (0x64, false, true, true),
            Self::SetInstanceGroup1 => (0x65, false, true, true),
            Self::SetInstanceGroup2 => (0x66, false, true, true),
            Self::SetEventScheme => (0x67, false, true, true),
            Self::SetEventFilter => (0x68, false, true, true),
            Self::QueryInstanceType => (0x80, true, false, false),
            Self::QueryResolution => (0x81, true, false, false),
            Self::QueryInstanceError => (0x82, true, false, false),
            Self::QueryInstanceStatus => (0x83, true, false, false),
            Self::QueryEventPriority => (0x84, true, false, false),
            Self::QueryInstanceEnabled => (0x86, true, false, false),
            Self::QueryPrimaryInstanceGroup => (0x88, true, false, false),
            Self::QueryInstanceGroup1 => (0x89, true, false, false),
            Self::QueryInstanceGroup2 => (0x8A, true, false, false),
            Self::QueryEventScheme => (0x8B, true, false, false),
            Self::QueryInputValue => (0x8C, true, false, false),
            Self::QueryInputValueLatch => (0x8D, true, false, false),
            Self::QueryFeatureType => (0x8E, true, false, false),
            Self::QueryNextFeatureType => (0x8F, true, false, false),
            Self::QueryEventFilter0To7 => (0x90, true, false, false),
            Self::QueryEventFilter8To15 => (0x91, true, false, false),
            Self::QueryEventFilter16To23 => (0x92, true, false, false),
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }

    #[must_use]
    pub const fn frame(
        self,
        address: Device103Address,
        instance: InstanceAddress,
    ) -> ForwardFrame24 {
        ForwardFrame24::command(address, instance, self.metadata().opcode)
    }
}

opcode_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Special103Command {
        Terminate = 0x00,
        Initialise = 0x01,
        Randomise = 0x02,
        Compare = 0x03,
        Withdraw = 0x04,
        SearchAddrH = 0x05,
        SearchAddrM = 0x06,
        SearchAddrL = 0x07,
        ProgramShortAddress = 0x08,
        VerifyShortAddress = 0x09,
        QueryShortAddress = 0x0A,
        WriteMemoryLocation = 0x20,
        WriteMemoryLocationNoReply = 0x21,
        Dtr0 = 0x30,
        Dtr1 = 0x31,
        Dtr2 = 0x32,
        SendTestframe = 0x33,
    }
}

impl Special103Command {
    // IEC 62386-103 Table 22
    #[must_use]
    pub const fn selector(self) -> (u8, bool, bool) {
        let expects_backward = matches!(
            self,
            Self::Compare
                | Self::VerifyShortAddress
                | Self::QueryShortAddress
                | Self::WriteMemoryLocation
        );
        let send_twice = matches!(self, Self::Initialise | Self::Randomise);
        (self as u8, expects_backward, send_twice)
    }

    #[must_use]
    pub fn from_selector(selector: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|command| *command as u8 == selector)
    }

    #[must_use]
    pub const fn frame(self, data: u8) -> ForwardFrame24 {
        ForwardFrame24::special(self.selector().0, data)
    }

    #[must_use]
    pub const fn expects_backward(self) -> bool {
        self.selector().1
    }

    #[must_use]
    pub const fn send_twice(self) -> bool {
        self.selector().2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button301Command {
    SetShortTimer,
    SetDoubleTimer,
    SetRepeatTimer,
    SetStuckTimer,
    QueryShortTimer,
    QueryShortTimerMin,
    QueryDoubleTimer,
    QueryDoubleTimerMin,
    QueryRepeatTimer,
    QueryStuckTimer,
}

impl Button301Command {
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let (opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::SetShortTimer => (0x00, false, true, true),
            Self::SetDoubleTimer => (0x01, false, true, true),
            Self::SetRepeatTimer => (0x02, false, true, true),
            Self::SetStuckTimer => (0x03, false, true, true),
            Self::QueryShortTimer => (0x0A, true, false, false),
            Self::QueryShortTimerMin => (0x0B, true, false, false),
            Self::QueryDoubleTimer => (0x0C, true, false, false),
            Self::QueryDoubleTimerMin => (0x0D, true, false, false),
            Self::QueryRepeatTimer => (0x0E, true, false, false),
            Self::QueryStuckTimer => (0x0F, true, false, false),
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy303Command {
    CatchMovement,
    SetHoldTimer,
    SetReportTimer,
    SetDeadtimeTimer,
    CancelHoldTimer,
    SetDetectionRange,
    SetSensitivity,
    QueryInstanceCapabilities,
    QueryDetectionRange,
    QuerySensitivity,
    QueryDeadtimeTimer,
    QueryHoldTimer,
    QueryReportTimer,
    QueryCatching,
}

impl Occupancy303Command {
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let (opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::CatchMovement => (0x20, false, false, false),
            Self::SetHoldTimer => (0x21, false, true, true),
            Self::SetReportTimer => (0x22, false, true, true),
            Self::SetDeadtimeTimer => (0x23, false, true, true),
            Self::CancelHoldTimer => (0x24, false, false, false),
            Self::SetDetectionRange => (0x25, false, true, true),
            Self::SetSensitivity => (0x26, false, true, true),
            Self::QueryInstanceCapabilities => (0x29, true, false, false),
            Self::QueryDetectionRange => (0x2A, true, false, false),
            Self::QuerySensitivity => (0x2B, true, false, false),
            Self::QueryDeadtimeTimer => (0x2C, true, false, false),
            Self::QueryHoldTimer => (0x2D, true, false, false),
            Self::QueryReportTimer => (0x2E, true, false, false),
            Self::QueryCatching => (0x2F, true, false, false),
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }

    #[must_use]
    pub const fn frame(
        self,
        address: Device103Address,
        instance: InstanceAddress,
    ) -> ForwardFrame24 {
        ForwardFrame24::command(address, instance, self.metadata().opcode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightSensor304Command {
    SetReportTimer,
    SetHysteresis,
    SetDeadtimeTimer,
    SetHysteresisMin,
    QueryHysteresisMin,
    QueryDeadtimeTimer,
    QueryReportTimer,
    QueryHysteresis,
}

impl LightSensor304Command {
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let (opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::SetReportTimer => (0x30, false, true, true),
            Self::SetHysteresis => (0x31, false, true, true),
            Self::SetDeadtimeTimer => (0x32, false, true, true),
            Self::SetHysteresisMin => (0x33, false, true, true),
            Self::QueryHysteresisMin => (0x3C, true, false, false),
            Self::QueryDeadtimeTimer => (0x3D, true, false, false),
            Self::QueryReportTimer => (0x3E, true, false, false),
            Self::QueryHysteresis => (0x3F, true, false, false),
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsoluteInput302Command {
    SetReportTimer,
    SetDeadtimeTimer,
    QueryDeadtimeTimer,
    QueryReportTimer,
    QuerySwitch,
}

impl AbsoluteInput302Command {
    #[must_use]
    pub const fn metadata(self) -> Command103Metadata {
        let (opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::SetReportTimer => (0x10, false, true, true),
            Self::SetDeadtimeTimer => (0x11, false, true, true),
            Self::QueryDeadtimeTimer => (0x1D, true, false, false),
            Self::QueryReportTimer => (0x1E, true, false, false),
            Self::QuerySwitch => (0x1F, true, false, false),
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackOpcodeMap {
    DiiaCorrected,
    Ed1,
}

const FEEDBACK_MAP_ABSENT_CODE: u8 = 0;
const FEEDBACK_MAP_DIIA_CORRECTED_CODE: u8 = 1;
const FEEDBACK_MAP_ED1_CODE: u8 = 2;

impl FeedbackOpcodeMap {
    pub const ABSENT_CODE: u8 = FEEDBACK_MAP_ABSENT_CODE;

    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::DiiaCorrected => FEEDBACK_MAP_DIIA_CORRECTED_CODE,
            Self::Ed1 => FEEDBACK_MAP_ED1_CODE,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            FEEDBACK_MAP_DIIA_CORRECTED_CODE => Some(Self::DiiaCorrected),
            FEEDBACK_MAP_ED1_CODE => Some(Self::Ed1),
            _ => None,
        }
    }
}

const FEEDBACK_QUERY_RELOCATION: u8 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback332Command {
    Activate,
    Stop,
    Select(u8),
    SetTiming,
    SetActiveBrightness,
    SetActiveColour,
    SetInactiveBrightness,
    SetInactiveColour,
    SetActiveVolume,
    SetActivePitch,
    QueryCapability,
    QueryActive,
    QueryTiming,
    QueryActiveBrightness,
    QueryActiveColour,
    QueryInactiveBrightness,
    QueryInactiveColour,
    QueryActiveVolume,
    QueryActivePitch,
    QueryColourCapability,
}

impl Feedback332Command {
    #[must_use]
    pub const fn metadata(self, map: FeedbackOpcodeMap) -> Command103Metadata {
        let (corrected_opcode, expects_backward, send_twice, uses_dtr0) = match self {
            Self::Activate => (0x10, false, false, false),
            Self::Stop => (0x11, false, false, false),
            Self::Select(group) => (0x20 + (group & 0x1F), false, false, false),
            Self::SetTiming => (0x12, false, true, true),
            Self::SetActiveBrightness => (0x13, false, true, true),
            Self::SetActiveColour => (0x14, false, true, true),
            Self::SetInactiveBrightness => (0x15, false, true, true),
            Self::SetInactiveColour => (0x16, false, true, true),
            Self::SetActiveVolume => (0x17, false, true, true),
            Self::SetActivePitch => (0x18, false, true, true),
            Self::QueryCapability => (0x4F, true, false, false),
            Self::QueryActive => (0x4E, true, false, false),
            Self::QueryTiming => (0x4D, true, false, false),
            Self::QueryActiveBrightness => (0x4C, true, false, false),
            Self::QueryActiveColour => (0x4B, true, false, false),
            Self::QueryInactiveBrightness => (0x4A, true, false, false),
            Self::QueryInactiveColour => (0x49, true, false, false),
            Self::QueryActiveVolume => (0x48, true, false, false),
            Self::QueryActivePitch => (0x47, true, false, false),
            Self::QueryColourCapability => (0x46, true, false, false),
        };
        let opcode = match (map, expects_backward) {
            (FeedbackOpcodeMap::Ed1, true) => corrected_opcode - FEEDBACK_QUERY_RELOCATION,
            _ => corrected_opcode,
        };
        Command103Metadata {
            opcode,
            expects_backward,
            send_twice,
            uses_dtr0,
        }
    }

    #[must_use]
    pub const fn frame(
        self,
        address: Device103Address,
        feature: InstanceAddress,
        map: FeedbackOpcodeMap,
    ) -> ForwardFrame24 {
        ForwardFrame24::command(address, feature, self.metadata(map).opcode)
    }
}

pub mod feedback_capability {
    pub const VISIBLE: u8 = 1 << 0;
    pub const BRIGHTNESS: u8 = 1 << 1;
    pub const COLOUR: u8 = 1 << 2;
    pub const AUDIBLE: u8 = 1 << 3;
    pub const VOLUME: u8 = 1 << 4;
    pub const PITCH: u8 = 1 << 5;
    pub const COMMON_BRIGHTNESS: u8 = 1 << 6;
}

pub mod feedback_colour_capability {
    pub const RED: u8 = 1 << 0;
    pub const GREEN: u8 = 1 << 1;
    pub const BLUE: u8 = 1 << 2;
    pub const RESOLUTION_2BIT: u8 = 1 << 3;
    pub const MIXING: u8 = 1 << 4;
    pub const COMMON_COLOUR: u8 = 1 << 5;
}

pub const FEEDBACK_COLOUR_MIN: u8 = 1;
pub const FEEDBACK_COLOUR_MAX: u8 = 63;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::dev103::address::MAX_SHORT_ADDRESS;
    use FeedbackOpcodeMap::{DiiaCorrected, Ed1};

    #[test]
    fn the_opcode_map_codes_are_the_values_the_registry_and_the_commands_carry() {
        assert_eq!((DiiaCorrected.code(), Ed1.code()), (1, 2));
        assert_eq!(FeedbackOpcodeMap::from_code(1), Some(DiiaCorrected));
        assert_eq!(FeedbackOpcodeMap::from_code(2), Some(Ed1));
        assert_eq!(FeedbackOpcodeMap::ABSENT_CODE, 0);
        assert_eq!(
            FeedbackOpcodeMap::from_code(FeedbackOpcodeMap::ABSENT_CODE),
            None,
            "the probe's own answer: no feedback"
        );
        assert_eq!(FeedbackOpcodeMap::from_code(3), None);
    }

    #[test]
    fn a_device_command_always_carries_the_device_instance_byte() {
        for address in [
            Device103Address::Short(0),
            Device103Address::Short(MAX_SHORT_ADDRESS),
            Device103Address::Group(7),
            Device103Address::Broadcast,
        ] {
            let frame = Device103Command::QueryDeviceStatus.frame(address);
            assert_eq!(
                frame.as_bytes()[1],
                0xFE,
                "§9.5.2: a device command with any other instance byte is not accepted"
            );
            assert!(frame.is_command());
        }
    }

    #[test]
    fn no_query_is_send_twice_and_every_configuration_write_is() {
        let queries = [
            Device103Command::QueryDeviceStatus.metadata(),
            Device103Command::QueryNumberOfInstances.metadata(),
            Instance103Command::QueryInstanceType.metadata(),
            Instance103Command::QueryEventScheme.metadata(),
            Button301Command::QueryShortTimerMin.metadata(),
            Occupancy303Command::QueryHoldTimer.metadata(),
            LightSensor304Command::QueryHysteresis.metadata(),
            AbsoluteInput302Command::QuerySwitch.metadata(),
        ];
        for meta in queries {
            assert!(!meta.send_twice, "a query is never send-twice: {meta:?}");
            assert!(meta.expects_backward);
        }

        let writes = [
            Instance103Command::SetEventScheme.metadata(),
            Instance103Command::SetEventFilter.metadata(),
            Instance103Command::SetPrimaryInstanceGroup.metadata(),
            Button301Command::SetShortTimer.metadata(),
            Occupancy303Command::SetHoldTimer.metadata(),
            LightSensor304Command::SetHysteresis.metadata(),
            AbsoluteInput302Command::SetReportTimer.metadata(),
        ];
        for meta in writes {
            assert!(meta.send_twice, "a configuration write is send-twice: {meta:?}");
            assert!(meta.uses_dtr0, "and it takes its operand from DTR0");
            assert!(!meta.expects_backward);
        }
    }

    #[test]
    fn the_two_303_control_instructions_are_not_send_twice() {
        for cmd in [
            Occupancy303Command::CatchMovement,
            Occupancy303Command::CancelHoldTimer,
        ] {
            let meta = cmd.metadata();
            assert!(!meta.send_twice, "{cmd:?}");
            assert!(!meta.expects_backward, "{cmd:?}");
        }
    }

    #[test]
    fn instance_opcode_ranges_are_disjoint_per_part() {
        let in_range = |op: u8, lo: u8, hi: u8| (lo..=hi).contains(&op);
        for cmd in [
            Button301Command::SetShortTimer,
            Button301Command::QueryStuckTimer,
        ] {
            assert!(in_range(cmd.metadata().opcode, 0x00, 0x0F), "{cmd:?}");
        }
        for cmd in [
            AbsoluteInput302Command::SetReportTimer,
            AbsoluteInput302Command::QuerySwitch,
        ] {
            assert!(in_range(cmd.metadata().opcode, 0x10, 0x1F), "{cmd:?}");
        }
        for cmd in [
            Occupancy303Command::CatchMovement,
            Occupancy303Command::QueryCatching,
        ] {
            assert!(in_range(cmd.metadata().opcode, 0x20, 0x2F), "{cmd:?}");
        }
        for cmd in [
            LightSensor304Command::SetReportTimer,
            LightSensor304Command::QueryHysteresis,
        ] {
            assert!(in_range(cmd.metadata().opcode, 0x30, 0x3F), "{cmd:?}");
        }
    }

    #[test]
    fn only_initialise_and_randomise_are_send_twice_specials() {
        for cmd in [Special103Command::Initialise, Special103Command::Randomise] {
            assert!(cmd.send_twice(), "{cmd:?}");
        }
        for cmd in [
            Special103Command::Terminate,
            Special103Command::Compare,
            Special103Command::Withdraw,
            Special103Command::ProgramShortAddress,
            Special103Command::VerifyShortAddress,
            Special103Command::QueryShortAddress,
            Special103Command::Dtr0,
        ] {
            assert!(!cmd.send_twice(), "{cmd:?}");
        }
    }

    #[test]
    fn special_frames_address_0xc1_and_put_the_operand_last() {
        assert_eq!(
            Special103Command::Initialise.frame(0xFF).as_bytes(),
            [0xC1, 0x01, 0xFF]
        );
        assert_eq!(
            Special103Command::ProgramShortAddress.frame(0x03).as_bytes(),
            [0xC1, 0x08, 0x03],
            "the operand is raw 00AAAAAA, not the 102 form"
        );
        assert_eq!(
            Special103Command::Dtr0.frame(0x2A).as_bytes(),
            [0xC1, 0x30, 0x2A]
        );
    }

    #[test]
    fn the_three_commissioning_queries_answer() {
        for cmd in [
            Special103Command::Compare,
            Special103Command::VerifyShortAddress,
            Special103Command::QueryShortAddress,
        ] {
            assert!(cmd.expects_backward(), "{cmd:?}");
        }
    }

    #[test]
    fn every_device_and_instance_opcode_is_unique_within_its_family() {
        let device = Device103Command::ALL;
        let mut opcodes: Vec<u8> = device.iter().map(|c| c.metadata().opcode).collect();
        let count = opcodes.len();
        opcodes.sort_unstable();
        opcodes.dedup();
        assert_eq!(opcodes.len(), count, "duplicate device opcode");
    }

    #[test]
    fn feedback_queries_relocate_and_nothing_else_does() {
        let pairs = [
            (Feedback332Command::QueryCapability, 0x4F, 0x2F),
            (Feedback332Command::QueryActive, 0x4E, 0x2E),
            (Feedback332Command::QueryTiming, 0x4D, 0x2D),
            (Feedback332Command::QueryActiveBrightness, 0x4C, 0x2C),
            (Feedback332Command::QueryActiveColour, 0x4B, 0x2B),
            (Feedback332Command::QueryInactiveBrightness, 0x4A, 0x2A),
            (Feedback332Command::QueryInactiveColour, 0x49, 0x29),
            (Feedback332Command::QueryActiveVolume, 0x48, 0x28),
            (Feedback332Command::QueryActivePitch, 0x47, 0x27),
        ];
        for (cmd, corrected, ed1) in pairs {
            assert_eq!(cmd.metadata(DiiaCorrected).opcode, corrected);
            assert_eq!(cmd.metadata(Ed1).opcode, ed1);
            assert!(cmd.metadata(DiiaCorrected).expects_backward);
        }
        for cmd in [
            Feedback332Command::Activate,
            Feedback332Command::Stop,
            Feedback332Command::Select(9),
            Feedback332Command::SetTiming,
            Feedback332Command::SetActiveColour,
        ] {
            assert_eq!(
                cmd.metadata(DiiaCorrected).opcode,
                cmd.metadata(Ed1).opcode,
                "only queries moved between editions"
            );
        }
    }

    #[test]
    fn every_opcode_names_the_command_that_carries_it() {
        for command in Device103Command::ALL {
            assert_eq!(Device103Command::from_opcode(command.metadata().opcode), Some(*command));
        }
        for command in Special103Command::ALL {
            assert_eq!(Special103Command::from_selector(command.selector().0), Some(*command));
        }
        assert_eq!(Device103Command::from_opcode(0x02), None);
        assert_eq!(Special103Command::from_selector(0x0B), None);
    }

    #[test]
    fn select_feedback_spans_its_block_and_collides_with_ed1_queries() {
        assert_eq!(
            Feedback332Command::Select(0)
                .metadata(FeedbackOpcodeMap::DiiaCorrected)
                .opcode,
            0x20
        );
        assert_eq!(
            Feedback332Command::Select(31)
                .metadata(FeedbackOpcodeMap::DiiaCorrected)
                .opcode,
            0x3F
        );
        assert_eq!(
            Feedback332Command::QueryCapability
                .metadata(FeedbackOpcodeMap::Ed1)
                .opcode,
            Feedback332Command::Select(15)
                .metadata(FeedbackOpcodeMap::DiiaCorrected)
                .opcode,
        );
    }

    #[test]
    fn feedback_frame_and_class_flags() {
        let m = Feedback332Command::SetActiveBrightness.metadata(FeedbackOpcodeMap::DiiaCorrected);
        assert!(m.send_twice && m.uses_dtr0 && !m.expects_backward);
        let m = Feedback332Command::Activate.metadata(FeedbackOpcodeMap::DiiaCorrected);
        assert!(!m.send_twice && !m.uses_dtr0);
        let frame = Feedback332Command::SetActiveBrightness.frame(
            Device103Address::Short(0),
            InstanceAddress::FeatureNumber(0),
            FeedbackOpcodeMap::DiiaCorrected,
        );
        assert_eq!(frame.as_bytes(), [0x01, 0x20, 0x13]);
        let frame = Feedback332Command::Select(4).frame(
            Device103Address::Broadcast,
            InstanceAddress::FeatureGroup(7),
            FeedbackOpcodeMap::DiiaCorrected,
        );
        assert_eq!(frame.as_bytes(), [0xFF, 0xA7, 0x24]);
    }
}
