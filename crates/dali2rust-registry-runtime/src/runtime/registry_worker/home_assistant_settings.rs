use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    HomeAssistantControllerIdUpdateCommand, HomeAssistantCredentialsUpdateCommand,
    HomeAssistantSettingsUpdateCommand, HomeAssistantTopicsUpdateCommand,
};

use crate::runtime::registry::home_assistant_settings::HomeAssistantSettingsRecord;
use crate::runtime::registry::publish::publish_home_assistant_settings_changed;

use super::command_ctx::CommandCtx;

fn publish_changed(
    publisher: &BusPublisher,
    corr: u64,
    tid: u16,
    applied: &HomeAssistantSettingsRecord,
) {
    publish_home_assistant_settings_changed(publisher, corr, tid, applied.enabled);
}

pub(super) fn handle_settings_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantSettingsUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.home_assistant_settings_applied,
        || ctx.store.apply_home_assistant_settings_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_credentials_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantCredentialsUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.home_assistant_settings_applied,
        || ctx.store.apply_home_assistant_credentials_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_topics_update(ctx: &CommandCtx<'_>, body: &HomeAssistantTopicsUpdateCommand) {
    ctx.update_global_settings(
        &ctx.counters.home_assistant_settings_applied,
        || ctx.store.apply_home_assistant_topics_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_controller_id_update(
    ctx: &CommandCtx<'_>,
    body: &HomeAssistantControllerIdUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.home_assistant_settings_applied,
        || ctx.store.apply_home_assistant_controller_id_from_command(body),
        publish_changed,
    );
}
