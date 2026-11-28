use dali2rust_contracts::msg::{ClusterRecordKind, ClusterRecordRef};
use dali2rust_domain::registry::{cluster_node_of, ClusterRecordReadPort};

use crate::info::ClusterInfoReadPort;

use super::shared::{BlockState, ClusterPresenceReadPort};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordStanding {
    Live,
    Orphan,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct HomeReads<'a> {
    pub presence: &'a dyn ClusterPresenceReadPort,
    pub info: &'a dyn ClusterInfoReadPort,
}

impl HomeReads<'_> {
    #[must_use]
    pub fn judged(&self, node: u8) -> bool {
        let (Some(view), Some(held)) = (self.presence.heard_node(node), self.info.info_held(node)) else {
            return false;
        };
        let heard_since_settled = view.settled_for_ms.is_some_and(|settled_ms| held.age_ms < settled_ms);
        view.live && !view.split && view.mac == held.mac && heard_since_settled
    }

    #[must_use]
    pub fn standing(&self, kind: ClusterRecordKind, record: ClusterRecordRef) -> RecordStanding {
        let home = cluster_node_of(record.number);
        let shown = self.info.info_record(home, kind, record.number);
        if shown.as_ref().is_some_and(|view| view.mark == record.mark) {
            return RecordStanding::Live;
        }
        if !self.judged(home) {
            return RecordStanding::Unknown;
        }
        verdict(shown.map(|view| view.mark.generation), record)
    }
}

#[must_use]
pub fn own_standing(
    records: &dyn ClusterRecordReadPort,
    block: BlockState,
    kind: ClusterRecordKind,
    record: ClusterRecordRef,
) -> RecordStanding {
    let held = records.cluster_record(kind, record.number);
    if held.as_ref().is_some_and(|view| view.mark == record.mark) {
        return RecordStanding::Live;
    }
    if block != BlockState::Settled {
        return RecordStanding::Unknown;
    }
    verdict(held.map(|view| view.mark.generation), record)
}

const fn verdict(held_generation: Option<u32>, record: ClusterRecordRef) -> RecordStanding {
    match held_generation {
        None => RecordStanding::Orphan,
        Some(generation) if generation > record.mark.generation => RecordStanding::Orphan,
        Some(_) => RecordStanding::Unknown,
    }
}
