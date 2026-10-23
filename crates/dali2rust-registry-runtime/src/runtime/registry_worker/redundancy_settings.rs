use dali2rust_contracts::msg::RedundancySettingsUpdateCommand;

use crate::runtime::registry::publish::publish_redundancy_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_redundancy_settings_update(
    ctx: &CommandCtx<'_>,
    body: &RedundancySettingsUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.redundancy_settings_applied,
        || ctx.store.apply_redundancy_settings_from_command(body),
        publish_redundancy_settings_changed,
    );
}
