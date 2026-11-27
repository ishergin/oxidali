use dali2rust_contracts::msg::{
    ClusterRecordUpdateCommand, ClusterSettingsCommitCommand, ClusterSettingsUpdateCommand,
    ClusterTextUpdateCommand,
};

use crate::runtime::registry::cluster_settings::{ClusterPatch, ClusterRefusal};
use crate::runtime::registry::publish::publish_cluster_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_cluster_settings_update(ctx: &CommandCtx<'_>, body: &ClusterSettingsUpdateCommand) {
    stage(ctx, body.series, ClusterPatch::Settings(body.clone()));
}

pub(super) fn handle_cluster_text_update(ctx: &CommandCtx<'_>, body: &ClusterTextUpdateCommand) {
    stage(ctx, body.series, ClusterPatch::Text(body.clone()));
}

pub(super) fn handle_cluster_record_update(ctx: &CommandCtx<'_>, body: &ClusterRecordUpdateCommand) {
    stage(ctx, body.series, ClusterPatch::Record(body.clone()));
}

pub(super) fn handle_cluster_settings_commit(ctx: &CommandCtx<'_>, body: &ClusterSettingsCommitCommand) {
    ctx.try_update_global_settings_or(
        &ctx.counters.cluster_settings_applied,
        || {
            ctx.store
                .commit_cluster_settings(body.series, body.frames)
                .map_err(|refusal| (refusal.code(), refusal.message()))
        },
        publish_cluster_settings_changed,
    );
}

fn stage(ctx: &CommandCtx<'_>, series: u64, patch: ClusterPatch) {
    if ctx.check_primary_adapter().is_err() {
        return;
    }
    match ctx.store.stage_cluster_patch(series, patch) {
        Ok(()) => ctx.confirm_ok(),
        Err(refusal) => refuse(ctx, refusal),
    }
}

fn refuse(ctx: &CommandCtx<'_>, refusal: ClusterRefusal) {
    ctx.reject_with(refusal.code(), refusal.message());
}
