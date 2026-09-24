use std::sync::atomic::Ordering;

use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{AdapterSettingsUpdateCommand, ErrorCode};

use crate::runtime::registry::publish::publish_adapter_settings_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::{check_adapter_id, publish_correlation_failed, publish_correlation_ok};
use super::RegistryCommandCounters;

pub(super) fn handle_adapter_settings(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    adapter_count: u8,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &AdapterSettingsUpdateCommand,
) {
    if check_adapter_id(publisher, counters, tid as u8, adapter_count, corr).is_err() {
        return;
    }
    let aid = tid as u8;
    if !store.apply_adapter_settings_from_command(
        aid,
        body.patch_mask,
        Some(body.name.as_str()),
        body.enabled,
    ) {
        counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
        publish_correlation_failed(
            publisher,
            corr,
            ErrorCode::InvalidValue,
            "adapter_settings_not_applied",
        );
        return;
    }
    let (n, en) = store
        .adapter_row_name_enabled(aid)
        .expect("adapter row after apply");
    publish_adapter_settings_changed(publisher, aid, corr, &n, en);
    counters
        .adapter_settings_applied
        .fetch_add(1, Ordering::Relaxed);
    publish_correlation_ok(publisher, corr);
}
