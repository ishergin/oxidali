use std::sync::atomic::Ordering;

use dali2rust_contracts::msg::{
    ErrorCode, PhysicalDeviceDeleteCommand, PhysicalDeviceNotesUpdateCommand,
    PhysicalDeviceOverrideCommand,
};

use crate::runtime::registry::publish::{publish_physical_device_changed, publish_unbound_lamps};

use super::command_ctx::CommandCtx;

pub(super) fn handle_pd_override(ctx: &CommandCtx<'_>, body: &PhysicalDeviceOverrideCommand) {
    handle_pd_mutation(ctx, body.adapter_id, body.short_address, || {
        ctx.store.apply_physical_device_override(body)
    });
}

pub(super) fn handle_pd_notes(ctx: &CommandCtx<'_>, body: &PhysicalDeviceNotesUpdateCommand) {
    handle_pd_mutation(ctx, body.adapter_id, body.short_address, || {
        ctx.store.apply_physical_device_notes(
            body.adapter_id,
            body.short_address,
            body.notes.as_str(),
        )
    });
}

pub(super) fn handle_pd_forget(ctx: &CommandCtx<'_>, body: &PhysicalDeviceDeleteCommand) {
    if ctx.check_adapter_target(body.adapter_id).is_err() {
        return;
    }
    let Some(outcome) = ctx
        .store
        .apply_physical_device_forget(body.adapter_id, body.short_address)
    else {
        ctx.reject_with(ErrorCode::NotFound, "physical_device_not_found");
        return;
    };
    publish_physical_device_changed(ctx.publisher, ctx.corr, body.adapter_id, body.short_address);
    publish_unbound_lamps(ctx.publisher, ctx.corr, body.adapter_id, &outcome);
    ctx.counters
        .physical_device_overrides_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}

fn handle_pd_mutation(
    ctx: &CommandCtx<'_>,
    adapter_id: u8,
    short_address: u8,
    apply: impl FnOnce() -> bool,
) {
    if ctx.check_adapter_target(adapter_id).is_err() {
        return;
    }
    if !apply() {
        ctx.reject("device_override_not_applied");
        return;
    }
    publish_physical_device_changed(ctx.publisher, ctx.corr, adapter_id, short_address);
    ctx.counters
        .physical_device_overrides_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}
