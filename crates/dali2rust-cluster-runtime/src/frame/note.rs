use serde::{Deserialize, Serialize};

use super::command::LightVerb;
use super::limits::{is_adapter, is_group, is_node, is_node_scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteReach {
    Group(u8),
    Broadcast,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteDeed {
    Light(LightVerb),
    Scene(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNote {
    pub wire: u8,
    pub adapter: u8,
    pub reach: NoteReach,
    pub deed: NoteDeed,
}

impl WireNote {
    pub(crate) const fn in_range(&self) -> bool {
        let reach = match self.reach {
            NoteReach::Group(group) => is_group(group),
            NoteReach::Broadcast => true,
        };
        let deed = match &self.deed {
            NoteDeed::Light(_) => true,
            NoteDeed::Scene(scene) => is_node_scene(*scene),
        };
        is_node(self.wire) && is_adapter(self.adapter) && reach && deed
    }
}
