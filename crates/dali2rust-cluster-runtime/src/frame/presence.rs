use serde::{Deserialize, Serialize};

use super::limits::{is_node, MAX_CLUSTER_NAME_BYTES, MAX_INSTALLATION_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeClaim {
    Holding,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pulse<'a> {
    pub cluster_name: &'a str,
    pub installation: &'a str,
    pub claim: NodeClaim,
    pub wire: u8,
    pub wire_protection: bool,
}

impl Pulse<'_> {
    pub(crate) const fn in_range(&self) -> bool {
        self.cluster_name.len() <= MAX_CLUSTER_NAME_BYTES
            && self.installation.len() <= MAX_INSTALLATION_BYTES
            && is_node(self.wire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub ha_face: bool,
}
