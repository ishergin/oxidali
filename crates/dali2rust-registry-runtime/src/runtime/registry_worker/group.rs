use std::sync::atomic::AtomicU32;

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    ConfigWriteResource, GroupMatrixDesiredRow, GroupMetadataUpdateCommand,
};

use crate::runtime::registry::config_write_stage::ConfigWriteRejection;
use crate::runtime::registry::publish::{publish_group_changed, publish_group_matrix_changed};
use crate::runtime::registry::RegistryStore;

use super::matrix::{
    commit_matrix, MatrixResource, MetadataCommand, GROUP_COUNT, VIRTUAL_LAMP_COUNT,
};
use super::RegistryCommandCounters;

impl MetadataCommand for GroupMetadataUpdateCommand {
    const ID_OUT_OF_RANGE: &'static str = "group_id_out_of_range";
    const ID_COUNT: u8 = GROUP_COUNT;

    fn adapter_id(&self) -> u8 {
        self.adapter_id
    }

    fn id(&self) -> u8 {
        self.group_id
    }

    fn apply_and_publish(&self, publisher: &BusPublisher, corr: u64, store: &RegistryStore) {
        store.apply_group_metadata_patch(
            self.adapter_id,
            self.group_id,
            self.patch_mask,
            Some(self.name.as_str()),
            self.ha_entity_enabled,
        );
        publish_group_changed(publisher, corr, self.adapter_id, self.group_id);
    }

    fn applied_counter(counters: &RegistryCommandCounters) -> &AtomicU32 {
        &counters.group_metadata_patches_applied
    }
}

pub(super) struct GroupMatrix;

impl GroupMatrix {
    pub(super) const NO_SCENE: u8 = 0;
}

impl MatrixResource for GroupMatrix {
    type Row = GroupMatrixDesiredRow;

    const ROW_REJECT: &'static str = "group_matrix_row_out_of_range";

    fn check_scope(_scene_id: u8) -> Result<(), &'static str> {
        Ok(())
    }

    fn row_is_valid(row: &Self::Row) -> bool {
        row.virtual_lamp_id < VIRTUAL_LAMP_COUNT
    }

    fn stage(
        store: &RegistryStore,
        corr: u64,
        adapter_id: u8,
        _scene_id: u8,
        rows: &[Self::Row],
    ) -> Result<(), ConfigWriteRejection> {
        store.stage_group_matrix_chunk(corr, adapter_id, rows)
    }
}

pub(super) fn commit_group_matrix(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    adapter_id: u8,
    chunks: u8,
) -> Result<(), &'static str> {
    commit_matrix(
        publisher,
        corr,
        store,
        counters,
        ConfigWriteResource::GroupMatrix,
        adapter_id,
        GroupMatrix::NO_SCENE,
        chunks,
        &counters.group_matrix_patches_applied,
        || publish_group_matrix_changed(publisher, corr, adapter_id),
    )
}
