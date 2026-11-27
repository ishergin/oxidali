use dali2rust_contracts::msg::{LevelTransition, LightSetpoint};
use serde::{Deserialize, Serialize};

use super::limits::{is_adapter, is_lamp, is_node, is_record_number};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BirthMark {
    pub generation: u32,
    pub nonce: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRef {
    pub number: u8,
    pub mark: BirthMark,
}

impl RecordRef {
    pub(crate) const fn in_range(&self) -> bool {
        is_record_number(self.number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LampAddress {
    pub node: u8,
    pub adapter: u8,
    pub lamp: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Channel(RecordRef),
    All,
    Lamp(LampAddress),
}

impl Target {
    const fn in_range(&self) -> bool {
        match self {
            Self::Channel(channel) => channel.in_range(),
            Self::All => true,
            Self::Lamp(lamp) => is_node(lamp.node) && is_adapter(lamp.adapter) && is_lamp(lamp.lamp),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecallScope {
    Node,
    Channel(RecordRef),
}

impl RecallScope {
    const fn in_range(&self) -> bool {
        match self {
            Self::Node => true,
            Self::Channel(channel) => channel.in_range(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightVerb {
    Setpoint(LightSetpoint),
    Transition(LevelTransition),
    StopFade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Light { target: Target, verb: LightVerb },
    RecallScene { scene: RecordRef, scope: RecallScope },
}

impl Action {
    const fn in_range(&self) -> bool {
        match self {
            Self::Light { target, .. } => target.in_range(),
            Self::RecallScene { scene, scope } => scene.in_range() && scope.in_range(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceClass {
    Operator,
    HomeAssistant,
    Hcl,
    Rules,
    ForeignMaster,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub action: Action,
    pub source: SourceClass,
    pub hold_hcl: bool,
    pub executed_on_wire: Option<u8>,
}

impl Command {
    pub(crate) fn in_range(&self) -> bool {
        self.action.in_range() && self.executed_on_wire.is_none_or(is_node)
    }
}
