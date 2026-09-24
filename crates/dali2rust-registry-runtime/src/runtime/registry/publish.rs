use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusPublisher};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    fixed_text_64, AdapterSettingsChangedEvent, GroupChangedEvent, GroupMatrixChangedEvent,
    LightSetpoint, Origin, PhysicalDeviceChangedEvent, RuntimeStateChangedEvent,
    VirtualLampChangedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

pub(crate) fn publish_registry_event<P>(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    payload: P,
) where
    dali2rust_contracts::msg::BusEventPayload: From<P>,
{
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        target_adapter_id,
        Some(Origin::Registry),
        payload,
    );
    publish_or_drop(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        "registry-events",
    );
}

pub(crate) fn publish_input_device_changed(
    publisher: &dali2rust_bus::BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    short_address: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        dali2rust_contracts::msg::InputDeviceChangedEvent {
            adapter_id,
            short_address,
        },
    );
}

pub(crate) fn publish_physical_device_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    short_address: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        PhysicalDeviceChangedEvent {
            adapter_id,
            short_address,
        },
    );
}

pub(crate) fn publish_virtual_lamp_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    virtual_lamp_id: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        VirtualLampChangedEvent {
            adapter_id,
            virtual_lamp_id,
        },
    );
}

pub(crate) fn publish_group_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    group_id: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        GroupChangedEvent {
            adapter_id,
            group_id,
        },
    );
}

pub(crate) fn publish_group_matrix_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        GroupMatrixChangedEvent { adapter_id },
    );
}

pub const REGISTRY_REQUIRED_EVENTS: &[&str] = &["OperationWorkerSignalEvent"];

pub(crate) fn publish_config_write_signal(
    publisher: &BusPublisher,
    workflow_correlation_id: u64,
    bus_id: dali2rust_bus::BusId,
    error: Option<(dali2rust_contracts::msg::ErrorCode, &str)>,
    counters: &crate::runtime::registry_worker::RegistryCommandCounters,
) {
    let signal = match error {
        None => dali2rust_contracts::msg::OperationWorkerSignalEvent::succeeded(
            workflow_correlation_id,
        ),
        Some((code, message)) => dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
            workflow_correlation_id,
            code,
            message,
        ),
    };
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        workflow_correlation_id,
        bus_id.0,
        Some(Origin::Registry),
        signal,
    );
    dali2rust_bus::publish_required_counted(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "registry-config-write-signal",
        dali2rust_bus::RequiredPublishCounters::new(
            &counters.config_write_signal_publish_retried,
            &counters.config_write_signal_publish_failed,
        ),
    );
}

pub(crate) fn publish_scene_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    scene_id: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        dali2rust_contracts::msg::SceneChangedEvent {
            adapter_id,
            scene_id,
        },
    );
}

pub(crate) fn publish_scene_matrix_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    scene_id: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        u16::from(adapter_id),
        dali2rust_contracts::msg::SceneMatrixChangedEvent {
            adapter_id,
            scene_id,
        },
    );
}

pub(crate) fn publish_hcl_schedule_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    schedule_id: dali2rust_contracts::msg::FixedText32,
    removed: bool,
    enabled: bool,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::HclScheduleChangedEvent {
            schedule_id,
            removed,
            enabled,
        },
    );
}

pub(crate) fn publish_home_assistant_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    enabled: bool,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::HomeAssistantSettingsChangedEvent { enabled },
    );
}

pub(crate) fn publish_dali_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::dali_settings::DaliSettingsRecord,
    application_active_moved_by: u8,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::DaliSettingsChangedEvent {
            dt8_auto_activation_repair: applied.dt8_auto_activation_repair,
            dt8_rgbwaf_control_assert: applied.dt8_rgbwaf_control_assert,
            application_active: applied.application_active,
            application_active_moved_by,
        },
    );
}

pub(crate) fn publish_redundancy_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::redundancy_settings::RedundancySettingsRecord,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::RedundancySettingsChangedEvent {
            enabled: applied.enabled,
            standby_role: applied.standby_role,
            probe_interval_ms: applied.probe_interval_ms,
            takeover_after_missed: applied.takeover_after_missed,
            boot_listen_ms: applied.boot_listen_ms,
            peer_device_short_address: applied.peer_device_short_address.unwrap_or(u8::MAX),
            peer_url: dali2rust_contracts::msg::fixed_text_64(&applied.peer_url),
        },
    );
}

pub(crate) fn publish_policies_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::policies::PoliciesRecord,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::PoliciesChangedEvent {
            system_failure_level: applied
                .system_failure_level
                .unwrap_or(dali2rust_contracts::msg::PoliciesUpdateCommand::UNMANAGED),
            power_on_level: applied
                .power_on_level
                .unwrap_or(dali2rust_contracts::msg::PoliciesUpdateCommand::UNMANAGED),
            apply_on_discovery: applied.apply_on_discovery,
        },
    );
}

pub(crate) fn publish_poller_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::poller_settings::PollerSettingsRecord,
) {
    publish_registry_event(
        publisher,
        correlation_id,
        target_adapter_id,
        dali2rust_contracts::msg::PollerSettingsChangedEvent {
            enabled: applied.enabled,
            interval_ms: applied.interval_ms,
            attribute_groups_mask: applied.attribute_groups_mask,
            include_dt8_color: applied.include_dt8_color,
            include_energy: applied.include_energy,
            include_diagnostics: applied.include_diagnostics,
            skip_unbound_virtual_lamps: applied.skip_unbound_virtual_lamps,
        },
    );
}

pub(crate) fn publish_adapter_settings_changed(
    publisher: &BusPublisher,
    adapter_id: u8,
    correlation_id: u64,
    name: &str,
    enabled: bool,
) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        u16::from(adapter_id),
        Some(Origin::Internal),
        AdapterSettingsChangedEvent {
            adapter_id,
            name: fixed_text_64(name),
            enabled,
        },
    );
    publish_or_drop(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        "registry-events",
    );
}

#[allow(clippy::too_many_arguments, reason = "one event, one call site; a struct here would only rename the fields")]
pub(crate) fn publish_runtime_state_changed(
    publisher: &BusPublisher,
    runtime_events_published: &AtomicU32,
    correlation_id: u64,
    adapter_id: u8,
    virtual_lamp_id: Option<u8>,
    short_address: Option<u8>,
    setpoint: &LightSetpoint,
    observation: &dali2rust_contracts::msg::RuntimeObservation,
    commit_source: dali2rust_contracts::msg::RuntimeSource,
    commit_dimensions: dali2rust_contracts::msg::SetpointDimensions,
) {
    let ev = event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        u16::from(adapter_id),
        Some(Origin::Internal),
        RuntimeStateChangedEvent {
            adapter_id,
            virtual_lamp_id,
            short_address,
            state_setpoint: setpoint.clone(),
            state_observation: observation.clone(),
            commit_source,
            commit_dimensions,
        },
    );
    publish_or_drop(
        publisher,
        BusChannel::Events,
        BusFrame::event(ev),
        "registry-events",
    );
    runtime_events_published.fetch_add(1, Ordering::Relaxed);
}
