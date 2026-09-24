use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    InputDeviceMetadataUpdateCommand, InputDeviceNotesUpdateCommand,
};

use crate::runtime::registry::RegistryStore;

use super::RegistryCommandCounters;

pub const PATCH_NAME: u8 = 1 << 0;
pub const PATCH_HA_EXPOSE: u8 = 1 << 1;
pub const PATCH_CLEAR_NAME: u8 = 1 << 2;
pub const PATCH_FORGET: u8 = 1 << 3;

pub(super) fn handle_input_device_metadata(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &InputDeviceMetadataUpdateCommand,
) {
    if body.patch_mask & PATCH_FORGET != 0 {
        let applied =
            store.forget_input_device(body.registry_adapter_id, body.short_address);
        finish(publisher, corr, store, counters, body.registry_adapter_id, body.short_address, applied);
        return;
    }
    let name = if body.patch_mask & PATCH_CLEAR_NAME != 0 {
        Some(None)
    } else if body.patch_mask & PATCH_NAME != 0 {
        Some(Some(body.name.as_str().to_string()))
    } else {
        None
    };
    let ha_expose = (body.patch_mask & PATCH_HA_EXPOSE != 0).then_some(body.ha_expose);

    let applied = store.patch_input_device_metadata(
        body.registry_adapter_id,
        body.short_address,
        name,
        None,
        ha_expose,
    );
    finish(publisher, corr, store, counters, body.registry_adapter_id, body.short_address, applied);
}

pub(super) fn handle_input_device_notes(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &InputDeviceNotesUpdateCommand,
) {
    let notes = body.notes.as_str();
    let applied = store.patch_input_device_metadata(
        body.registry_adapter_id,
        body.short_address,
        None,
        Some((!notes.is_empty()).then(|| notes.to_string())),
        None,
    );
    finish(publisher, corr, store, counters, body.registry_adapter_id, body.short_address, applied);
}

fn finish(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    adapter_id: u8,
    short_address: u8,
    applied: bool,
) {
    if applied {
        counters
            .input_device_metadata_applied
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        crate::runtime::registry::publish::publish_input_device_changed(
            publisher,
            corr,
            adapter_id,
            short_address,
        );
    } else {
        counters
            .input_device_metadata_rejected
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    let _ = store;
}
