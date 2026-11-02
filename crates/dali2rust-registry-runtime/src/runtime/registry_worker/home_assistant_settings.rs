use dali2rust_contracts::msg::{
    HomeAssistantControllerIdUpdateCommand, HomeAssistantCredentialsUpdateCommand,
    HomeAssistantSettingsCommitCommand, HomeAssistantSettingsUpdateCommand,
    HomeAssistantTopicsUpdateCommand,
};

use crate::runtime::registry::home_assistant_settings::HomeAssistantPatch;
use crate::runtime::registry::publish::publish_home_assistant_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_settings_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantSettingsUpdateCommand,
) {
    stage(ctx, body.series, HomeAssistantPatch::Settings(body.clone()));
}

pub(super) fn handle_credentials_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantCredentialsUpdateCommand,
) {
    stage(ctx, body.series, HomeAssistantPatch::Credentials(body.clone()));
}

pub(super) fn handle_topics_update(ctx: &CommandCtx<'_>, body: &HomeAssistantTopicsUpdateCommand) {
    stage(ctx, body.series, HomeAssistantPatch::Topics(body.clone()));
}

pub(super) fn handle_controller_id_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantControllerIdUpdateCommand,
) {
    stage(ctx, body.series, HomeAssistantPatch::ControllerId(body.clone()));
}

pub(super) fn handle_settings_commit(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantSettingsCommitCommand,
) {
    ctx.try_update_global_settings(
        &ctx.counters.home_assistant_settings_applied,
        || {
            ctx.store
                .commit_home_assistant_settings(body.series, body.frames)
                .map_err(|rejection| rejection.message())
        },
        publish_home_assistant_settings_changed,
    );
}

fn stage(ctx: &CommandCtx<'_>, series: u64, patch: HomeAssistantPatch) {
    if ctx.check_primary_adapter().is_err() {
        return;
    }
    match ctx.store.stage_home_assistant_patch(series, patch) {
        Ok(()) => ctx.confirm_ok(),
        Err(rejection) => ctx.reject(rejection.message()),
    }
}
