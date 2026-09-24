use std::sync::Arc;

use dali2rust_domain::registry::{
    GroupApplySnapshot, GroupMetadataWatchPort, GroupReadPort, GroupView,
};
use serde::Serialize;

use super::adapter_state::AdapterHttpState;
use super::physical_device_state::{capabilities_view_to_dto, CapabilityFlagsDto};

#[derive(Clone, Debug, Serialize)]
pub struct GroupDto {
    pub adapter_id: u8,
    pub group_id: u8,
    pub name: String,
    pub ha_entity_enabled: bool,
    pub capabilities_summary: CapabilityFlagsDto,
    pub dirty: bool,
    pub member_count_desired: u8,
    pub member_count_applied: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupMatrixGroupDto {
    pub group_id: u8,
    pub name: String,
    pub dirty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupMembershipMatrixRowDto {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub desired: [bool; 16],
    pub applied: [bool; 16],
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupMembershipMatrixDto {
    pub adapter_id: u8,
    pub groups: Vec<GroupMatrixGroupDto>,
    pub rows: Vec<GroupMembershipMatrixRowDto>,
    pub dirty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupsListBody {
    pub adapter_id: u8,
    pub groups: Vec<GroupDto>,
}

pub fn group_view_to_dto(view: GroupView) -> GroupDto {
    GroupDto {
        adapter_id: view.adapter_id,
        group_id: view.group_id,
        name: view.name,
        ha_entity_enabled: view.ha_entity_enabled,
        capabilities_summary: capabilities_view_to_dto(&view.capabilities_summary),
        dirty: view.dirty,
        member_count_desired: view.member_count_desired,
        member_count_applied: view.member_count_applied,
    }
}

pub trait GroupMetadataApplyWatch: Send + Sync {
    fn group_metadata_applied_load(&self) -> u32;
}

pub struct GroupMetadataApplyWatchBridge {
    port: Arc<dyn GroupMetadataWatchPort>,
}

impl GroupMetadataApplyWatchBridge {
    pub fn new(port: Arc<dyn GroupMetadataWatchPort>) -> Self {
        Self { port }
    }
}

impl GroupMetadataApplyWatch for GroupMetadataApplyWatchBridge {
    fn group_metadata_applied_load(&self) -> u32 {
        self.port.group_metadata_applied_load()
    }
}

pub trait GroupHttpState: AdapterHttpState {
    fn group_dto(&self, adapter_id: u8, group_id: u8) -> Option<GroupDto>;
    fn list_group_dtos(&self, adapter_id: u8) -> Vec<GroupDto>;
    fn group_membership_matrix_dto(&self, adapter_id: u8) -> Option<GroupMembershipMatrixDto>;
    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot>;
}

pub struct GroupHttpStateBridge {
    port: Arc<dyn GroupReadPort>,
}

impl GroupHttpStateBridge {
    pub fn new(port: Arc<dyn GroupReadPort>) -> Self {
        Self { port }
    }
}

crate::http::adapter_state::impl_adapter_http_state_from_port!(GroupHttpStateBridge);

impl GroupHttpState for GroupHttpStateBridge {
    fn group_dto(&self, adapter_id: u8, group_id: u8) -> Option<GroupDto> {
        self.port.group_view(adapter_id, group_id).map(group_view_to_dto)
    }

    fn list_group_dtos(&self, adapter_id: u8) -> Vec<GroupDto> {
        self.port
            .list_group_views(adapter_id)
            .into_iter()
            .map(group_view_to_dto)
            .collect()
    }

    fn group_membership_matrix_dto(&self, adapter_id: u8) -> Option<GroupMembershipMatrixDto> {
        self.port.group_membership_matrix_view(adapter_id).map(|view| {
            GroupMembershipMatrixDto {
                adapter_id: view.adapter_id,
                groups: view
                    .groups
                    .into_iter()
                    .map(|group| GroupMatrixGroupDto {
                        group_id: group.group_id,
                        name: group.name,
                        dirty: group.dirty,
                    })
                    .collect(),
                rows: view
                    .rows
                    .into_iter()
                    .map(|row| GroupMembershipMatrixRowDto {
                        virtual_lamp_id: row.virtual_lamp_id,
                        name: row.name,
                        desired: row.desired,
                        applied: row.applied,
                    })
                    .collect(),
                dirty: view.dirty,
            }
        })
    }

    fn group_apply_snapshot(&self, adapter_id: u8) -> Option<GroupApplySnapshot> {
        self.port.group_apply_snapshot(adapter_id)
    }
}
