use std::sync::atomic::Ordering;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{
    ErrorCode, VirtualLampBindCommand, VirtualLampConfigUpdateCommand, VirtualLampDeleteCommand,
    VirtualLampRebindCommand, VirtualLampUnbindCommand,
};

use crate::runtime::registry::publish::publish_virtual_lamp_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::{
    check_adapter_id, check_primary_adapter, publish_correlation_failed, publish_correlation_ok,
};
use super::RegistryCommandCounters;

pub(super) fn handle_vl_config(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &VirtualLampConfigUpdateCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    store.apply_virtual_lamp_config_patch(
        body.adapter_id,
        body.virtual_lamp_id,
        body.patch_mask,
        Some(body.name.as_str()),
        body.ha_entity_enabled,
    );
    publish_virtual_lamp_changed(publisher, corr, body.adapter_id, body.virtual_lamp_id);
    counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    counters
        .virtual_lamp_metadata_patches_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

pub(super) fn handle_vl_bind(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &VirtualLampBindCommand,
) {
    handle_vl_bind_or_rebind(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        body.adapter_id,
        body.virtual_lamp_id,
        body.physical_short_address,
        "virtual_lamp_bind_rejected",
    );
}

pub(super) fn handle_vl_rebind(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &VirtualLampRebindCommand,
) {
    handle_vl_bind_or_rebind(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        adapter_count,
        store,
        counters,
        body.adapter_id,
        body.virtual_lamp_id,
        body.physical_short_address,
        "virtual_lamp_rebind_rejected",
    );
}

fn handle_vl_bind_or_rebind(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    adapter_id: u8,
    virtual_lamp_id: u8,
    physical_short_address: u8,
    reject_message: &str,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    if !store.apply_virtual_lamp_bind(adapter_id, virtual_lamp_id, physical_short_address) {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(publisher, corr, ErrorCode::Conflict, reject_message);
        return;
    }
    publish_virtual_lamp_changed(publisher, corr, adapter_id, virtual_lamp_id);
    counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    counters
        .virtual_lamp_bindings_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

pub(super) fn handle_vl_unbind(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &VirtualLampUnbindCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    store.apply_virtual_lamp_unbind(body.adapter_id, body.virtual_lamp_id);
    publish_virtual_lamp_changed(publisher, corr, body.adapter_id, body.virtual_lamp_id);
    counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    counters
        .virtual_lamp_bindings_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

pub(super) fn handle_vl_delete(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &VirtualLampDeleteCommand,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    if !store.apply_virtual_lamp_delete(body.adapter_id, body.virtual_lamp_id) {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(
            publisher,
            corr,
            ErrorCode::NotFound,
            "virtual_lamp_not_found",
        );
        return;
    }
    publish_virtual_lamp_changed(publisher, corr, body.adapter_id, body.virtual_lamp_id);
    counters
        .config_updates_applied
        .fetch_add(1, Ordering::Relaxed);
    counters
        .virtual_lamp_metadata_patches_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}
