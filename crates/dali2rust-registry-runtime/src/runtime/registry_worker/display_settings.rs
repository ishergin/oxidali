use dali2rust_contracts::msg::DisplaySettingsUpdateCommand;

use crate::runtime::registry::publish::publish_display_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_display_settings_update(
    ctx: &CommandCtx<'_>,
    body: &DisplaySettingsUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.display_settings_applied,
        || ctx.store.apply_display_settings_from_command(body),
        publish_display_settings_changed,
    );
}
