use std::sync::atomic::Ordering;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};

use dali2rust_contracts::msg::BusCommandPayload;

use crate::runtime::registry::RegistryStore;

use super::adapter::handle_adapter_settings;
use super::config_write::handle_config_write_commit;
use super::group::GroupMatrix;
use super::hcl::{handle_hcl_schedule_delete, handle_hcl_schedule_upsert};
use super::matrix::{handle_metadata, stage_matrix_chunk};
use super::home_assistant_settings::{
    handle_controller_id_update, handle_credentials_update, handle_settings_update,
    handle_topics_update,
};
use super::physical_device::{handle_pd_forget, handle_pd_notes, handle_pd_override};
use super::dali_settings::handle_dali_settings_update;
use super::policies::handle_policies_update;
use super::transfer::handle_slice_reload;
use super::redundancy_settings::handle_redundancy_settings_update;
use super::poller_settings::handle_poller_settings_update;
use super::runtime_apply::{handle_level_transition, handle_runtime_update};
use super::scene::SceneMatrix;
use super::virtual_lamp::{
    handle_vl_bind, handle_vl_config, handle_vl_delete, handle_vl_rebind, handle_vl_unbind,
};
use super::RegistryCommandCounters;

pub(super) fn process_one(
    frame: BusFrame,
    publisher: &BusPublisher,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    persistence_slices: Option<&std::sync::Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
) {
    let BusFrame::Command(ce_arc) = frame else {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ce = ce_arc.as_ref();
    let tid = ce.meta.target_adapter_id;
    let corr = ce.meta.correlation_id;
    dispatch_registry_command(
        &ce.payload,
        publisher,
        tid,
        corr,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        persistence_slices,
    );
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const REGISTRY_WORKER_HANDLED_COMMANDS;
    fn dispatch_registry_command(
        payload: &BusCommandPayload,
        publisher: &BusPublisher,
        tid: u16,
        corr: u64,
        primary_adapter_id: BusId,
        adapter_count: u8,
        store: &RegistryStore,
        counters: &RegistryCommandCounters,
        persistence_slices: Option<&std::sync::Arc<dyn dali2rust_platform::slice_store::SliceStore>>,
    );
    payload = payload;
    ignored = { counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    AdapterSettingsUpdateCommand(body) =>
        handle_adapter_settings(publisher, tid, corr, adapter_count, store, counters, body),
    VirtualLampConfigUpdateCommand(body) =>
        handle_vl_config(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    GroupMetadataUpdateCommand(body) =>
        handle_metadata(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    GroupMatrixDesiredPatchCommand(body) =>
        stage_matrix_chunk::<GroupMatrix>(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body.adapter_id, GroupMatrix::NO_SCENE, body.rows.as_slice()),
    GroupMatrixDesiredReplaceCommand(body) =>
        stage_matrix_chunk::<GroupMatrix>(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body.adapter_id, GroupMatrix::NO_SCENE, body.rows.as_slice()),
    SceneMetadataUpdateCommand(body) =>
        handle_metadata(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    SceneMatrixDesiredPatchCommand(body) =>
        stage_matrix_chunk::<SceneMatrix>(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body.adapter_id, body.scene_id, body.rows.as_slice()),
    SceneMatrixDesiredReplaceCommand(body) =>
        stage_matrix_chunk::<SceneMatrix>(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body.adapter_id, body.scene_id, body.rows.as_slice()),
    ConfigWriteCommitCommand(body) =>
        handle_config_write_commit(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    VirtualLampBindCommand(body) =>
        handle_vl_bind(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    VirtualLampRebindCommand(body) =>
        handle_vl_rebind(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    VirtualLampUnbindCommand(body) =>
        handle_vl_unbind(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    VirtualLampDeleteCommand(body) =>
        handle_vl_delete(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    RegistryRuntimeUpdateCommand(reg) =>
        handle_runtime_update(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, reg),
    RegistryLevelTransitionCommand(cmd) =>
        handle_level_transition(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, cmd),
    PhysicalDeviceOverrideCommand(body) =>
        handle_pd_override(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    PhysicalDeviceNotesUpdateCommand(body) =>
        handle_pd_notes(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    PhysicalDeviceDeleteCommand(body) =>
        handle_pd_forget(publisher, tid, corr, primary_adapter_id, adapter_count, store, counters, body),
    HclScheduleUpsertCommand(body) =>
        handle_hcl_schedule_upsert(publisher, tid, corr, primary_adapter_id, store, counters, body),
    HclScheduleDeleteCommand(body) =>
        handle_hcl_schedule_delete(publisher, tid, corr, primary_adapter_id, store, counters, body),
    InputDeviceMetadataUpdateCommand(body) => {
        super::input_devices::handle_input_device_metadata(publisher, corr, store, counters, body);
    },
    InputDeviceNotesUpdateCommand(body) => {
        super::input_devices::handle_input_device_notes(publisher, corr, store, counters, body);
    },
    PollerSettingsUpdateCommand(body) =>
        handle_poller_settings_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    DaliSettingsUpdateCommand(body) =>
        handle_dali_settings_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    RedundancySettingsUpdateCommand(body) =>
        handle_redundancy_settings_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    PoliciesUpdateCommand(body) =>
        handle_policies_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    RegistrySliceReloadCommand(body) =>
        handle_slice_reload(publisher, tid, corr, primary_adapter_id, store, persistence_slices, adapter_count, counters, body),
    HomeAssistantSettingsUpdateCommand(body) =>
        handle_settings_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    HomeAssistantCredentialsUpdateCommand(body) =>
        handle_credentials_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    HomeAssistantTopicsUpdateCommand(body) =>
        handle_topics_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
    HomeAssistantControllerIdUpdateCommand(body) =>
        handle_controller_id_update(publisher, tid, corr, primary_adapter_id, store, counters, body),
}
