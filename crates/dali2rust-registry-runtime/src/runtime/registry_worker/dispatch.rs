use std::sync::Arc;
use std::sync::atomic::Ordering;

use dali2rust_bus::{BusFrame, BusId, BusPublisher};

use dali2rust_contracts::msg::BusCommandPayload;

use crate::runtime::registry::RegistryStore;

use super::adapter::handle_adapter_settings;
use super::command_ctx::CommandCtx;
use super::config_write::handle_config_write_commit;
use super::group::GroupMatrix;
use super::hcl::{
    handle_hcl_schedule_delete, handle_hcl_schedule_enable, handle_hcl_schedule_upsert,
};
use super::input_devices::{handle_input_device_metadata, handle_input_device_notes};
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
    store: &Arc<RegistryStore>,
    counters: &RegistryCommandCounters,
    persistence: Option<&super::SlicePersistence>,
) {
    let BusFrame::Command(ce_arc) = frame else {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ce = ce_arc.as_ref();
    let ctx = CommandCtx {
        publisher,
        tid: ce.meta.target_adapter_id,
        corr: ce.meta.correlation_id,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        persistence,
    };
    dispatch_registry_command(&ce.payload, &ctx);
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const REGISTRY_WORKER_HANDLED_COMMANDS;
    fn dispatch_registry_command(payload: &BusCommandPayload, ctx: &CommandCtx<'_>);
    payload = payload;
    ignored = { ctx.counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    AdapterSettingsUpdateCommand(body) =>
        handle_adapter_settings(ctx, body),
    VirtualLampConfigUpdateCommand(body) =>
        handle_vl_config(ctx, body),
    GroupMetadataUpdateCommand(body) =>
        handle_metadata(ctx, body),
    GroupMatrixDesiredPatchCommand(body) =>
        stage_matrix_chunk::<GroupMatrix>(ctx, body.adapter_id, GroupMatrix::NO_SCENE, body.rows.as_slice()),
    GroupMatrixDesiredReplaceCommand(body) =>
        stage_matrix_chunk::<GroupMatrix>(ctx, body.adapter_id, GroupMatrix::NO_SCENE, body.rows.as_slice()),
    SceneMetadataUpdateCommand(body) =>
        handle_metadata(ctx, body),
    SceneMatrixDesiredPatchCommand(body) =>
        stage_matrix_chunk::<SceneMatrix>(ctx, body.adapter_id, body.scene_id, body.rows.as_slice()),
    SceneMatrixDesiredReplaceCommand(body) =>
        stage_matrix_chunk::<SceneMatrix>(ctx, body.adapter_id, body.scene_id, body.rows.as_slice()),
    ConfigWriteCommitCommand(body) =>
        handle_config_write_commit(ctx, body),
    VirtualLampBindCommand(body) =>
        handle_vl_bind(ctx, body),
    VirtualLampRebindCommand(body) =>
        handle_vl_rebind(ctx, body),
    VirtualLampUnbindCommand(body) =>
        handle_vl_unbind(ctx, body),
    VirtualLampDeleteCommand(body) =>
        handle_vl_delete(ctx, body),
    RegistryRuntimeUpdateCommand(reg) =>
        handle_runtime_update(ctx, reg),
    RegistryLevelTransitionCommand(cmd) =>
        handle_level_transition(ctx, cmd),
    PhysicalDeviceOverrideCommand(body) =>
        handle_pd_override(ctx, body),
    PhysicalDeviceNotesUpdateCommand(body) =>
        handle_pd_notes(ctx, body),
    PhysicalDeviceDeleteCommand(body) =>
        handle_pd_forget(ctx, body),
    HclScheduleUpsertCommand(body) =>
        handle_hcl_schedule_upsert(ctx, body),
    HclScheduleDeleteCommand(body) =>
        handle_hcl_schedule_delete(ctx, body),
    HclScheduleEnableCommand(body) =>
        handle_hcl_schedule_enable(ctx, body),
    InputDeviceMetadataUpdateCommand(body) =>
        handle_input_device_metadata(ctx, body),
    InputDeviceNotesUpdateCommand(body) =>
        handle_input_device_notes(ctx, body),
    PollerSettingsUpdateCommand(body) =>
        handle_poller_settings_update(ctx, body),
    DaliSettingsUpdateCommand(body) =>
        handle_dali_settings_update(ctx, body),
    RedundancySettingsUpdateCommand(body) =>
        handle_redundancy_settings_update(ctx, body),
    PoliciesUpdateCommand(body) =>
        handle_policies_update(ctx, body),
    RegistrySliceReloadCommand(body) =>
        handle_slice_reload(ctx, body),
    HomeAssistantSettingsUpdateCommand(body) =>
        handle_settings_update(ctx, body),
    HomeAssistantCredentialsUpdateCommand(body) =>
        handle_credentials_update(ctx, body),
    HomeAssistantTopicsUpdateCommand(body) =>
        handle_topics_update(ctx, body),
    HomeAssistantControllerIdUpdateCommand(body) =>
        handle_controller_id_update(ctx, body),
}
