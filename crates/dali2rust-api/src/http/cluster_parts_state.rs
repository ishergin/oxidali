use std::sync::Arc;

use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{ClusterPartKeys, ClusterPartReadPort, ClusterPartView};
use serde::Serialize;

use crate::http::cluster_records_state::ClusterBirthMarkDto;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClusterPartRecordDto {
    pub number: u8,
    pub birth_mark: ClusterBirthMarkDto,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClusterPartDto {
    pub adapter: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<u8>,
    pub record: ClusterPartRecordDto,
    pub send: bool,
    pub accept: bool,
    pub revision: u32,
}

pub trait ClusterPartsHttpState: Send + Sync {
    fn cluster_part_dto(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto>;

    fn cluster_part_keys(&self, kind: ClusterRecordKind) -> ClusterPartKeys;
}

pub struct ClusterPartsHttpStateBridge {
    parts: Arc<dyn ClusterPartReadPort>,
}

impl ClusterPartsHttpStateBridge {
    pub fn new(parts: Arc<dyn ClusterPartReadPort>) -> Self {
        Self { parts }
    }
}

impl ClusterPartsHttpState for ClusterPartsHttpStateBridge {
    fn cluster_part_dto(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto> {
        self.parts.cluster_part(kind, adapter, local).map(cluster_part_view_to_dto)
    }

    fn cluster_part_keys(&self, kind: ClusterRecordKind) -> ClusterPartKeys {
        self.parts.cluster_part_keys(kind)
    }
}

pub(crate) fn cluster_part_view_to_dto(view: ClusterPartView) -> ClusterPartDto {
    let (group, scene) = match view.kind {
        ClusterRecordKind::Channel => (Some(view.local), None),
        ClusterRecordKind::Scene => (None, Some(view.local)),
    };
    let mark = view.record.mark;
    ClusterPartDto {
        adapter: view.adapter,
        group,
        scene,
        record: ClusterPartRecordDto {
            number: view.record.number,
            birth_mark: ClusterBirthMarkDto { generation: mark.generation, nonce: mark.nonce },
        },
        send: view.send,
        accept: view.accept,
        revision: view.revision,
    }
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{ClusterBirthMark, ClusterRecordRef};

    use super::*;

    const DTO_CEILING_BYTES: usize = 32;

    fn view(kind: ClusterRecordKind) -> ClusterPartView {
        ClusterPartView {
            kind,
            adapter: 2,
            local: 9,
            record: ClusterRecordRef { number: 40, mark: ClusterBirthMark { generation: 3, nonce: 0xdead_beef } },
            send: true,
            accept: false,
            revision: 6,
        }
    }

    #[test]
    fn the_dto_stays_within_its_ceiling() {
        let bytes = core::mem::size_of::<ClusterPartDto>();
        assert!(bytes <= DTO_CEILING_BYTES, "ClusterPartDto is {bytes} B of {DTO_CEILING_BYTES} B");
    }

    #[test]
    fn every_field_comes_from_its_own_view_field() {
        let dto = cluster_part_view_to_dto(view(ClusterRecordKind::Channel));
        let record = (dto.record.number, dto.record.birth_mark.generation, dto.record.birth_mark.nonce);
        assert_eq!((dto.adapter, dto.group, dto.scene), (2, Some(9), None));
        assert_eq!(record, (40, 3, 0xdead_beef));
        assert_eq!((dto.send, dto.accept, dto.revision), (true, false, 6));
    }

    #[test]
    fn a_scene_part_names_its_scene_and_no_group() {
        let json = serde_json::to_value(cluster_part_view_to_dto(view(ClusterRecordKind::Scene))).expect("json");
        assert_eq!((json.get("group"), json["scene"].as_u64()), (None, Some(9)));
    }
}
