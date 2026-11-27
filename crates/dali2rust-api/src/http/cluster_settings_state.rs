use std::sync::Arc;

use dali2rust_domain::registry::{ClusterSettingsApplyWatchPort, ClusterSettingsReadPort, ClusterSettingsView};
use serde::Serialize;

use crate::http::role::ControllerRolePort;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClusterBlockDto {
    pub first: u8,
    pub last: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterSettingsDto {
    pub enabled: bool,
    pub cluster_name: String,
    pub node_number: u8,
    pub port: u16,
    pub wire_same_as: Option<u8>,
    pub forward_operator: bool,
    pub forward_home_assistant: bool,
    pub forward_hcl: bool,
    pub forward_foreign_master: bool,
    pub accept_all_lamps: bool,
    pub send_all_lamps: bool,
    pub accept_single_lamps: bool,
    pub wire_protection: bool,
    pub read_only: bool,
    pub block: ClusterBlockDto,
}

pub trait ClusterSettingsHttpState: Send + Sync {
    fn cluster_settings_dto(&self) -> ClusterSettingsDto;
}

pub trait ClusterSettingsApplyWatch: Send + Sync {
    fn cluster_settings_applied_load(&self) -> u32;
}

pub struct ClusterSettingsHttpStateBridge {
    settings: Arc<dyn ClusterSettingsReadPort>,
    role: Arc<dyn ControllerRolePort>,
}

impl ClusterSettingsHttpStateBridge {
    pub fn new(settings: Arc<dyn ClusterSettingsReadPort>, role: Arc<dyn ControllerRolePort>) -> Self {
        Self { settings, role }
    }
}

impl ClusterSettingsHttpState for ClusterSettingsHttpStateBridge {
    fn cluster_settings_dto(&self) -> ClusterSettingsDto {
        cluster_settings_view_to_dto(self.settings.cluster_settings_view(), !self.role.is_active())
    }
}

pub struct ClusterSettingsApplyWatchBridge {
    port: Arc<dyn ClusterSettingsApplyWatchPort>,
}

impl ClusterSettingsApplyWatchBridge {
    pub fn new(port: Arc<dyn ClusterSettingsApplyWatchPort>) -> Self {
        Self { port }
    }
}

impl ClusterSettingsApplyWatch for ClusterSettingsApplyWatchBridge {
    fn cluster_settings_applied_load(&self) -> u32 {
        self.port.cluster_settings_applied_load()
    }
}

pub(crate) fn cluster_settings_view_to_dto(view: ClusterSettingsView, read_only: bool) -> ClusterSettingsDto {
    let block = view.block();
    ClusterSettingsDto {
        block: ClusterBlockDto { first: *block.start(), last: *block.end() },
        wire_same_as: view.wire.same_as(),
        forward_operator: view.forwarding.operator,
        forward_home_assistant: view.forwarding.home_assistant,
        forward_hcl: view.forwarding.hcl,
        forward_foreign_master: view.forwarding.foreign_master,
        enabled: view.enabled,
        cluster_name: view.cluster_name,
        node_number: view.node_number,
        port: view.port,
        accept_all_lamps: view.accept_all_lamps,
        send_all_lamps: view.send_all_lamps,
        accept_single_lamps: view.accept_single_lamps,
        wire_protection: view.wire_protection,
        read_only,
    }
}

#[cfg(test)]
mod tests {
    use dali2rust_domain::registry::{ClusterWire, CLUSTER_NODE_COUNT, CLUSTER_RECORD_NUMBER_COUNT};

    use super::*;

    const DTO_CEILING_BYTES: usize = 64;

    #[test]
    fn the_dto_stays_within_its_ceiling() {
        let bytes = core::mem::size_of::<ClusterSettingsDto>();
        assert!(bytes <= DTO_CEILING_BYTES, "ClusterSettingsDto is {bytes} B of {DTO_CEILING_BYTES} B");
    }

    #[test]
    fn the_block_is_the_node_s_sixteen_numbers() {
        let view = ClusterSettingsView { node_number: CLUSTER_NODE_COUNT - 1, ..ClusterSettingsView::default() };
        let dto = cluster_settings_view_to_dto(view, false);
        assert_eq!((dto.block.first, dto.block.last), (112, CLUSTER_RECORD_NUMBER_COUNT - 1));
    }

    #[test]
    fn an_own_wire_reads_as_null_and_a_shared_one_as_its_node() {
        let shared = ClusterSettingsView { wire: ClusterWire::SameAs(5), ..ClusterSettingsView::default() };
        assert_eq!(cluster_settings_view_to_dto(shared, true).wire_same_as, Some(5));
        assert_eq!(cluster_settings_view_to_dto(ClusterSettingsView::default(), true).wire_same_as, None);
    }
}
