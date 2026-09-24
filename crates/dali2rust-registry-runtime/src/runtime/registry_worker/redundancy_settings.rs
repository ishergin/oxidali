use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::RedundancySettingsUpdateCommand;

use crate::runtime::registry::publish::publish_redundancy_settings_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::handle_global_settings_update;
use super::RegistryCommandCounters;

pub(super) fn handle_redundancy_settings_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &RedundancySettingsUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.redundancy_settings_applied,
        || store.apply_redundancy_settings_from_command(body),
        publish_redundancy_settings_changed,
    );
}
