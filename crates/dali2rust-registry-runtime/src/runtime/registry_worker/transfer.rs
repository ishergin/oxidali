use std::sync::atomic::Ordering;
use std::sync::Arc;

use dali2rust_bsp::stack_runner::{RunError, StackRunner};
use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{ErrorCode, RegistrySliceReloadCommand};
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::slice_store::{SliceKey, SliceStore, StoreError};

use crate::runtime::registry::publish::publish_config_write_signal;
use crate::runtime::registry::transfer::ImportWriteFailure;
use crate::runtime::registry::{HydrateCounts, RegistryStore};

use super::command_ctx::{CommandCtx, ReloadPorts};
use super::{RegistryCommandCounters, SlicePersistence};

pub(super) fn handle_slice_reload(
    ctx: &CommandCtx<'_>,
    reload: ReloadPorts<'_>,
    body: &RegistrySliceReloadCommand,
) {
    if ctx.check_primary_adapter().is_err() {
        return;
    }
    let imported = import_then_reload(reload.store, reload.persistence, ctx.adapter_count, ctx.corr);
    if let Some(counts) = imported.counts {
        log::info!(
            "registry: reload after import of {} — {} loaded, {} defaulted, {} errors",
            body.slice_name.as_str(),
            counts.loaded,
            counts.defaulted,
            counts.errors
        );
        count_reload(ctx.counters);
    }
    if imported.written {
        announce_reload(ctx.publisher, ctx.primary_adapter_id, body);
    }
    if let Some((_, why)) = imported.failure {
        log::warn!("registry: import of {} failed: {why}", body.slice_name.as_str());
    }
    if ctx.corr != CORRELATION_NONE {
        publish_config_write_signal(
            ctx.publisher,
            ctx.corr,
            ctx.primary_adapter_id,
            imported.failure,
            ctx.counters,
        );
    }
}

type ImportFailure = (ErrorCode, &'static str);

const PERSISTENCE_DISABLED: ImportFailure = (ErrorCode::Conflict, "persistence_disabled");
const NO_STAGE: ImportFailure = (ErrorCode::InvalidValue, "import_no_stage");
const INVALID_SLICE: ImportFailure = (ErrorCode::InvalidValue, "invalid_slice");
const FLASH_BUSY: ImportFailure = (ErrorCode::OperationFailed, "flash_busy");
const STORE_FAILED: ImportFailure = (ErrorCode::OperationFailed, "store_failed");
const NO_RELOAD_STACK: ImportFailure = (ErrorCode::Conflict, "reload_stack_unavailable");
const RELOAD_JOB_LOST: ImportFailure = (ErrorCode::OperationFailed, "reload_job_lost");
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
    store: &Arc<RegistryStore>,
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
    store: &Arc<RegistryStore>,
    persistence: &SlicePersistence,
    corr: u64,
) -> Result<Written, ImportFailure> {
    let staged = store.take_import(corr).ok_or(NO_STAGE)?;
    let (validating, foreign) = (Arc::clone(store), Arc::clone(&persistence.foreign));
    let (staged, refused) =
        on_hydration_stack("validate", move || validating.keep_decodable(staged, foreign.as_ref()))?;
    if staged.is_empty() {
        return Err(INVALID_SLICE);
    }
    let invalid = (refused > 0).then_some(INVALID_SLICE);
    let keys = |upto: usize| staged.iter().take(upto).map(|slice| slice.key).collect();
    match store.write_staged(persistence.slices.as_ref(), &staged) {
        Ok(()) => Ok(Written { keys: keys(staged.len()), failure: invalid }),
        Err(ImportWriteFailure::FirmwareWriteOpen) => Err(FLASH_BUSY),
        Err(ImportWriteFailure::Store { written: 0, error }) => Err(write_failure(&error)),
        Err(ImportWriteFailure::Store { written, error }) => {
            Ok(Written { keys: keys(written), failure: Some(write_failure(&error)) })
        }
    }
}

fn write_failure(error: &StoreError) -> ImportFailure {
    if matches!(error, StoreError::Deferred) {
        FLASH_BUSY
    } else {
        STORE_FAILED
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
    store: &Arc<RegistryStore>,
    slices: &dyn SliceStore,
    adapter_count: u8,
) -> Result<HydrateCounts, ImportFailure> {
    let snapshot = dali2rust_platform::slice_store::InMemorySlices::snapshot(slices, adapter_count);
    log::info!("registry: reload snapshot holds {} B", snapshot.bytes());
    let hydrating = Arc::clone(store);
    on_hydration_stack("hydrate", move || hydrating.hydrate_counts_from_store(&snapshot, adapter_count))
}

static HYDRATION: StackRunner = StackRunner::new(
    c"registry-reload",
    dali2rust_bsp::std_thread_stack::HYDRATION_WORKER,
    dali2rust_bsp::esp_thread::StackHome::External,
);

fn on_hydration_stack<T: Send + 'static>(
    note: &'static str,
    run: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ImportFailure> {
    HYDRATION
        .run(move || {
            let value = run();
            RELOAD_STACK.note(note);
            value
        })
        .map_err(|e| {
            log::error!("registry: import {note} failed: {e}");
            match e {
                RunError::Busy | RunError::Unstarted(_) => NO_RELOAD_STACK,
                RunError::Unanswered => RELOAD_JOB_LOST,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::on_hydration_stack;

    #[test]
    fn the_import_jobs_run_on_one_long_lived_thread_beside_the_registry_worker() {
        let first = on_hydration_stack("test", || std::thread::current().id()).expect("first job");
        let second = on_hydration_stack("test", || std::thread::current().id()).expect("second job");
        assert_eq!(first, second, "each import job spawned a thread of its own");
        assert_ne!(first, std::thread::current().id(), "the import job ran on the registry worker's stack");
    }
}
