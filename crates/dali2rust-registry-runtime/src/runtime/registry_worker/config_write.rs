use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{ConfigWriteCommitCommand, ConfigWriteResource, ErrorCode};

use crate::runtime::registry::publish::publish_config_write_signal;
use crate::runtime::registry::RegistryStore;

use super::confirm::{check_adapter_id, check_primary_adapter};
use super::matrix::{reject, SCENE_COUNT};
use super::RegistryCommandCounters;

#[allow(clippy::too_many_arguments, reason = "dispatch-table arity, shared by every handler here")]
pub(super) fn handle_config_write_commit(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &ConfigWriteCommitCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    let outcome = match body.resource {
        ConfigWriteResource::GroupMatrix => super::group::commit_group_matrix(
            publisher,
            corr,
            store,
            counters,
            body.adapter_id,
            body.chunks,
        ),
        ConfigWriteResource::SceneMatrix => commit_scene(publisher, corr, store, counters, body),
    };
    publish_config_write_signal(
        publisher,
        corr,
        primary_adapter_id,
        outcome.err().map(|message| (ErrorCode::InvalidValue, message)),
        counters,
    );
}

fn commit_scene(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &ConfigWriteCommitCommand,
) -> Result<(), &'static str> {
    let Some(scene_id) = body.scene_id else {
        reject(publisher, counters, corr, "scene_id_missing");
        return Err("scene_id_missing");
    };
    if scene_id >= SCENE_COUNT {
        reject(publisher, counters, corr, "scene_id_out_of_range");
        return Err("scene_id_out_of_range");
    }
    super::scene::commit_scene_matrix(
        publisher,
        corr,
        store,
        counters,
        body.adapter_id,
        scene_id,
        body.chunks,
    )
}
