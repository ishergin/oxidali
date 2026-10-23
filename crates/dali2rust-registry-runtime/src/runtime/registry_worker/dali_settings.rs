use dali2rust_contracts::msg::DaliSettingsUpdateCommand;

use crate::runtime::registry::publish::publish_dali_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_dali_settings_update(ctx: &CommandCtx<'_>, body: &DaliSettingsUpdateCommand) {
    let was_active = ctx.store.read_inner().dali_settings.application_active;
    ctx.update_global_settings(
        &ctx.counters.dali_settings_applied,
        || ctx.store.apply_dali_settings_from_command(body),
        |publisher, corr, target, applied| {
            let moved_by = if applied.application_active == was_active {
                dali2rust_contracts::msg::APPLICATION_ACTIVE_UNMOVED
            } else {
                dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_COMMAND
            };
            publish_dali_settings_changed(publisher, corr, target, applied, moved_by);
        },
    );
}
