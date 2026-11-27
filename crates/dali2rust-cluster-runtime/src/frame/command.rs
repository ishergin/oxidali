use serde::{Deserialize, Serialize};

use super::limits::{is_adapter, is_lamp, is_node, is_record_number};

pub use dali2rust_contracts::msg::ClusterAction as Action;
pub use dali2rust_contracts::msg::ClusterBirthMark as BirthMark;
pub use dali2rust_contracts::msg::ClusterLampAddress as LampAddress;
pub use dali2rust_contracts::msg::ClusterLightVerb as LightVerb;
pub use dali2rust_contracts::msg::ClusterRecallScope as RecallScope;
pub use dali2rust_contracts::msg::ClusterRecordRef as RecordRef;
pub use dali2rust_contracts::msg::ClusterSourceClass as SourceClass;
pub use dali2rust_contracts::msg::ClusterTarget as Target;

pub(crate) const fn is_record_ref(record: &RecordRef) -> bool {
    is_record_number(record.number)
}

const fn target_in_range(target: &Target) -> bool {
    match target {
        Target::Channel(channel) => is_record_ref(channel),
        Target::All => true,
        Target::Lamp(lamp) => is_node(lamp.node) && is_adapter(lamp.adapter) && is_lamp(lamp.lamp),
    }
}

const fn scope_in_range(scope: &RecallScope) -> bool {
    match scope {
        RecallScope::Node => true,
        RecallScope::Channel(channel) => is_record_ref(channel),
    }
}

pub const fn action_in_range(action: &Action) -> bool {
    match action {
        Action::Light { target, .. } => target_in_range(target),
        Action::RecallScene { scene, scope } => is_record_ref(scene) && scope_in_range(scope),
    }
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
        action_in_range(&self.action) && self.executed_on_wire.is_none_or(is_node)
    }
}
