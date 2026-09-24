use crate::dali::frame::ForwardFrame;
use crate::dali::types::DaliAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardCommand {
    DirectArcPower {
        level: u8,
    },

    Off,
    Up,
    Down,
    StepUp,
    StepDown,
    RecallMaxLevel,
    RecallMinLevel,
    StepDownAndOff,
    OnAndStepUp,
    EnableDapcSequence,
    GoToLastActiveLevel,

    GoToScene {
        scene: u8,
    },

    Reset,
    StoreActualLevelInDtr0,
    SavePersistentVariables,
    SetOperatingMode,
    ResetMemoryBank,
    IdentifyDevice,
    SetMaxLevel,
    SetMinLevel,
    SetSystemFailureLevel,
    SetPowerOnLevel,
    SetFadeTime,
    SetFadeRate,
    SetExtendedFadeTime,

    SetScene {
        scene: u8,
    },
    RemoveScene {
        scene: u8,
    },
    AddToGroup {
        group: u8,
    },
    RemoveFromGroup {
        group: u8,
    },

    SetShortAddress,
    EnableWriteMemory,

    QueryStatus,
    QueryControlGearPresent,
    QueryLampFailure,
    QueryLampPowerOn,
    QueryLimitError,
    QueryResetState,
    QueryMissingShortAddress,
    QueryVersionNumber,
    QueryContentDtr0,
    QueryDeviceType,
    QueryPhysicalMinimum,
    QueryPowerFailure,
    QueryContentDtr1,
    QueryContentDtr2,
    QueryOperatingMode,
    QueryLightSourceType,
    QueryActualLevel,
    QueryMaxLevel,
    QueryMinLevel,
    QueryPowerOnLevel,
    QuerySystemFailureLevel,
    QueryFadeTimeFadeRate,
    QueryManufacturerSpecificMode,
    QueryControlGearFailure,
    QueryExtendedFadeTime,
    QueryNextDeviceType,

    QuerySceneLevel {
        scene: u8,
    },

    QueryGroups0To7,
    QueryGroups8To15,
    QueryRandomAddressH,
    QueryRandomAddressM,
    QueryRandomAddressL,
}

impl StandardCommand {
    pub fn encoding(&self) -> CommandEncoding {
        if let Some(enc) = self.encoding_arc_power() {
            return enc;
        }
        if let Some(enc) = self.encoding_config() {
            return enc;
        }
        if let Some(enc) = self.encoding_query() {
            return enc;
        }
        unreachable!("all StandardCommand variants covered by encoding_* methods")
    }

    fn encoding_arc_power(&self) -> Option<CommandEncoding> {
        Some(match self {
            Self::DirectArcPower { level } => CommandEncoding::DirectArc(*level),
            Self::Off => CommandEncoding::Simple(0x00),
            Self::Up => CommandEncoding::Simple(0x01),
            Self::Down => CommandEncoding::Simple(0x02),
            Self::StepUp => CommandEncoding::Simple(0x03),
            Self::StepDown => CommandEncoding::Simple(0x04),
            Self::RecallMaxLevel => CommandEncoding::Simple(0x05),
            Self::RecallMinLevel => CommandEncoding::Simple(0x06),
            Self::StepDownAndOff => CommandEncoding::Simple(0x07),
            Self::OnAndStepUp => CommandEncoding::Simple(0x08),
            Self::EnableDapcSequence => CommandEncoding::Simple(0x09),
            Self::GoToLastActiveLevel => CommandEncoding::Simple(0x0A),
            Self::GoToScene { scene } => CommandEncoding::Scene(0x10, *scene),
            _ => return None,
        })
    }

    fn encoding_config(&self) -> Option<CommandEncoding> {
        Some(match self {
            Self::Reset => CommandEncoding::Config(0x20),
            Self::StoreActualLevelInDtr0 => CommandEncoding::Config(0x21),
            Self::SavePersistentVariables => CommandEncoding::Config(0x22),
            Self::SetOperatingMode => CommandEncoding::Config(0x23),
            Self::ResetMemoryBank => CommandEncoding::Config(0x24),
            Self::IdentifyDevice => CommandEncoding::Config(0x25),
            Self::SetMaxLevel => CommandEncoding::Config(0x2A),
            Self::SetMinLevel => CommandEncoding::Config(0x2B),
            Self::SetSystemFailureLevel => CommandEncoding::Config(0x2C),
            Self::SetPowerOnLevel => CommandEncoding::Config(0x2D),
            Self::SetFadeTime => CommandEncoding::Config(0x2E),
            Self::SetFadeRate => CommandEncoding::Config(0x2F),
            Self::SetExtendedFadeTime => CommandEncoding::Config(0x30),
            Self::SetScene { scene } => CommandEncoding::ConfigWithParam(0x40, *scene),
            Self::RemoveScene { scene } => CommandEncoding::ConfigWithParam(0x50, *scene),
            Self::AddToGroup { group } => CommandEncoding::ConfigWithParam(0x60, *group),
            Self::RemoveFromGroup { group } => CommandEncoding::ConfigWithParam(0x70, *group),
            Self::SetShortAddress => CommandEncoding::Config(0x80),
            Self::EnableWriteMemory => CommandEncoding::Config(0x81),
            _ => return None,
        })
    }

    fn encoding_query(&self) -> Option<CommandEncoding> {
        Some(match self {
            Self::QueryStatus => CommandEncoding::Query(0x90),
            Self::QueryControlGearPresent => CommandEncoding::Query(0x91),
            Self::QueryLampFailure => CommandEncoding::Query(0x92),
            Self::QueryLampPowerOn => CommandEncoding::Query(0x93),
            Self::QueryLimitError => CommandEncoding::Query(0x94),
            Self::QueryResetState => CommandEncoding::Query(0x95),
            Self::QueryMissingShortAddress => CommandEncoding::Query(0x96),
            Self::QueryVersionNumber => CommandEncoding::Query(0x97),
            Self::QueryContentDtr0 => CommandEncoding::Query(0x98),
            Self::QueryDeviceType => CommandEncoding::Query(0x99),
            Self::QueryPhysicalMinimum => CommandEncoding::Query(0x9A),
            Self::QueryPowerFailure => CommandEncoding::Query(0x9B),
            Self::QueryContentDtr1 => CommandEncoding::Query(0x9C),
            Self::QueryContentDtr2 => CommandEncoding::Query(0x9D),
            Self::QueryOperatingMode => CommandEncoding::Query(0x9E),
            Self::QueryLightSourceType => CommandEncoding::Query(0x9F),
            Self::QueryActualLevel => CommandEncoding::Query(0xA0),
            Self::QueryMaxLevel => CommandEncoding::Query(0xA1),
            Self::QueryMinLevel => CommandEncoding::Query(0xA2),
            Self::QueryPowerOnLevel => CommandEncoding::Query(0xA3),
            Self::QuerySystemFailureLevel => CommandEncoding::Query(0xA4),
            Self::QueryFadeTimeFadeRate => CommandEncoding::Query(0xA5),
            Self::QueryManufacturerSpecificMode => CommandEncoding::Query(0xA6),
            Self::QueryControlGearFailure => CommandEncoding::Query(0xAA),
            Self::QueryExtendedFadeTime => CommandEncoding::Query(0xA8),
            Self::QueryNextDeviceType => CommandEncoding::Query(0xA7),
            Self::QuerySceneLevel { scene } => CommandEncoding::QueryWithParam(0xB0, *scene),
            Self::QueryGroups0To7 => CommandEncoding::Query(0xC0),
            Self::QueryGroups8To15 => CommandEncoding::Query(0xC1),
            Self::QueryRandomAddressH => CommandEncoding::Query(0xC2),
            Self::QueryRandomAddressM => CommandEncoding::Query(0xC3),
            Self::QueryRandomAddressL => CommandEncoding::Query(0xC4),
            _ => return None,
        })
    }

    pub fn is_query(&self) -> bool {
        matches!(
            self.encoding(),
            CommandEncoding::Query(_) | CommandEncoding::QueryWithParam(_, _)
        )
    }

    pub fn to_forward_frame(&self, address: &DaliAddress) -> ForwardFrame {
        let addr = address.encode_address_byte();
        match self.encoding() {
            CommandEncoding::DirectArc(level) => ForwardFrame::new(addr, level),
            CommandEncoding::Simple(opcode) => ForwardFrame::new(addr | 0x01, opcode),
            CommandEncoding::Scene(base, scene) => {
                ForwardFrame::new(addr | 0x01, base | (scene & 0x0F))
            }
            CommandEncoding::Query(opcode) => ForwardFrame::new(addr | 0x01, opcode),
            CommandEncoding::QueryWithParam(base, param) => {
                ForwardFrame::new(addr | 0x01, base | (param & 0x0F))
            }
            CommandEncoding::Config(opcode) => ForwardFrame::new(addr | 0x01, opcode),
            CommandEncoding::ConfigWithParam(base, param) => {
                ForwardFrame::new(addr | 0x01, base | (param & 0x0F))
            }
        }
    }

    pub fn requires_repeat(&self) -> bool {
        matches!(
            self.encoding(),
            CommandEncoding::Config(_) | CommandEncoding::ConfigWithParam(_, _)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEncoding {
    DirectArc(u8),
    Simple(u8),
    Scene(u8, u8),
    Query(u8),
    QueryWithParam(u8, u8),
    Config(u8),
    ConfigWithParam(u8, u8),
}
