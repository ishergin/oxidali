use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_contracts::msg::{
    ClusterPartUpdateCommand, ClusterRecordCopyCommand, ClusterRecordTakeBackCommand, ClusterRecordUpdateCommand,
    ClusterSettingsCommitCommand, ClusterSettingsUpdateCommand, ClusterTextUpdateCommand,
};

use crate::runtime::registry::cluster_copies::CopyOutcome;
use crate::runtime::registry::cluster_records::TakeBackOutcome;
use crate::runtime::registry::cluster_settings::{ClusterPatch, ClusterRefusal, ClusterSettingsRecord};
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

pub(super) fn handle_cluster_part_update(ctx: &CommandCtx<'_>, body: &ClusterPartUpdateCommand) {
    stage(ctx, body.series, ClusterPatch::Part(body.clone()));
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

pub(super) fn handle_cluster_record_copy(ctx: &CommandCtx<'_>, body: &ClusterRecordCopyCommand) {
    if ctx.check_primary_adapter().is_err() {
        return;
    }
    let applied = ctx.store.apply_cluster_copy(body).map(|(outcome, record)| {
        (outcome != CopyOutcome::Unchanged, outcome == CopyOutcome::Stored, record)
    });
    answer_learned(ctx, applied, &ctx.counters.cluster_copies_stored);
}

pub(super) fn handle_cluster_record_take_back(ctx: &CommandCtx<'_>, body: &ClusterRecordTakeBackCommand) {
    if ctx.check_primary_adapter().is_err() {
        return;
    }
    let applied = ctx.store.apply_cluster_take_back(body).map(|(outcome, record)| {
        (outcome != TakeBackOutcome::Unchanged, outcome == TakeBackOutcome::Adopted, record)
    });
    answer_learned(ctx, applied, &ctx.counters.cluster_records_taken_back);
}

fn answer_learned(
    ctx: &CommandCtx<'_>,
    applied: Result<(bool, bool, ClusterSettingsRecord), ClusterRefusal>,
    counter: &AtomicU32,
) {
    match applied {
        Ok((changed, counted, record)) => {
            if changed {
                publish_cluster_settings_changed(ctx.publisher, ctx.corr, ctx.tid, &record);
            }
            if counted {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            ctx.confirm_ok();
        }
        Err(refusal) => refuse(ctx, refusal),
    }
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
