use crate::frame::NodeClaim;

use super::roster::Roster;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OwnMode {
    #[default]
    Off,
    NoAddress,
    BindFailed,
    Standby,
    Listening,
    Waiting,
    Running,
}

impl OwnMode {
    pub const fn transmits(self) -> bool {
        matches!(self, Self::Listening | Self::Waiting | Self::Running)
    }

    pub const fn bound(self) -> bool {
        matches!(self, Self::Standby | Self::Listening | Self::Waiting | Self::Running)
    }

    pub const fn claim(self) -> NodeClaim {
        match self {
            Self::Running => NodeClaim::Holding,
            _ => NodeClaim::Waiting,
        }
    }
}

pub fn claim_mode(roster: &Roster<'_>, current: OwnMode, listened_ms: u32) -> OwnMode {
    if listened_ms < roster.timing.listen_ms() {
        return OwnMode::Listening;
    }
    match roster.rival() {
        None => OwnMode::Running,
        Some(rival) if current == OwnMode::Running && rival.mac > roster.own.mac => OwnMode::Running,
        Some(_) => OwnMode::Waiting,
    }
}
