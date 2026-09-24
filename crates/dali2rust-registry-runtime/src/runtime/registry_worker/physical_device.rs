use std::sync::atomic::Ordering;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    ErrorCode, PhysicalDeviceDeleteCommand, PhysicalDeviceNotesUpdateCommand,
    PhysicalDeviceOverrideCommand,
};

use crate::runtime::registry::publish::{
    publish_group_matrix_changed, publish_physical_device_changed,
    publish_virtual_lamp_changed,
};
use crate::runtime::registry::RegistryStore;

use super::confirm::{
    check_adapter_id, check_primary_adapter, publish_correlation_failed, publish_correlation_ok,
};
use super::RegistryCommandCounters;

pub(super) fn handle_pd_override(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &PhysicalDeviceOverrideCommand,
) {
    handle_pd_mutation(
        PdMutationCtx {
            publisher,
            tid,
            corr,
            primary_adapter_id,
            adapter_count,
            counters,
            adapter_id: body.adapter_id,
            short_address: body.short_address,
        },
        || {
            store.apply_physical_device_override(body)
        },
    );
}

pub(super) fn handle_pd_notes(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &PhysicalDeviceNotesUpdateCommand,
) {
    handle_pd_mutation(
        PdMutationCtx {
            publisher,
            tid,
            corr,
            primary_adapter_id,
            adapter_count,
            counters,
            adapter_id: body.adapter_id,
            short_address: body.short_address,
        },
        || {
            store.apply_physical_device_notes(
                body.adapter_id,
                body.short_address,
                body.notes.as_str(),
            )
        },
    );
}

pub(super) fn handle_pd_forget(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &PhysicalDeviceDeleteCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    let Some(outcome) = store.apply_physical_device_forget(body.adapter_id, body.short_address)
    else {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(
            publisher,
            corr,
            ErrorCode::NotFound,
            "physical_device_not_found",
        );
        return;
    };
    publish_physical_device_changed(publisher, corr, body.adapter_id, body.short_address);
    for lamp_id in &outcome.unbound_lamps {
        publish_virtual_lamp_changed(publisher, corr, body.adapter_id, *lamp_id);
    }
    if outcome.groups_changed {
        publish_group_matrix_changed(publisher, corr, body.adapter_id);
    }
    counters
        .physical_device_overrides_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

struct PdMutationCtx<'a> {
    publisher: &'a BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    counters: &'a RegistryCommandCounters,
    adapter_id: u8,
    short_address: u8,
}

fn handle_pd_mutation(ctx: PdMutationCtx<'_>, apply: impl FnOnce() -> bool) {
    if check_primary_adapter(ctx.publisher, ctx.tid, ctx.primary_adapter_id, ctx.corr).is_err()
        || check_adapter_id(
            ctx.publisher,
            ctx.counters,
            ctx.adapter_id,
            ctx.adapter_count,
            ctx.corr,
        )
        .is_err()
    {
        return;
    }
    if !apply() {
        ctx.counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(
            ctx.publisher,
            ctx.corr,
            ErrorCode::InvalidValue,
            "physical_device_override_not_applied",
        );
        return;
    }
    publish_physical_device_changed(ctx.publisher, ctx.corr, ctx.adapter_id, ctx.short_address);
    ctx.counters
        .physical_device_overrides_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(ctx.publisher, ctx.corr);
}
