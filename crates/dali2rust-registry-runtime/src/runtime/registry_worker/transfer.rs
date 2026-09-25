use std::sync::atomic::Ordering;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::RegistrySliceReloadCommand;
use dali2rust_platform::slice_store::SliceStore;

use crate::runtime::registry::RegistryStore;

use super::RegistryCommandCounters;

pub(super) fn handle_slice_reload(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    store: &RegistryStore,
    slices: Option<&std::sync::Arc<dyn SliceStore>>,
    adapter_count: u8,
    counters: &RegistryCommandCounters,
    body: &RegistrySliceReloadCommand,
) {
    if super::confirm::check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err() {
        return;
    }
    let Some(slices) = slices else {
        counters
            .ignored_commands
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::confirm::publish_correlation_failed(
            publisher,
            corr,
            dali2rust_contracts::msg::ErrorCode::Conflict,
            "persistence_disabled",
        );
        return;
    };
    let Some(counts) = reload_on_hydration_stack(store, slices.as_ref(), adapter_count) else {
        refuse_reload(publisher, corr, store);
        return;
    };
    log::info!(
        "registry: reload after import of {} — {} loaded, {} defaulted, {} errors",
        body.slice_name.as_str(),
        counts.loaded,
        counts.defaulted,
        counts.errors
    );
    count_reload(counters);
    announce_reload(publisher, primary_adapter_id, body);
    super::confirm::publish_correlation_ok(publisher, corr);
}

fn count_reload(counters: &RegistryCommandCounters) {
    counters.slice_reloads.fetch_add(1, Ordering::Relaxed);
    counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
    counters.home_assistant_settings_applied.fetch_add(1, Ordering::Relaxed);
}

fn announce_reload(
    publisher: &BusPublisher,
    primary_adapter_id: BusId,
    body: &RegistrySliceReloadCommand,
) {
    let ev = dali2rust_contracts::bus::event_envelope(
        dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
        dali2rust_contracts::CORRELATION_NONE,
        primary_adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Registry),
        dali2rust_contracts::msg::RegistrySliceReloadedEvent {
            slice_name: dali2rust_contracts::msg::fixed_text_32(body.slice_name.as_str()),
        },
    );
    dali2rust_bus::publish_required(
        publisher,
        dali2rust_bus::BusChannel::Events,
        dali2rust_bus::BusFrame::event(ev),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "registry-slice-reloaded",
    );
}

fn refuse_reload(publisher: &BusPublisher, corr: u64, store: &RegistryStore) {
    store
        .persist_counters
        .hydrate_error_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    super::confirm::publish_correlation_failed(
        publisher,
        corr,
        dali2rust_contracts::msg::ErrorCode::Conflict,
        "reload_stack_unavailable",
    );
}

static RELOAD_STACK: dali2rust_bsp::stack_probe::StackLowWater =
    dali2rust_bsp::stack_probe::StackLowWater::new(
        "registry-reload",
        dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
    );

fn reload_on_hydration_stack(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    adapter_count: u8,
) -> Option<crate::runtime::registry::HydrateCounts> {
    let snapshot = dali2rust_platform::slice_store::InMemorySlices::snapshot(slices, adapter_count);
    log::info!("registry: reload snapshot holds {} B", snapshot.bytes());
    dali2rust_bsp::esp_thread::run_on_external_stack(
        c"registry-reload",
        dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
        || {
            let counts = store.hydrate_counts_from_store(&snapshot, adapter_count);
            RELOAD_STACK.note("hydrate");
            counts
        },
    )
    .map_err(|e| log::error!("registry: reload refused, no stack for hydration: {e}"))
    .ok()
}
