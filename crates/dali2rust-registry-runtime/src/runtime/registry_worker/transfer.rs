use std::sync::atomic::Ordering;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{ErrorCode, RegistrySliceReloadCommand};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};

use crate::runtime::registry::publish::publish_config_write_signal;
use crate::runtime::registry::transfer::ImportWriteFailure;
use crate::runtime::registry::{HydrateCounts, RegistryStore};

use super::{RegistryCommandCounters, SlicePersistence};

pub(super) fn handle_slice_reload(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    store: &RegistryStore,
    persistence: Option<&SlicePersistence>,
    adapter_count: u8,
    counters: &RegistryCommandCounters,
    body: &RegistrySliceReloadCommand,
) {
    if super::confirm::check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err() {
        return;
    }
    let imported = import_then_reload(store, persistence, adapter_count, corr);
    if let Some(counts) = imported.counts {
        log::info!(
            "registry: reload after import of {} — {} loaded, {} defaulted, {} errors",
            body.slice_name.as_str(),
            counts.loaded,
            counts.defaulted,
            counts.errors
        );
        count_reload(counters);
    }
    if imported.written {
        announce_reload(publisher, primary_adapter_id, body);
    }
    if let Some((_, why)) = imported.failure {
        log::warn!("registry: import of {} failed: {why}", body.slice_name.as_str());
    }
    if corr != CORRELATION_NONE {
        publish_config_write_signal(publisher, corr, primary_adapter_id, imported.failure, counters);
    }
}

type ImportFailure = (ErrorCode, &'static str);

const PERSISTENCE_DISABLED: ImportFailure = (ErrorCode::Conflict, "persistence_disabled");
const NO_STAGE: ImportFailure = (ErrorCode::InvalidValue, "import_no_stage");
const INVALID_SLICE: ImportFailure = (ErrorCode::InvalidValue, "invalid_slice");
const FLASH_BUSY: ImportFailure = (ErrorCode::OperationFailed, "flash_busy");
const STORE_FAILED: ImportFailure = (ErrorCode::OperationFailed, "store_failed");
const NO_RELOAD_STACK: ImportFailure = (ErrorCode::Conflict, "reload_stack_unavailable");
const NOT_RELOADED: ImportFailure = (ErrorCode::OperationFailed, "written_not_reloaded");

struct Imported {
    written: bool,
    counts: Option<HydrateCounts>,
    failure: Option<ImportFailure>,
}

impl Imported {
    const fn refused(failure: ImportFailure) -> Self {
        Self { written: false, counts: None, failure: Some(failure) }
    }
}

struct Written {
    keys: Vec<SliceKey>,
    failure: Option<ImportFailure>,
}

fn import_then_reload(
    store: &RegistryStore,
    persistence: Option<&SlicePersistence>,
    adapter_count: u8,
    corr: u64,
) -> Imported {
    let Some(persistence) = persistence else {
        return Imported::refused(PERSISTENCE_DISABLED);
    };
    let written = match validate_and_write(store, persistence, corr) {
        Ok(written) => written,
        Err(failure) => return Imported::refused(failure),
    };
    let reloaded = reload_on_hydration_stack(store, persistence.slices.as_ref(), adapter_count);
    written.keys.iter().for_each(|key| persistence.foreign.imported(*key));
    match reloaded {
        Ok(counts) => Imported { written: true, counts: Some(counts), failure: written.failure },
        Err(_) => {
            store.withhold_until_read(&written.keys, adapter_count);
            Imported { written: true, counts: None, failure: Some(NOT_RELOADED) }
        }
    }
}

fn validate_and_write(
    store: &RegistryStore,
    persistence: &SlicePersistence,
    corr: u64,
) -> Result<Written, ImportFailure> {
    let staged = store.take_import(corr).ok_or(NO_STAGE)?;
    let foreign = persistence.foreign.as_ref();
    let (staged, refused) = on_hydration_stack("validate", || store.keep_decodable(staged, foreign))?;
    if staged.is_empty() {
        return Err(INVALID_SLICE);
    }
    let invalid = (refused > 0).then_some(INVALID_SLICE);
    let keys = |upto: usize| staged.iter().take(upto).map(|slice| slice.key).collect();
    match store.write_staged(persistence.slices.as_ref(), &staged) {
        Ok(()) => Ok(Written { keys: keys(staged.len()), failure: invalid }),
        Err(ImportWriteFailure::FirmwareWriteOpen) => Err(FLASH_BUSY),
        Err(ImportWriteFailure::Store { written: 0, .. }) => Err(STORE_FAILED),
        Err(ImportWriteFailure::Store { written, .. }) => {
            Ok(Written { keys: keys(written), failure: Some(STORE_FAILED) })
        }
    }
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

static RELOAD_STACK: dali2rust_bsp::stack_probe::StackLowWater =
    dali2rust_bsp::stack_probe::StackLowWater::new(
        "registry-reload",
        dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
    );

fn reload_on_hydration_stack(
    store: &RegistryStore,
    slices: &dyn SliceStore,
    adapter_count: u8,
) -> Result<HydrateCounts, ImportFailure> {
    let snapshot = dali2rust_platform::slice_store::InMemorySlices::snapshot(slices, adapter_count);
    log::info!("registry: reload snapshot holds {} B", snapshot.bytes());
    on_hydration_stack("hydrate", || store.hydrate_counts_from_store(&snapshot, adapter_count))
}

fn on_hydration_stack<T: Send>(
    note: &'static str,
    run: impl FnOnce() -> T + Send,
) -> Result<T, ImportFailure> {
    dali2rust_bsp::esp_thread::run_on_external_stack(
        c"registry-reload",
        dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
        || {
            let value = run();
            RELOAD_STACK.note(note);
            value
        },
    )
    .map_err(|e| {
        log::error!("registry: import refused, no stack for hydration: {e}");
        NO_RELOAD_STACK
    })
}
