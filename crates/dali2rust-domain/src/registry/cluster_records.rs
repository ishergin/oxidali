use dali2rust_contracts::msg::{ClusterBirthMark, ClusterRecordKind};

use super::cluster::{cluster_block, CLUSTER_NUMBERS_PER_NODE, CLUSTER_RECORD_NUMBER_COUNT};

pub const MAX_CLUSTER_RECORD_NAME_BYTES: usize = 64;

pub const CLUSTER_RECORD_KINDS: [ClusterRecordKind; 2] = [ClusterRecordKind::Channel, ClusterRecordKind::Scene];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterRecordView {
    pub kind: ClusterRecordKind,
    pub number: u8,
    pub home_node: u8,
    pub mark: ClusterBirthMark,
    pub name: String,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterRecordNumbers {
    pub first: u8,
    pub live: u16,
}

impl ClusterRecordNumbers {
    pub fn iter(self) -> impl Iterator<Item = u8> {
        (0..CLUSTER_NUMBERS_PER_NODE)
            .filter(move |index| self.live & (1 << index) != 0)
            .map(move |index| self.first.saturating_add(index))
    }

    #[must_use]
    pub const fn any(self) -> bool {
        self.live != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterRecordConflict {
    NumberOfAnotherNode,
    NumberTaken,
    BlockFull,
    StaleRevision,
    NodeNumberPinned,
    NonceTaken,
    RecordNotLive,
    RecordLinked,
    BlockUnsettled,
}

impl ClusterRecordConflict {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NumberOfAnotherNode => "cluster_number_of_another_node",
            Self::NumberTaken => "cluster_number_taken",
            Self::BlockFull => "cluster_block_full",
            Self::StaleRevision => "cluster_stale_revision",
            Self::NodeNumberPinned => "cluster_node_number_pinned",
            Self::NonceTaken => "cluster_nonce_taken",
            Self::RecordNotLive => "cluster_record_not_live",
            Self::RecordLinked => "cluster_record_linked",
            Self::BlockUnsettled => "cluster_block_unsettled",
        }
    }
}

#[must_use]
pub const fn is_cluster_record_number(number: u8) -> bool {
    number < CLUSTER_RECORD_NUMBER_COUNT
}

#[must_use]
pub fn block_index(node: u8, number: u8) -> Option<u8> {
    let block = cluster_block(node);
    block.contains(&number).then(|| number - block.start())
}

#[must_use]
pub fn next_record_index(issued: impl Fn(u8) -> bool, live: impl Fn(u8) -> bool) -> Option<u8> {
    let mut indices = 0..CLUSTER_NUMBERS_PER_NODE;
    indices.clone().find(|index| !issued(*index)).or_else(|| indices.find(|index| !live(*index)))
}

#[must_use]
pub fn default_record_name(kind: ClusterRecordKind, number: u8) -> String {
    match kind {
        ClusterRecordKind::Channel => format!("Channel {number}"),
        ClusterRecordKind::Scene => format!("Scene {number}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_belongs_to_the_block_of_its_node_only() {
        assert_eq!(block_index(2, 32), Some(0));
        assert_eq!(block_index(2, 47), Some(15));
        assert_eq!(block_index(2, 31), None);
        assert_eq!(block_index(2, 48), None);
        assert_eq!(block_index(7, 127), Some(15));
    }

    #[test]
    fn a_never_issued_index_goes_before_a_freed_one() {
        let issued = |index: u8| index < 3;
        let live = |index: u8| index != 1 && index < 3;
        assert_eq!(next_record_index(issued, live), Some(3));
    }

    #[test]
    fn a_freed_index_goes_once_every_index_was_issued() {
        assert_eq!(next_record_index(|_| true, |index| index != 9 && index != 12), Some(9));
        assert_eq!(next_record_index(|_| true, |_| true), None);
    }

    #[test]
    fn the_live_numbers_are_read_from_the_mask_in_the_block() {
        let numbers = ClusterRecordNumbers { first: 32, live: 0b1000_0000_0000_0101 };
        assert_eq!(numbers.iter().collect::<Vec<_>>(), [32, 34, 47]);
        assert!(numbers.any());
        assert!(!ClusterRecordNumbers::default().any());
    }

    #[test]
    fn a_default_name_says_the_kind_and_the_number() {
        assert_eq!(default_record_name(ClusterRecordKind::Channel, 8), "Channel 8");
        assert_eq!(default_record_name(ClusterRecordKind::Scene, 127), "Scene 127");
        assert!(default_record_name(ClusterRecordKind::Channel, 127).len() <= MAX_CLUSTER_RECORD_NAME_BYTES);
    }

    #[test]
    fn the_record_numbers_end_at_the_last_block() {
        assert!(is_cluster_record_number(CLUSTER_RECORD_NUMBER_COUNT - 1));
        assert!(!is_cluster_record_number(CLUSTER_RECORD_NUMBER_COUNT));
    }
}
