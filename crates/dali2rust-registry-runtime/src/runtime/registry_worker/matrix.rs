use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::{ConfigWriteResource, ErrorCode};

use crate::runtime::registry::config_write_stage::ConfigWriteRejection;
use crate::runtime::registry::RegistryStore;

use super::confirm::{
    check_adapter_id, check_primary_adapter, publish_correlation_failed, publish_correlation_ok,
};
use super::RegistryCommandCounters;

pub(super) use dali2rust_domain::registry::{GROUP_COUNT, SCENE_COUNT, VIRTUAL_LAMP_COUNT};

pub(super) fn reject(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    message: &str,
) {
    counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
    publish_correlation_failed(publisher, corr, ErrorCode::InvalidValue, message);
}

pub(super) fn finish_stage(
    publisher: &BusPublisher,
    counters: &RegistryCommandCounters,
    corr: u64,
    staged: Result<(), ConfigWriteRejection>,
) {
    match staged {
        Ok(()) => publish_correlation_ok(publisher, corr),
        Err(rejection) => reject(publisher, counters, corr, rejection.message()),
    }
}

pub(super) trait MetadataCommand {
    const ID_OUT_OF_RANGE: &'static str;
    const ID_COUNT: u8;

    fn adapter_id(&self) -> u8;
    fn id(&self) -> u8;

    fn apply_and_publish(&self, publisher: &BusPublisher, corr: u64, store: &RegistryStore);

    fn applied_counter(counters: &RegistryCommandCounters) -> &AtomicU32;
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-table arity")]
pub(super) fn handle_metadata<M: MetadataCommand>(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &M,
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, body.adapter_id(), adapter_count, corr).is_err()
    {
        return;
    }
    if body.id() >= M::ID_COUNT {
        publish_correlation_failed(publisher, corr, ErrorCode::InvalidValue, M::ID_OUT_OF_RANGE);
        return;
    }
    body.apply_and_publish(publisher, corr, store);
    M::applied_counter(counters).fetch_add(1, Ordering::Relaxed);
    counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}

pub(super) trait MatrixResource {
    type Row;

    const ROW_REJECT: &'static str;

    fn check_scope(scene_id: u8) -> Result<(), &'static str>;

    fn row_is_valid(row: &Self::Row) -> bool;

    fn stage(
        store: &RegistryStore,
        corr: u64,
        adapter_id: u8,
        scene_id: u8,
        rows: &[Self::Row],
    ) -> Result<(), ConfigWriteRejection>;
}

#[allow(clippy::too_many_arguments, reason = "mirrors the dispatch-table arity")]
pub(super) fn stage_matrix_chunk<R: MatrixResource>(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: BusId,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    adapter_id: u8,
    scene_id: u8,
    rows: &[R::Row],
) {
    if check_primary_adapter(publisher, tid, primary_adapter_id, corr).is_err()
        || check_adapter_id(publisher, counters, adapter_id, adapter_count, corr).is_err()
    {
        return;
    }
    if let Err(message) = R::check_scope(scene_id) {
        publish_correlation_failed(publisher, corr, ErrorCode::InvalidValue, message);
        return;
    }
    if !rows.iter().all(R::row_is_valid) {
        reject(publisher, counters, corr, R::ROW_REJECT);
        return;
    }
    finish_stage(
        publisher,
        counters,
        corr,
        R::stage(store, corr, adapter_id, scene_id, rows),
    );
}

pub(super) fn commit_matrix(
    publisher: &BusPublisher,
    corr: u64,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    resource: ConfigWriteResource,
    adapter_id: u8,
    scene_id: u8,
    chunks: u8,
    applied: &AtomicU32,
    on_changed: impl FnOnce(),
) -> Result<(), &'static str> {
    match store.commit_config_write(corr, resource, adapter_id, scene_id, chunks) {
        Ok(_changed) => {
            on_changed();
            applied.fetch_add(1, Ordering::Relaxed);
            counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
            publish_correlation_ok(publisher, corr);
            Ok(())
        }
        Err(rejection) => {
            reject(publisher, counters, corr, rejection.message());
            Err(rejection.message())
        }
    }
}
