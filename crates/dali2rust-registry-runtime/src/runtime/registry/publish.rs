use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusPublisher, PublishResult};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    fixed_text_64, AdapterSettingsChangedEvent, FixedText32, GroupChangedEvent,
    GroupMatrixChangedEvent, HclScheduleChangedEvent, HomeAssistantSettingsChangedEvent,
    InputDeviceChangedEvent, LightSetpoint, Origin, PhysicalDeviceChangedEvent,
    RuntimeStateChangedEvent, SceneChangedEvent, SceneMatrixChangedEvent,
    VirtualLampChangedEvent,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

use super::home_assistant_settings::HomeAssistantSettingsRecord;

pub(crate) fn registry_event_frame<P>(
    correlation_id: u64,
    target_adapter_id: u16,
    payload: P,
) -> BusFrame
where
    dali2rust_contracts::msg::BusEventPayload: From<P>,
{
    let (source, origin) = (SOURCE_ID_UNSPECIFIED, Some(Origin::Registry));
    BusFrame::event(event_envelope(source, correlation_id, target_adapter_id, origin, payload))
}

pub(crate) fn publish_registry_event<P>(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    payload: P,
) -> PublishResult
where
    dali2rust_contracts::msg::BusEventPayload: From<P>,
{
    let frame = registry_event_frame(correlation_id, target_adapter_id, payload);
    publish_or_drop(publisher, BusChannel::Events, frame, "registry-events")
}

pub(crate) fn input_device_changed(adapter_id: u8, short_address: u8) -> InputDeviceChangedEvent {
    InputDeviceChangedEvent { adapter_id, short_address }
}

pub(crate) fn publish_input_device_changed(
    publisher: &dali2rust_bus::BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    short_address: u8,
) -> PublishResult {
    let payload = input_device_changed(adapter_id, short_address);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn physical_device_changed(
    adapter_id: u8,
    short_address: u8,
) -> PhysicalDeviceChangedEvent {
    PhysicalDeviceChangedEvent { adapter_id, short_address }
}

pub(crate) fn publish_physical_device_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    short_address: u8,
) -> PublishResult {
    let payload = physical_device_changed(adapter_id, short_address);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn virtual_lamp_changed(adapter_id: u8, virtual_lamp_id: u8) -> VirtualLampChangedEvent {
    VirtualLampChangedEvent { adapter_id, virtual_lamp_id }
}

pub(crate) fn publish_virtual_lamp_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    virtual_lamp_id: u8,
) -> PublishResult {
    let payload = virtual_lamp_changed(adapter_id, virtual_lamp_id);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn group_changed(adapter_id: u8, group_id: u8) -> GroupChangedEvent {
    GroupChangedEvent { adapter_id, group_id }
}

pub(crate) fn publish_group_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    group_id: u8,
) -> PublishResult {
    let payload = group_changed(adapter_id, group_id);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn group_matrix_changed(adapter_id: u8) -> GroupMatrixChangedEvent {
    GroupMatrixChangedEvent { adapter_id }
}

pub(crate) fn publish_group_matrix_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
) -> PublishResult {
    let payload = group_matrix_changed(adapter_id);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn publish_unbound_lamps(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    outcome: &super::physical_devices::ForgetOutcome,
) {
    for lamp_id in &outcome.unbound_lamps {
        publish_virtual_lamp_changed(publisher, correlation_id, adapter_id, *lamp_id);
    }
    if outcome.groups_changed {
        publish_group_matrix_changed(publisher, correlation_id, adapter_id);
    }
}

pub const REGISTRY_REQUIRED_EVENTS: &[&str] = &["OperationWorkerSignalEvent", "RegistrySliceReloadedEvent"];

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

pub(crate) fn scene_changed(adapter_id: u8, scene_id: u8) -> SceneChangedEvent {
    SceneChangedEvent { adapter_id, scene_id }
}

pub(crate) fn publish_scene_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    scene_id: u8,
) -> PublishResult {
    let payload = scene_changed(adapter_id, scene_id);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn scene_matrix_changed(adapter_id: u8, scene_id: u8) -> SceneMatrixChangedEvent {
    SceneMatrixChangedEvent { adapter_id, scene_id }
}

pub(crate) fn publish_scene_matrix_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: u8,
    scene_id: u8,
) -> PublishResult {
    let payload = scene_matrix_changed(adapter_id, scene_id);
    publish_registry_event(publisher, correlation_id, u16::from(adapter_id), payload)
}

pub(crate) fn hcl_schedule_changed(
    schedule_id: &FixedText32,
    enabled: Option<bool>,
) -> HclScheduleChangedEvent {
    HclScheduleChangedEvent {
        schedule_id: schedule_id.clone(),
        removed: enabled.is_none(),
        enabled: enabled.unwrap_or(false),
    }
}

pub(crate) fn publish_hcl_schedule_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    schedule_id: &FixedText32,
    enabled: Option<bool>,
) -> PublishResult {
    let payload = hcl_schedule_changed(schedule_id, enabled);
    publish_registry_event(publisher, correlation_id, target_adapter_id, payload)
}

pub(crate) fn home_assistant_settings_changed(
    applied: &HomeAssistantSettingsRecord,
) -> HomeAssistantSettingsChangedEvent {
    HomeAssistantSettingsChangedEvent { enabled: applied.enabled }
}

pub(crate) fn publish_home_assistant_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &HomeAssistantSettingsRecord,
) -> PublishResult {
    let payload = home_assistant_settings_changed(applied);
    publish_registry_event(publisher, correlation_id, target_adapter_id, payload)
}

pub(crate) fn publish_dali_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::dali_settings::DaliSettingsRecord,
    application_active_moved_by: u8,
) -> PublishResult {
    let payload = dali_settings_changed(applied, application_active_moved_by);
    publish_registry_event(publisher, correlation_id, target_adapter_id, payload)
}

pub(crate) fn dali_settings_changed(
    applied: &super::dali_settings::DaliSettingsRecord,
    application_active_moved_by: u8,
) -> dali2rust_contracts::msg::DaliSettingsChangedEvent {
    dali2rust_contracts::msg::DaliSettingsChangedEvent {
        dt8_auto_activation_repair: applied.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: applied.dt8_rgbwaf_control_assert,
        application_active: applied.application_active,
        application_active_moved_by,
    }
}

pub(crate) fn publish_redundancy_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::redundancy_settings::RedundancySettingsRecord,
) -> PublishResult {
    let payload = redundancy_settings_changed(applied);
    publish_registry_event(publisher, correlation_id, target_adapter_id, payload)
}

pub(crate) fn redundancy_settings_changed(
    applied: &super::redundancy_settings::RedundancySettingsRecord,
) -> dali2rust_contracts::msg::RedundancySettingsChangedEvent {
    dali2rust_contracts::msg::RedundancySettingsChangedEvent {
        enabled: applied.enabled,
        standby_role: applied.standby_role,
        probe_interval_ms: applied.probe_interval_ms,
        takeover_after_missed: applied.takeover_after_missed,
        boot_listen_ms: applied.boot_listen_ms,
        peer_device_short_address: applied.peer_device_short_address.unwrap_or(u8::MAX),
        peer_url: dali2rust_contracts::msg::fixed_text_64(&applied.peer_url),
    }
}

pub(crate) fn publish_policies_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::policies::PoliciesRecord,
) -> PublishResult {
    publish_registry_event(publisher, correlation_id, target_adapter_id, policies_changed(applied))
}

pub(crate) fn policies_changed(
    applied: &super::policies::PoliciesRecord,
) -> dali2rust_contracts::msg::PoliciesChangedEvent {
    let unmanaged = dali2rust_contracts::msg::PoliciesUpdateCommand::UNMANAGED;
    dali2rust_contracts::msg::PoliciesChangedEvent {
        system_failure_level: applied.system_failure_level.unwrap_or(unmanaged),
        power_on_level: applied.power_on_level.unwrap_or(unmanaged),
        apply_on_discovery: applied.apply_on_discovery,
    }
}

pub(crate) fn publish_poller_settings_changed(
    publisher: &BusPublisher,
    correlation_id: u64,
    target_adapter_id: u16,
    applied: &super::poller_settings::PollerSettingsRecord,
) -> PublishResult {
    let payload = poller_settings_changed(applied);
    publish_registry_event(publisher, correlation_id, target_adapter_id, payload)
}

pub(crate) fn poller_settings_changed(
    applied: &super::poller_settings::PollerSettingsRecord,
) -> dali2rust_contracts::msg::PollerSettingsChangedEvent {
    dali2rust_contracts::msg::PollerSettingsChangedEvent {
        enabled: applied.enabled,
        interval_ms: applied.interval_ms,
        attribute_groups_mask: applied.attribute_groups_mask,
        include_dt8_color: applied.include_dt8_color,
        include_energy: applied.include_energy,
        include_diagnostics: applied.include_diagnostics,
        skip_unbound_virtual_lamps: applied.skip_unbound_virtual_lamps,
    }
}

pub(crate) fn publish_adapter_settings_changed(
    publisher: &BusPublisher,
    adapter_id: u8,
    correlation_id: u64,
    name: &str,
    enabled: bool,
) -> PublishResult {
    let frame = adapter_settings_frame(adapter_id, correlation_id, name, enabled);
    publish_or_drop(publisher, BusChannel::Events, frame, "registry-events")
}

pub(crate) fn adapter_settings_frame(
    adapter_id: u8,
    correlation_id: u64,
    name: &str,
    enabled: bool,
) -> BusFrame {
    let payload = AdapterSettingsChangedEvent { adapter_id, name: fixed_text_64(name), enabled };
    let (target, origin) = (u16::from(adapter_id), Some(Origin::Internal));
    BusFrame::event(event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, target, origin, payload))
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeCommitMeta {
    pub(crate) source: dali2rust_contracts::msg::RuntimeSource,
    pub(crate) dimensions: dali2rust_contracts::msg::SetpointDimensions,
    pub(crate) holds_hcl: bool,
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
    commit: RuntimeCommitMeta,
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
            commit_source: commit.source,
            commit_dimensions: commit.dimensions,
            commit_holds_hcl: commit.holds_hcl,
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
