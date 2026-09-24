use super::*;

use dali2rust_contracts::msg::{
    Dali103CommissionCommand, Dali103IdentifyCommand, Dali103InstanceConfigureCommand,
    Dali103ScanCommand, Dali103ScanProgressEvent, ErrorCode,
};
use dali2rust_domain::dali::dev103::EventScheme;

use crate::runtime::executor::dev103::{
    set_instance_enabled_verified,
    commission_control_devices, identify_device, scan_control_devices, set_button_timer_verified,
    set_event_filter_verified, set_event_priority_verified, set_event_scheme_verified,
    set_instance_group_verified, ScannedDevice,
};
use crate::runtime::executor::dev103_feedback::{
    configure_feedback, drive_feedback, ConfiguredFeedback,
};

pub const PATCH_EVENT_SCHEME: u16 = 1 << 0;
pub const PATCH_EVENT_FILTER: u16 = 1 << 1;
pub const PATCH_EVENT_PRIORITY: u16 = 1 << 2;
pub const PATCH_INSTANCE_GROUP_0: u16 = 1 << 3;
pub const PATCH_INSTANCE_GROUP_1: u16 = 1 << 4;
pub const PATCH_INSTANCE_GROUP_2: u16 = 1 << 5;
pub const PATCH_TIMER_SHORT: u16 = 1 << 6;
pub const PATCH_TIMER_DOUBLE: u16 = 1 << 7;
pub const PATCH_TIMER_REPEAT: u16 = 1 << 8;
pub const PATCH_TIMER_STUCK: u16 = 1 << 9;
pub const PATCH_INSTANCE_ENABLED: u16 = 1 << 10;

pub const ALL_PATCH_BITS: u16 = PATCH_EVENT_SCHEME
    | PATCH_EVENT_FILTER
    | PATCH_EVENT_PRIORITY
    | PATCH_INSTANCE_GROUP_0
    | PATCH_INSTANCE_GROUP_1
    | PATCH_INSTANCE_GROUP_2
    | PATCH_TIMER_SHORT
    | PATCH_TIMER_DOUBLE
    | PATCH_TIMER_REPEAT
    | PATCH_TIMER_STUCK
    | PATCH_INSTANCE_ENABLED;

pub const TIMER_PATCH_BITS: [u16; 4] = [
    PATCH_TIMER_SHORT,
    PATCH_TIMER_DOUBLE,
    PATCH_TIMER_REPEAT,
    PATCH_TIMER_STUCK,
];

pub(super) fn handle_103_scan(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &Dali103ScanCommand,
    counters: &DaliWorkerCounters,
) {
    publish_scan_started(publisher, adapter_id, correlation_id, cmd.registry_adapter_id, counters);
    let registry_adapter_id = cmd.registry_adapter_id;
    let mut on_device = |device: &ScannedDevice| {
        publish_scan_progress(
            publisher,
            adapter_id,
            correlation_id,
            registry_adapter_id,
            device,
            counters,
        );
        publish_scan_feedback(
            publisher,
            adapter_id,
            correlation_id,
            registry_adapter_id,
            device,
            counters,
        );
    };
    match scan_control_devices(controller, &mut on_device) {
        Ok(summary) => {
            counters.input_scans_completed.fetch_add(1, Ordering::Relaxed);
            publish_worker_succeeded(publisher, adapter_id, correlation_id, counters);
            counters
                .input_addresses_contended
                .fetch_add(u32::from(summary.addresses_contended), Ordering::Relaxed);
            counters
                .input_presence_reprobed
                .fetch_add(u32::from(summary.addresses_reprobed), Ordering::Relaxed);
        }
        Err(error) => fail_operation(publisher, adapter_id, correlation_id, error, counters),
    }
}

pub(super) fn handle_103_commission(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &Dali103CommissionCommand,
    counters: &DaliWorkerCounters,
) {
    let mut on_addressed = |_short: u8, _random: u32| {
        counters.input_devices_addressed.fetch_add(1, Ordering::Relaxed);
    };
    match commission_control_devices(controller, cmd.include_addressed, &mut on_addressed) {
        Ok(_) => publish_worker_succeeded(publisher, adapter_id, correlation_id, counters),
        Err(error) => fail_operation(publisher, adapter_id, correlation_id, error, counters),
    }
}

pub(super) fn handle_103_identify(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &Dali103IdentifyCommand,
    counters: &DaliWorkerCounters,
) {
    match identify_device(controller, cmd.short_address) {
        Ok(()) => publish_worker_succeeded(publisher, adapter_id, correlation_id, counters),
        Err(error) => fail_operation(publisher, adapter_id, correlation_id, error, counters),
    }
}

pub(super) fn handle_103_instance_configure(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &Dali103InstanceConfigureCommand,
    counters: &DaliWorkerCounters,
) {
    let mut proved = ProvedWrites::default();
    let outcome = configure_instance(controller, cmd, &mut proved);
    if outcome.is_ok() || proved.mask != 0 {
        publish_instance_configured(publisher, adapter_id, correlation_id, cmd, &proved, counters);
    }
    if let Err(error) = outcome {
        counters.input_config_rejected.fetch_add(1, Ordering::Relaxed);
        fail_operation(publisher, adapter_id, correlation_id, error, counters);
        return;
    }
    publish_worker_succeeded(publisher, adapter_id, correlation_id, counters);
}

#[derive(Default)]
struct ProvedWrites {
    mask: u16,
    instance_status: Option<u8>,
}

fn configure_instance(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    proved: &mut ProvedWrites,
) -> Result<(), SemanticDaliError> {
    let (short, instance) = (cmd.short_address, cmd.instance_number);
    if cmd.patch_mask & PATCH_INSTANCE_ENABLED != 0 {
        proved.instance_status = Some(set_instance_enabled_verified(
            controller,
            short,
            instance,
            cmd.instance_enabled,
        )?);
        proved.mask |= PATCH_INSTANCE_ENABLED;
        controller.step_boundary();
    }
    configure_event_fields(controller, cmd, proved)?;
    configure_timers(controller, cmd, proved)?;
    // IEC 62386-103 §9.6.3
    if cmd.patch_mask & PATCH_EVENT_SCHEME != 0 {
        let scheme =
            EventScheme::from_code(cmd.event_scheme).ok_or(SemanticDaliError::OperationFailed(
                "invalid_event_scheme",
            ))?;
        set_event_scheme_verified(controller, short, instance, scheme)?;
        proved.mask |= PATCH_EVENT_SCHEME;
        controller.step_boundary();
    }
    Ok(())
}

fn configure_event_fields(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    proved: &mut ProvedWrites,
) -> Result<(), SemanticDaliError> {
    let (short, instance) = (cmd.short_address, cmd.instance_number);
    if cmd.patch_mask & PATCH_EVENT_FILTER != 0 {
        set_event_filter_verified(controller, short, instance, cmd.event_filter)?;
        proved.mask |= PATCH_EVENT_FILTER;
        controller.step_boundary();
    }
    if cmd.patch_mask & PATCH_EVENT_PRIORITY != 0 {
        set_event_priority_verified(controller, short, instance, cmd.event_priority)?;
        proved.mask |= PATCH_EVENT_PRIORITY;
        controller.step_boundary();
    }
    for (bit, slot) in [
        (PATCH_INSTANCE_GROUP_0, 0u8),
        (PATCH_INSTANCE_GROUP_1, 1),
        (PATCH_INSTANCE_GROUP_2, 2),
    ] {
        if cmd.patch_mask & bit != 0 {
            let group = cmd.instance_groups[slot as usize];
            set_instance_group_verified(controller, short, instance, slot, group)?;
            proved.mask |= bit;
            controller.step_boundary();
        }
    }
    Ok(())
}

fn configure_timers(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    proved: &mut ProvedWrites,
) -> Result<(), SemanticDaliError> {
    for (slot, bit) in TIMER_PATCH_BITS.iter().enumerate() {
        if cmd.patch_mask & bit == 0 {
            continue;
        }
        let value = cmd.timer_multipliers[slot]
            .ok_or(SemanticDaliError::OperationFailed("invalid_timer"))?;
        set_button_timer_verified(
            controller,
            cmd.short_address,
            cmd.instance_number,
            slot,
            value,
        )?;
        proved.mask |= bit;
        controller.step_boundary();
    }
    Ok(())
}

fn publish_scan_started(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    registry_adapter_id: u8,
    counters: &DaliWorkerCounters,
) {
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        dali2rust_contracts::msg::Dali103ScanStartedEvent {
            registry_adapter_id,
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "input-scan-started");
}

fn publish_scan_progress(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    registry_adapter_id: u8,
    device: &ScannedDevice,
    counters: &DaliWorkerCounters,
) {
    let mut instance_types = [None; MAX_REPORTED_INSTANCES];
    for (slot, (_, instance_type)) in device
        .instances
        .iter()
        .take(MAX_REPORTED_INSTANCES)
        .enumerate()
    {
        instance_types[slot] = Some(*instance_type);
    }
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        Dali103ScanProgressEvent {
            registry_adapter_id,
            short_address: device.short_address,
            presence_unproven: device.presence_unproven,
            instance_count: device.instance_count,
            instance_types,
            device_capabilities: device.declarations.capabilities,
            device_status: device.declarations.status,
            version_number: device.declarations.version_number,
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "input-scan-progress");
}

pub const MAX_REPORTED_INSTANCES: usize = 8;

fn fail_operation(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    error: SemanticDaliError,
    counters: &DaliWorkerCounters,
) {
    publish_worker_failed(
        publisher,
        adapter_id,
        correlation_id,
        error.code(),
        error.message(),
        counters,
    );
}

fn publish_instance_configured(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &Dali103InstanceConfigureCommand,
    proved: &ProvedWrites,
    counters: &DaliWorkerCounters,
) {
    let mask = proved.mask;
    let mut instance_groups = [None; 3];
    for (slot, group) in cmd.instance_groups.iter().enumerate() {
        let bit = PATCH_INSTANCE_GROUP_0 << slot;
        if mask & bit != 0 {
            instance_groups[slot] = Some(*group);
        }
    }
    let mut body =
        configured_event_base(cmd.registry_adapter_id, cmd.short_address, cmd.instance_number);
    body.instance_status = proved.instance_status;
    body.instance_status_written = proved.instance_status.is_some();
    body.event_scheme = (mask & PATCH_EVENT_SCHEME != 0).then_some(cmd.event_scheme);
    body.event_filter = (mask & PATCH_EVENT_FILTER != 0).then_some(cmd.event_filter);
    body.event_priority = (mask & PATCH_EVENT_PRIORITY != 0).then_some(cmd.event_priority);
    body.instance_groups = instance_groups;
    body.timers = configured_timers(cmd, mask);
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        body,
    );
    publish_event_required(
        publisher,
        ev,
        counters,
        PublishKind::Series,
        "input-instance-configured",
    );
}

fn configured_timers(cmd: &Dali103InstanceConfigureCommand, mask: u16) -> [Option<u8>; 4] {
    let mut timers = [None; 4];
    for (slot, bit) in TIMER_PATCH_BITS.iter().enumerate() {
        if mask & bit != 0 {
            timers[slot] = cmd.timer_multipliers[slot];
        }
    }
    timers
}

fn publish_worker_failed(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    code: ErrorCode,
    message: &str,
    counters: &DaliWorkerCounters,
) {
    super::publish::publish_worker_failed(
        publisher,
        adapter_id,
        correlation_id,
        dali2rust_contracts::msg::Origin::Api,
        code,
        message,
        counters,
    );
}

pub(super) fn handle_103_feedback_configure(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
    counters: &DaliWorkerCounters,
) {
    let mut proved = ConfiguredFeedback::default();
    let outcome = configure_feedback(controller, cmd, &mut proved);
    if outcome.is_ok() || proved.proved_any() {
        publish_feedback_configured(publisher, adapter_id, correlation_id, cmd, &proved, counters);
    }
    if let Err(error) = outcome {
        counters.input_config_rejected.fetch_add(1, Ordering::Relaxed);
        fail_operation(publisher, adapter_id, correlation_id, error, counters);
        return;
    }
    publish_worker_succeeded(publisher, adapter_id, correlation_id, counters);
}

fn configured_event_base(
    registry_adapter_id: u8,
    short_address: u8,
    instance_number: u8,
) -> dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
    dali2rust_contracts::msg::Dali103InstanceConfiguredEvent {
        registry_adapter_id,
        short_address,
        instance_number,
        event_scheme: None,
        event_filter: None,
        event_priority: None,
        instance_groups: [None; 3],
        timers: [None; 4],
        manual_config_active: None,
        feedback_opcode_map: None,
        feedback_capability: None,
        feedback_colour_capability: None,
        feedback_timing: None,
        feedback_active_brightness: None,
        feedback_active_colour: None,
        feedback_inactive_brightness: None,
        feedback_inactive_colour: None,
        instance_status: None,
        resolution: None,
        instance_status_written: false,
    }
}

fn publish_scan_feedback(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    registry_adapter_id: u8,
    device: &ScannedDevice,
    counters: &DaliWorkerCounters,
) {
    for (((number, _), probe), facts) in device
        .instances
        .iter()
        .zip(&device.feedback)
        .zip(&device.instance_facts)
    {
        let mut body = configured_event_base(registry_adapter_id, device.short_address, *number);
        body.feedback_opcode_map = Some(probe.map_code);
        body.feedback_capability = probe.capability;
        body.feedback_colour_capability = probe.colour_capability;
        body.instance_status = facts.status;
        body.resolution = facts.resolution;
        let ev = dali2rust_contracts::bus::event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            adapter_id.0,
            Some(dali2rust_contracts::msg::Origin::Api),
            body,
        );
        publish_event_required(publisher, ev, counters, PublishKind::Series, "input-scan-feedback");
    }
}

fn publish_feedback_configured(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackConfigureCommand,
    confirmed: &ConfiguredFeedback,
    counters: &DaliWorkerCounters,
) {
    let mut body =
        configured_event_base(cmd.registry_adapter_id, cmd.short_address, cmd.instance_number);
    body.feedback_opcode_map = Some(confirmed.map_code);
    body.feedback_timing = confirmed.timing;
    body.feedback_active_brightness = confirmed.active_brightness;
    body.feedback_active_colour = confirmed.active_colour;
    body.feedback_inactive_brightness = confirmed.inactive_brightness;
    body.feedback_inactive_colour = confirmed.inactive_colour;
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Api),
        body,
    );
    publish_event_required(
        publisher,
        ev,
        counters,
        PublishKind::Series,
        "input-feedback-configured",
    );
}

pub(super) fn handle_103_feedback_drive(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::Dali103FeedbackDriveCommand,
    counters: &DaliWorkerCounters,
) {
    match drive_feedback(controller, cmd) {
        Ok(()) => super::publish::publish_confirmation_ok(publisher, correlation_id, counters),
        Err(error) => {
            counters.execution_failed.fetch_add(1, Ordering::Relaxed);
            super::publish::emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                error.code(),
                error.message(),
            );
        }
    }
}

fn publish_worker_succeeded(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    counters: &DaliWorkerCounters,
) {
    super::publish::publish_worker_succeeded(
        publisher,
        adapter_id,
        correlation_id,
        dali2rust_contracts::msg::Origin::Api,
        counters,
    );
}
