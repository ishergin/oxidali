use dali2rust_contracts::msg::PollerSettingsUpdateCommand;

use crate::runtime::registry::publish::publish_poller_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_poller_settings_update(
    ctx: &CommandCtx<'_>,
    body: &PollerSettingsUpdateCommand,
) {
    ctx.update_global_settings(
        &ctx.counters.poller_settings_applied,
        || ctx.store.apply_poller_settings_from_command(body),
        publish_poller_settings_changed,
    );
}
