use std::sync::atomic::AtomicU32;

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    ConfigWriteResource, SceneMatrixDesiredRow, SceneMetadataUpdateCommand,
};

use crate::runtime::registry::config_write_stage::ConfigWriteRejection;
use crate::runtime::registry::publish::{publish_scene_changed, publish_scene_matrix_changed};
use crate::runtime::registry::RegistryStore;

use super::matrix::{
    commit_matrix, MatrixResource, MetadataCommand, SCENE_COUNT, VIRTUAL_LAMP_COUNT,
};
use super::RegistryCommandCounters;

impl MetadataCommand for SceneMetadataUpdateCommand {
    const ID_OUT_OF_RANGE: &'static str = "scene_id_out_of_range";
    const ID_COUNT: u8 = SCENE_COUNT;

    fn adapter_id(&self) -> u8 {
        self.adapter_id
    }

    fn id(&self) -> u8 {
        self.scene_id
    }

    fn apply_and_publish(&self, publisher: &BusPublisher, corr: u64, store: &RegistryStore) {
        store.apply_scene_metadata_patch(
            self.adapter_id,
            self.scene_id,
            self.patch_mask,
            Some(self.name.as_str()),
            self.ha_select_enabled,
        );
        publish_scene_changed(publisher, corr, self.adapter_id, self.scene_id);
    }

    fn applied_counter(counters: &RegistryCommandCounters) -> &AtomicU32 {
        &counters.scene_metadata_patches_applied
    }
}

pub(super) struct SceneMatrix;

impl MatrixResource for SceneMatrix {
    type Row = SceneMatrixDesiredRow;

    const ROW_REJECT: &'static str = "scene_matrix_row_invalid";

    fn check_scope(scene_id: u8) -> Result<(), &'static str> {
        if scene_id < SCENE_COUNT {
            Ok(())
        } else {
            Err("scene_id_out_of_range")
        }
    }

    fn row_is_valid(row: &Self::Row) -> bool {
        row.virtual_lamp_id < VIRTUAL_LAMP_COUNT && row.included == row.target.is_some()
    }

    fn stage(
        store: &RegistryStore,
        corr: u64,
        adapter_id: u8,
        scene_id: u8,
        rows: &[Self::Row],
    ) -> Result<(), ConfigWriteRejection> {
        store.stage_scene_matrix_chunk(corr, adapter_id, scene_id, rows)
    }
}

pub(super) fn commit_scene_matrix(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    adapter_id: u8,
    scene_id: u8,
    chunks: u8,
) -> Result<(), &'static str> {
    commit_matrix(
        publisher,
        corr,
        store,
        counters,
        ConfigWriteResource::SceneMatrix,
        adapter_id,
        scene_id,
        chunks,
        &counters.scene_matrix_patches_applied,
        || publish_scene_matrix_changed(publisher, corr, adapter_id, scene_id),
    )
}
