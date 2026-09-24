use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::DaliSettingsUpdateCommand;

use crate::runtime::registry::publish::publish_dali_settings_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::handle_global_settings_update;
use super::RegistryCommandCounters;

pub(super) fn handle_dali_settings_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &DaliSettingsUpdateCommand,
) {
    let was_active = store.read_inner().dali_settings.application_active;
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.dali_settings_applied,
        || store.apply_dali_settings_from_command(body),
        |publisher, corr, target, applied| {
            let moved_by = if applied.application_active == was_active {
                dali2rust_contracts::msg::APPLICATION_ACTIVE_UNMOVED
            } else {
                dali2rust_contracts::msg::APPLICATION_ACTIVE_MOVED_BY_COMMAND
            };
            publish_dali_settings_changed(publisher, corr, target, applied, moved_by);
        },
    );
}
