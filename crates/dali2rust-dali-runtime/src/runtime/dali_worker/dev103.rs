use super::*;

use dali2rust_contracts::msg::{
    Dali103CommissionCommand, Dali103IdentifyCommand, Dali103InstanceConfigureCommand,
    Dali103ScanCommand, Dali103ScanProgressEvent, ErrorCode, InstancePatchField,
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
    for field in InstancePatchField::ALL {
        if !cmd.patches(field) {
            continue;
        }
        write_instance_field(controller, cmd, field, proved)?;
        proved.mask |= field.bit();
        controller.step_boundary();
    }
    Ok(())
}

fn write_instance_field(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    field: InstancePatchField,
    proved: &mut ProvedWrites,
) -> Result<(), SemanticDaliError> {
    match field {
        InstancePatchField::InstanceEnabled => write_instance_enabled(controller, cmd, proved),
        InstancePatchField::EventFilter => write_event_filter(controller, cmd),
        InstancePatchField::EventPriority => write_event_priority(controller, cmd),
        InstancePatchField::InstanceGroup0
        | InstancePatchField::InstanceGroup1
        | InstancePatchField::InstanceGroup2 => write_instance_group(controller, cmd, field),
        InstancePatchField::TimerShort
        | InstancePatchField::TimerDouble
        | InstancePatchField::TimerRepeat
        | InstancePatchField::TimerStuck => write_timer(controller, cmd, field),
        InstancePatchField::EventScheme => write_event_scheme(controller, cmd),
    }
}

fn write_instance_enabled(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    proved: &mut ProvedWrites,
) -> Result<(), SemanticDaliError> {
    let (short, instance) = (cmd.short_address, cmd.instance_number);
    let status = set_instance_enabled_verified(controller, short, instance, cmd.instance_enabled)?;
    proved.instance_status = Some(status);
    Ok(())
}

fn write_event_filter(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
) -> Result<(), SemanticDaliError> {
    set_event_filter_verified(controller, cmd.short_address, cmd.instance_number, cmd.event_filter)
}

fn write_event_priority(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
) -> Result<(), SemanticDaliError> {
    let priority = cmd.event_priority;
    set_event_priority_verified(controller, cmd.short_address, cmd.instance_number, priority)
}

fn write_instance_group(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    field: InstancePatchField,
) -> Result<(), SemanticDaliError> {
    let slot = slot_after(InstancePatchField::InstanceGroup0, field);
    let group = cmd.instance_groups[usize::from(slot)];
    set_instance_group_verified(controller, cmd.short_address, cmd.instance_number, slot, group)
}

fn write_timer(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
    field: InstancePatchField,
) -> Result<(), SemanticDaliError> {
    let slot = usize::from(slot_after(InstancePatchField::TimerShort, field));
    let value = cmd.timer_multipliers[slot]
        .ok_or(SemanticDaliError::OperationFailed("invalid_timer"))?;
    set_button_timer_verified(controller, cmd.short_address, cmd.instance_number, slot, value)
}

fn slot_after(first: InstancePatchField, field: InstancePatchField) -> u8 {
    (field as u8).saturating_sub(first as u8)
}

// IEC 62386-103 §9.6.3
fn write_event_scheme(
    controller: &mut impl DaliApplicationController,
    cmd: &Dali103InstanceConfigureCommand,
) -> Result<(), SemanticDaliError> {
    let scheme = EventScheme::from_code(cmd.event_scheme)
        .ok_or(SemanticDaliError::OperationFailed("invalid_event_scheme"))?;
    set_event_scheme_verified(controller, cmd.short_address, cmd.instance_number, scheme)
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
            device_capabilities: device.declarations.capabilities,
            device_status: device.declarations.status,
            version_number: device.declarations.version_number,
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "input-scan-progress");
}

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
        if mask & InstancePatchField::INSTANCE_GROUPS[slot].bit() != 0 {
            instance_groups[slot] = Some(*group);
        }
    }
    let mut body =
        configured_event_base(cmd.registry_adapter_id, cmd.short_address, cmd.instance_number);
    body.instance_status = proved.instance_status;
    body.instance_status_written = proved.instance_status.is_some();
    let proved_field = |field: InstancePatchField| mask & field.bit() != 0;
    body.event_scheme = proved_field(InstancePatchField::EventScheme).then_some(cmd.event_scheme);
    body.event_filter = proved_field(InstancePatchField::EventFilter).then_some(cmd.event_filter);
    body.event_priority =
        proved_field(InstancePatchField::EventPriority).then_some(cmd.event_priority);
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
    for (slot, field) in InstancePatchField::TIMERS.iter().enumerate() {
        if mask & field.bit() != 0 {
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
        instance_type: None,
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
    for (((number, instance_type), probe), facts) in device
        .instances
        .iter()
        .zip(&device.feedback)
        .zip(&device.instance_facts)
    {
        let mut body = configured_event_base(registry_adapter_id, device.short_address, *number);
        body.instance_type = Some(*instance_type);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::clock::StdClock;
    use crate::runtime::controller::DaliController;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
    use dali2rust_domain::dali::dev103::{
        Button301Command, Device103Address, ForwardFrame24, Instance103Command, InstanceAddress,
    };
    use std::sync::Mutex;

    const REFUSED_GROUP: u8 = 40;
    const FIRST_GROUP: u8 = 4;
    const FIRST_TIMER_UNITS: u8 = 10;

    fn command_patching(field: InstancePatchField) -> Dali103InstanceConfigureCommand {
        let mut cmd = Dali103InstanceConfigureCommand {
            registry_adapter_id: 0,
            short_address: 0,
            instance_number: 0,
            patch_mask: 0,
            event_scheme: EventScheme::Device.code(),
            event_filter: [0x01, 0, 0],
            event_priority: 4,
            instance_groups: [Some(REFUSED_GROUP); 3],
            timer_multipliers: [None; 4],
            instance_enabled: false,
        };
        for (slot, member) in (0u8..).zip(InstancePatchField::INSTANCE_GROUPS) {
            if member == field {
                cmd.instance_groups[usize::from(slot)] = Some(FIRST_GROUP + slot);
            }
        }
        for (slot, member) in (0u8..).zip(InstancePatchField::TIMERS) {
            if member == field {
                cmd.timer_multipliers[usize::from(slot)] = Some(FIRST_TIMER_UNITS + slot);
            }
        }
        cmd.patch(field);
        cmd
    }

    const EVERY_ANSWER: u8 = 4;

    fn set_frame(field: InstancePatchField) -> [u8; 3] {
        let (address, instance) = (Device103Address::Short(0), InstanceAddress::Number(0));
        let button = |cmd: Button301Command| ForwardFrame24::command(address, instance, cmd.metadata().opcode);
        let frame = match field {
            InstancePatchField::InstanceEnabled => Instance103Command::DisableInstance.frame(address, instance),
            InstancePatchField::EventFilter => Instance103Command::SetEventFilter.frame(address, instance),
            InstancePatchField::EventPriority => Instance103Command::SetEventPriority.frame(address, instance),
            InstancePatchField::InstanceGroup0 => {
                Instance103Command::SetPrimaryInstanceGroup.frame(address, instance)
            }
            InstancePatchField::InstanceGroup1 => Instance103Command::SetInstanceGroup1.frame(address, instance),
            InstancePatchField::InstanceGroup2 => Instance103Command::SetInstanceGroup2.frame(address, instance),
            InstancePatchField::TimerShort => button(Button301Command::SetShortTimer),
            InstancePatchField::TimerDouble => button(Button301Command::SetDoubleTimer),
            InstancePatchField::TimerRepeat => button(Button301Command::SetRepeatTimer),
            InstancePatchField::TimerStuck => button(Button301Command::SetStuckTimer),
            InstancePatchField::EventScheme => Instance103Command::SetEventScheme.frame(address, instance),
        };
        frame.as_bytes()
    }

    #[test]
    fn a_whole_patch_is_written_in_table_order_with_the_instance_first_and_the_scheme_last() {
        let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
        transport.lock().expect("mock lock").set_persistent_response(EVERY_ANSWER);
        let mut controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        let mut cmd = command_patching(InstancePatchField::EventScheme);
        cmd.patch_mask = Dali103InstanceConfigureCommand::ALL_PATCH_BITS;
        cmd.event_scheme = EVERY_ANSWER;
        cmd.event_filter = [EVERY_ANSWER, 0, 0];
        cmd.event_priority = EVERY_ANSWER;
        cmd.instance_groups = [Some(EVERY_ANSWER); 3];
        cmd.timer_multipliers = [Some(EVERY_ANSWER); 4];
        let mut proved = ProvedWrites::default();
        configure_instance(&mut controller, &cmd, &mut proved).expect("every field lands");
        assert_eq!(proved.mask, Dali103InstanceConfigureCommand::ALL_PATCH_BITS);

        let frames = transport.lock().expect("mock lock").sent_frames24();
        let mut written: Vec<InstancePatchField> = Vec::new();
        for frame in frames {
            let field = InstancePatchField::ALL.into_iter().find(|f| set_frame(*f) == frame);
            if let Some(field) = field.filter(|f| written.last() != Some(f)) {
                written.push(field);
            }
        }
        assert_eq!(written, InstancePatchField::ALL, "IEC 62386-103 §9.6.3: the scheme goes last");
    }

    #[test]
    fn every_field_of_the_patch_table_is_written_to_its_own_slot_and_proved() {
        for field in InstancePatchField::ALL {
            let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
            let mut controller = DaliController::new(transport, Box::new(StdClock::new()));
            let mut proved = ProvedWrites::default();
            configure_instance(&mut controller, &command_patching(field), &mut proved)
                .unwrap_or_else(|error| panic!("{field:?} did not land: {error:?}"));
            assert_eq!(proved.mask, field.bit(), "{field:?} proved something else");
        }
    }
}
