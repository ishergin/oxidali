use dali2rust_contracts::msg::{ConfigWriteCommitCommand, ConfigWriteResource, ErrorCode};

use crate::runtime::registry::publish::publish_config_write_signal;

use super::command_ctx::CommandCtx;
use super::matrix::SCENE_COUNT;

pub(super) fn handle_config_write_commit(ctx: &CommandCtx<'_>, body: &ConfigWriteCommitCommand) {
    if ctx.check_adapter_target(body.adapter_id).is_err() {
        return;
    }
    let outcome = match body.resource {
        ConfigWriteResource::GroupMatrix => {
            super::group::commit_group_matrix(ctx, body.adapter_id, body.chunks)
        }
        ConfigWriteResource::SceneMatrix => commit_scene(ctx, body),
    };
    publish_config_write_signal(
        ctx.publisher,
        ctx.corr,
        ctx.primary_adapter_id,
        outcome.err().map(|message| (ErrorCode::InvalidValue, message)),
        ctx.counters,
    );
}

fn commit_scene(ctx: &CommandCtx<'_>, body: &ConfigWriteCommitCommand) -> Result<(), &'static str> {
    let Some(scene_id) = body.scene_id else {
        ctx.reject("scene_id_missing");
        return Err("scene_id_missing");
    };
    if scene_id >= SCENE_COUNT {
        ctx.reject("scene_id_out_of_range");
        return Err("scene_id_out_of_range");
    }
    super::scene::commit_scene_matrix(ctx, body.adapter_id, scene_id, body.chunks)
}
