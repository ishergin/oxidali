use dali2rust_contracts::msg::PoliciesUpdateCommand;

use crate::runtime::registry::publish::publish_policies_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_policies_update(ctx: &CommandCtx<'_>, body: &PoliciesUpdateCommand) {
    ctx.update_global_settings(
        &ctx.counters.policies_applied,
        || ctx.store.apply_policies_from_command(body),
        publish_policies_changed,
    );
}
