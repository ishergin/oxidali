use dali2rust_bus::BusPublisher;
use dali2rust_contracts::msg::{
    HomeAssistantControllerIdUpdateCommand, HomeAssistantCredentialsUpdateCommand,
    HomeAssistantSettingsUpdateCommand, HomeAssistantTopicsUpdateCommand,
};

use crate::runtime::registry::home_assistant_settings::HomeAssistantSettingsRecord;
use crate::runtime::registry::publish::publish_home_assistant_settings_changed;
use crate::runtime::registry::RegistryStore;

use super::confirm::handle_global_settings_update;
use super::RegistryCommandCounters;

fn publish_changed(
    publisher: &BusPublisher,
    corr: u64,
    tid: u16,
    applied: &HomeAssistantSettingsRecord,
) {
    publish_home_assistant_settings_changed(publisher, corr, tid, applied.enabled);
}

pub(super) fn handle_settings_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HomeAssistantSettingsUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.home_assistant_settings_applied,
        || store.apply_home_assistant_settings_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_credentials_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HomeAssistantCredentialsUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.home_assistant_settings_applied,
        || store.apply_home_assistant_credentials_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_topics_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HomeAssistantTopicsUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.home_assistant_settings_applied,
        || store.apply_home_assistant_topics_from_command(body),
        publish_changed,
    );
}

pub(super) fn handle_controller_id_update(
    publisher: &BusPublisher,
    tid: u16,
    corr: u64,
    primary_adapter_id: dali2rust_bus::BusId,
    store: &RegistryStore,
    counters: &RegistryCommandCounters,
    body: &HomeAssistantControllerIdUpdateCommand,
) {
    handle_global_settings_update(
        publisher,
        tid,
        corr,
        primary_adapter_id,
        counters,
        &counters.home_assistant_settings_applied,
        || store.apply_home_assistant_controller_id_from_command(body),
        publish_changed,
    );
}
