use dali2rust_contracts::msg::{ErrorCode, RedundancySettingsUpdateCommand};

use crate::runtime::registry::publish::publish_redundancy_settings_changed;

use super::command_ctx::CommandCtx;

const PEER_ADDRESS_IS_THIS_CONTROLLER: &str = "peer_address_is_this_controller";

pub(super) fn handle_redundancy_settings_update(
    ctx: &CommandCtx<'_>,
    body: &RedundancySettingsUpdateCommand,
) {
    if ctx.store.claims_our_own_address(body) {
        ctx.confirm_failed(ErrorCode::Conflict, PEER_ADDRESS_IS_THIS_CONTROLLER);
        return;
    }
    ctx.update_global_settings(
        &ctx.counters.redundancy_settings_applied,
        || ctx.store.apply_redundancy_settings_from_command(body),
        publish_redundancy_settings_changed,
    );
}
