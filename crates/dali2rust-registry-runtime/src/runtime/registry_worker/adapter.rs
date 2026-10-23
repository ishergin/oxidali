use std::sync::atomic::Ordering;

use dali2rust_contracts::msg::{AdapterSettingsUpdateCommand, ErrorCode};

use crate::runtime::registry::publish::publish_adapter_settings_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_adapter_settings(ctx: &CommandCtx<'_>, body: &AdapterSettingsUpdateCommand) {
    if ctx.check_adapter_id(ctx.tid as u8).is_err() {
        return;
    }
    let aid = ctx.tid as u8;
    if !ctx.store.apply_adapter_settings_from_command(
        aid,
        body.patch_mask,
        Some(body.name.as_str()),
        body.enabled,
    ) {
        ctx.counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        ctx.confirm_failed(ErrorCode::InvalidValue, "adapter_settings_not_applied");
        return;
    }
    let (n, en) = ctx.store
        .adapter_row_name_enabled(aid)
        .expect("adapter row after apply");
    publish_adapter_settings_changed(ctx.publisher, aid, ctx.corr, &n, en);
    ctx.counters
        .adapter_settings_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}
