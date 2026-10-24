use std::sync::atomic::Ordering;

use dali2rust_contracts::msg::{
    ErrorCode, VirtualLampBindCommand, VirtualLampConfigUpdateCommand, VirtualLampDeleteCommand,
    VirtualLampRebindCommand, VirtualLampUnbindCommand,
};

use crate::runtime::registry::publish::publish_virtual_lamp_changed;

use super::command_ctx::CommandCtx;

pub(super) fn handle_vl_config(ctx: &CommandCtx<'_>, body: &VirtualLampConfigUpdateCommand) {
    if ctx.check_adapter_target(body.adapter_id).is_err() {
        return;
    }
    ctx.store.apply_virtual_lamp_config_patch(
        body.adapter_id,
        body.virtual_lamp_id,
        body.patch_mask,
        Some(body.name.as_str()),
        body.ha_entity_enabled,
    );
    publish_virtual_lamp_changed(ctx.publisher, ctx.corr, body.adapter_id, body.virtual_lamp_id);
    ctx.counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.counters
        .virtual_lamp_metadata_patches_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}

pub(super) fn handle_vl_bind(ctx: &CommandCtx<'_>, body: &VirtualLampBindCommand) {
    handle_vl_bind_or_rebind(
        ctx,
        body.adapter_id,
        body.virtual_lamp_id,
        body.physical_short_address,
        "virtual_lamp_bind_rejected",
    );
}

pub(super) fn handle_vl_rebind(ctx: &CommandCtx<'_>, body: &VirtualLampRebindCommand) {
    handle_vl_bind_or_rebind(
        ctx,
        body.adapter_id,
        body.virtual_lamp_id,
        body.physical_short_address,
        "virtual_lamp_rebind_rejected",
    );
}

fn handle_vl_bind_or_rebind(
    ctx: &CommandCtx<'_>,
    adapter_id: u8,
    virtual_lamp_id: u8,
    physical_short_address: u8,
    reject_message: &str,
) {
    if ctx.check_adapter_target(adapter_id).is_err() {
        return;
    }
    if !ctx.store.apply_virtual_lamp_bind(adapter_id, virtual_lamp_id, physical_short_address) {
        ctx.reject_with(ErrorCode::Conflict, reject_message);
        return;
    }
    publish_virtual_lamp_changed(ctx.publisher, ctx.corr, adapter_id, virtual_lamp_id);
    ctx.counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.counters
        .virtual_lamp_bindings_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}

pub(super) fn handle_vl_unbind(ctx: &CommandCtx<'_>, body: &VirtualLampUnbindCommand) {
    if ctx.check_adapter_target(body.adapter_id).is_err() {
        return;
    }
    ctx.store.apply_virtual_lamp_unbind(body.adapter_id, body.virtual_lamp_id);
    publish_virtual_lamp_changed(ctx.publisher, ctx.corr, body.adapter_id, body.virtual_lamp_id);
    ctx.counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.counters
        .virtual_lamp_bindings_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}

pub(super) fn handle_vl_delete(ctx: &CommandCtx<'_>, body: &VirtualLampDeleteCommand) {
    if ctx.check_adapter_target(body.adapter_id).is_err() {
        return;
    }
    if !ctx.store.apply_virtual_lamp_delete(body.adapter_id, body.virtual_lamp_id) {
        ctx.reject_with(ErrorCode::NotFound, "virtual_lamp_not_found");
        return;
    }
    publish_virtual_lamp_changed(ctx.publisher, ctx.corr, body.adapter_id, body.virtual_lamp_id);
    ctx.counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.counters
        .virtual_lamp_metadata_patches_applied
        .fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
}
