use dali2rust_contracts::msg::{DaliSettingsUpdateCommand, ErrorCode};

use crate::runtime::registry::publish::publish_dali_settings_changed;

use super::command_ctx::CommandCtx;

const SHORT_ADDRESS_TAKEN_BY_PEER: &str = "short_address_taken_by_peer";

pub(super) fn handle_dali_settings_update(ctx: &CommandCtx<'_>, body: &DaliSettingsUpdateCommand) {
    if ctx.store.claims_the_peer_address(body) {
        ctx.confirm_failed(ErrorCode::Conflict, SHORT_ADDRESS_TAKEN_BY_PEER);
        return;
    }
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
