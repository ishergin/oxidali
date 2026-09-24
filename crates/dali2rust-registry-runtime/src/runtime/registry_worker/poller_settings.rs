use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::PollerSettingsUpdateCommand;

use crate::runtime::registry::publish::publish_poller_settings_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::handle_global_settings_update;
use super::RegistryCommandCounters;

pub(super) fn handle_poller_settings_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &PollerSettingsUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.poller_settings_applied,
        || store.apply_poller_settings_from_command(body),
        publish_poller_settings_changed,
    );
}
