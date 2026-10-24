use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{ConfigWriteResource, ErrorCode};

use crate::runtime::registry::config_write_stage::ConfigWriteRejection;
use crate::runtime::registry::RegistryStore;

use super::command_ctx::CommandCtx;
use super::RegistryCommandCounters;

pub(super) use dali2rust_domain::registry::{GROUP_COUNT, SCENE_COUNT, VIRTUAL_LAMP_COUNT};

fn finish_stage(ctx: &CommandCtx<'_>, staged: Result<(), ConfigWriteRejection>) {
    match staged {
        Ok(()) => ctx.confirm_ok(),
        Err(rejection) => ctx.reject(rejection.message()),
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

pub(super) fn handle_metadata<M: MetadataCommand>(ctx: &CommandCtx<'_>, body: &M) {
    if ctx.check_adapter_target(body.adapter_id()).is_err() {
        return;
    }
    if body.id() >= M::ID_COUNT {
        ctx.confirm_failed(ErrorCode::InvalidValue, M::ID_OUT_OF_RANGE);
        return;
    }
    body.apply_and_publish(ctx.publisher, ctx.corr, ctx.store);
    M::applied_counter(ctx.counters).fetch_add(1, Ordering::Relaxed);
    ctx.counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
    ctx.confirm_ok();
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

pub(super) fn stage_matrix_chunk<R: MatrixResource>(
    ctx: &CommandCtx<'_>,
    adapter_id: u8,
    scene_id: u8,
    rows: &[R::Row],
) {
    if ctx.check_adapter_target(adapter_id).is_err() {
        return;
    }
    if let Err(message) = R::check_scope(scene_id) {
        ctx.confirm_failed(ErrorCode::InvalidValue, message);
        return;
    }
    if !rows.iter().all(R::row_is_valid) {
        ctx.reject(R::ROW_REJECT);
        return;
    }
    finish_stage(ctx, R::stage(ctx.store, ctx.corr, adapter_id, scene_id, rows));
}

pub(super) fn commit_matrix<R>(
    ctx: &CommandCtx<'_>,
    resource: ConfigWriteResource,
    adapter_id: u8,
    scene_id: u8,
    chunks: u8,
    applied: &AtomicU32,
    on_changed: impl FnOnce() -> R,
) -> Result<(), &'static str> {
    match ctx.store.commit_config_write(ctx.corr, resource, adapter_id, scene_id, chunks) {
        Ok(_changed) => {
            on_changed();
            applied.fetch_add(1, Ordering::Relaxed);
            ctx.counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
            ctx.confirm_ok();
            Ok(())
        }
        Err(rejection) => {
            ctx.reject(rejection.message());
            Err(rejection.message())
        }
    }
}
