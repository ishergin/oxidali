use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_contracts::msg::ErrorCode;

use crate::runtime::registry::RegistryStore;

use super::confirm::{publish_correlation_failed, publish_correlation_ok};
use super::{RegistryCommandCounters, SlicePersistence};

#[derive(Clone, Copy)]
pub(super) struct CommandCtx<'a> {
    pub(super) publisher: &'a BusPublisher,
    pub(super) tid: u16,
    pub(super) corr: u64,
    pub(super) primary_adapter_id: BusId,
    pub(super) adapter_count: u8,
    pub(super) store: &'a Arc<RegistryStore>,
    pub(super) counters: &'a RegistryCommandCounters,
    pub(super) persistence: Option<&'a SlicePersistence>,
}

impl CommandCtx<'_> {
    pub(super) fn confirm_ok(&self) {
        publish_correlation_ok(self.publisher, self.corr);
    }

    pub(super) fn confirm_failed(&self, code: ErrorCode, message: &str) {
        publish_correlation_failed(self.publisher, self.corr, code, message);
    }

    pub(super) fn reject(&self, message: &str) {
        self.counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        self.confirm_failed(ErrorCode::InvalidValue, message);
    }

    pub(super) fn check_primary_adapter(&self) -> Result<(), ()> {
        if BusId(self.tid) != self.primary_adapter_id {
            self.confirm_failed(ErrorCode::InvalidResourceId, "wrong_target_adapter");
            Err(())
        } else {
            Ok(())
        }
    }

    pub(super) fn check_adapter_id(&self, adapter_id: u8) -> Result<(), ()> {
        if adapter_id >= self.adapter_count {
            self.counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
            self.confirm_failed(ErrorCode::InvalidResourceId, "adapter_id_out_of_range");
            Err(())
        } else {
            Ok(())
        }
    }

    pub(super) fn check_adapter_target(&self, adapter_id: u8) -> Result<(), ()> {
        self.check_primary_adapter()?;
        self.check_adapter_id(adapter_id)
    }

    pub(super) fn update_global_settings<A>(
        &self,
        applied_counter: &AtomicU32,
        apply: impl FnOnce() -> A,
        publish_changed: impl FnOnce(&BusPublisher, u64, u16, &A),
    ) {
        if self.check_primary_adapter().is_err() {
            return;
        }
        let applied = apply();
        publish_changed(self.publisher, self.corr, self.tid, &applied);
        applied_counter.fetch_add(1, Ordering::Relaxed);
        self.counters.config_updates_applied.fetch_add(1, Ordering::Relaxed);
        self.confirm_ok();
    }
}
