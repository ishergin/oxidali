use std::hash::{BuildHasher, RandomState};
use std::sync::Arc;

use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{ClusterRecordNumbers, ClusterRecordReadPort, ClusterRecordView};
use serde::Serialize;

use crate::http::cluster_info_state::ClusterInfoSource;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ClusterBirthMarkDto {
    pub generation: u32,
    pub nonce: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterRecordDto {
    pub number: u8,
    pub home_node: u8,
    pub birth_mark: ClusterBirthMarkDto,
    pub name: String,
    pub revision: u32,
    pub source: ClusterInfoSource,
}

pub trait ClusterRecordsHttpState: Send + Sync {
    fn cluster_record_dto(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto>;

    fn cluster_record_numbers(&self, kind: ClusterRecordKind) -> ClusterRecordNumbers;
}

pub struct ClusterRecordsHttpStateBridge {
    records: Arc<dyn ClusterRecordReadPort>,
}

impl ClusterRecordsHttpStateBridge {
    pub fn new(records: Arc<dyn ClusterRecordReadPort>) -> Self {
        Self { records }
    }
}

impl ClusterRecordsHttpState for ClusterRecordsHttpStateBridge {
    fn cluster_record_dto(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        self.records.cluster_record(kind, number).map(cluster_record_view_to_dto)
    }

    fn cluster_record_numbers(&self, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        self.records.cluster_record_numbers(kind)
    }
}

pub fn cluster_record_view_to_dto(view: ClusterRecordView) -> ClusterRecordDto {
    ClusterRecordDto {
        number: view.number,
        home_node: view.home_node,
        birth_mark: ClusterBirthMarkDto { generation: view.mark.generation, nonce: view.mark.nonce },
        name: view.name,
        revision: view.revision,
        source: ClusterInfoSource::Own,
    }
}

pub(crate) fn record_nonce() -> u32 {
    let bytes = RandomState::new().hash_one(()).to_le_bytes();
    let [a, b, c, d, e, f, g, h] = bytes;
    u32::from_le_bytes([a, b, c, d]) ^ u32::from_le_bytes([e, f, g, h])
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::ClusterBirthMark;

    use super::*;

    const DTO_CEILING_BYTES: usize = 64;

    #[test]
    fn the_dto_stays_within_its_ceiling() {
        let bytes = core::mem::size_of::<ClusterRecordDto>();
        assert!(bytes <= DTO_CEILING_BYTES, "ClusterRecordDto is {bytes} B of {DTO_CEILING_BYTES} B");
    }

    #[test]
    fn every_field_comes_from_its_own_view_field() {
        let view = ClusterRecordView {
            kind: ClusterRecordKind::Scene,
            number: 34,
            home_node: 2,
            mark: ClusterBirthMark { generation: 5, nonce: 0xdead_beef },
            name: "Hall".to_owned(),
            revision: 7,
        };
        let dto = cluster_record_view_to_dto(view);
        let read = (dto.number, dto.home_node, dto.birth_mark.generation, dto.birth_mark.nonce);
        assert_eq!(read, (34, 2, 5, 0xdead_beef));
        assert_eq!((dto.name.as_str(), dto.revision), ("Hall", 7));
    }

    #[test]
    fn two_nonces_drawn_in_a_row_differ() {
        let drawn: std::collections::HashSet<u32> = (0..8).map(|_| record_nonce()).collect();
        assert!(drawn.len() > 1, "eight nonces were all {drawn:?}");
    }
}
