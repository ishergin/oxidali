use dali2rust_contracts::msg::{ClusterRecordKind, ClusterRecordRef};

use super::limits::{GROUP_COUNT, MAX_ADAPTERS, SCENE_COUNT};

pub const CLUSTER_PART_LOCALS: u8 = GROUP_COUNT;

const _: () = assert!(GROUP_COUNT == SCENE_COUNT, "a part key is one group or one scene of an adapter");

const ADAPTER_SLOTS: usize = MAX_ADAPTERS as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterPartView {
    pub kind: ClusterRecordKind,
    pub adapter: u8,
    pub local: u8,
    pub record: ClusterRecordRef,
    pub send: bool,
    pub accept: bool,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterPartKeys {
    pub linked: [u16; ADAPTER_SLOTS],
}

impl ClusterPartKeys {
    pub fn insert(&mut self, adapter: u8, local: u8) {
        if let Some(mask) = self.linked.get_mut(usize::from(adapter)) {
            *mask |= 1_u16.checked_shl(u32::from(local)).unwrap_or(0);
        }
    }

    pub fn iter(self) -> impl Iterator<Item = (u8, u8)> {
        (0..MAX_ADAPTERS).flat_map(move |adapter| {
            let mask = self.linked.get(usize::from(adapter)).copied().unwrap_or(0);
            (0..CLUSTER_PART_LOCALS).filter(move |local| mask & (1 << local) != 0).map(move |local| (adapter, local))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_out_by_adapter_then_by_group_or_scene() {
        let mut keys = ClusterPartKeys::default();
        for (adapter, local) in [(7, 0), (0, 15), (0, 3), (2, 9)] {
            keys.insert(adapter, local);
        }
        assert_eq!(keys.iter().collect::<Vec<_>>(), [(0, 3), (0, 15), (2, 9), (7, 0)]);
    }

    #[test]
    fn a_key_past_the_last_adapter_or_group_is_not_kept() {
        let mut keys = ClusterPartKeys::default();
        keys.insert(MAX_ADAPTERS, 0);
        keys.insert(0, CLUSTER_PART_LOCALS);
        assert_eq!(keys, ClusterPartKeys::default());
    }
}
