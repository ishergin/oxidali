use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::PoliciesUpdateCommand;

use crate::runtime::registry::publish::publish_policies_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::handle_global_settings_update;
use super::RegistryCommandCounters;

pub(super) fn handle_policies_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &PoliciesUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.policies_applied,
        || store.apply_policies_from_command(body),
        publish_policies_changed,
    );
}
