use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_bsp::std_thread_stack;
use dali2rust_bsp::unix_clock::unix_wall_clock_millis;
use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, PublishResult};
use dali2rust_contracts::bus::{
    build_confirmation_envelope, build_confirmation_envelope_violation,
    build_confirmation_envelope_with_product_error,
};
use dali2rust_contracts::msg::DeliveryStatus;
use dali2rust_contracts::msg::{
    BusCommandPayload, CommandEnvelope, ConfirmationEnvelope, DaliProgramGroupMembershipCommand,
    DaliProgramTarget, DaliRecallLastActiveLevelCommand, DaliTargetScope, DiscoveryMode,
    ErrorCode, EventEnvelope, HclTargetScope, LightSetpoint, OperationWorkerSignal, Origin,
    RuntimeSource,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::dali::commands::dali_command_from_wire;
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::ses::command_priority;
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_domain::registry::RegistryReadPort;
use dali2rust_platform::dali::{WirePriority, YieldGranularity};

use crate::runtime::config::DaliRuntimeConfig;
use dali2rust_domain::dali::devices::dt8_color::gear_features_automatic_activation;

use crate::runtime::required_publish::Kind as PublishKind;
use crate::runtime::executor::arbitration;
use crate::runtime::executor::bus_health::probe_bus_health;
use crate::runtime::executor::{
    apply_broadcast_target_state, apply_group_target_state, apply_short_target_state,
    apply_with_sequence_retry, AssertRgbwafControl, ColorWritePolicy, RepairAutoActivation,
    commission_next_unaddressed_with_retry_budget, detect_device,
    change_short_address, discover_known_control_gear_with_retry_budget, identify_device,
    program_group_membership, replace_device, run_commissioning_step,
    read_attributes,
    read_memory_bank, group_address, send_standard, write_short_attributes, DiscoveredDevice,
    SemanticDaliError,
};
use dali2rust_bsp::esp_thread;

mod attribute_read;
mod blocked;
mod commissioning;
mod counters;
mod dev103;

pub mod dev103_patch_mask {
    pub use super::dev103::{
        ALL_PATCH_BITS, PATCH_EVENT_FILTER, PATCH_EVENT_PRIORITY, PATCH_EVENT_SCHEME,
        PATCH_INSTANCE_ENABLED,
        PATCH_INSTANCE_GROUP_0, PATCH_INSTANCE_GROUP_1, PATCH_INSTANCE_GROUP_2,
        PATCH_TIMER_DOUBLE, PATCH_TIMER_REPEAT, PATCH_TIMER_SHORT, PATCH_TIMER_STUCK,
        TIMER_PATCH_BITS,
    };
}
mod discovery;
mod membership;
mod memory_bank;
mod program_outcome;
mod publish;
mod scene;
mod scheduling;
mod stop_fade;
mod target_state;
mod wire;
mod write_attributes;

use attribute_read::handle_read_attributes;
use blocked::{adapter_command_blocked, controller_passive_blocked};
use dev103::{
    handle_103_commission, handle_103_identify, handle_103_instance_configure, handle_103_scan,
};
use commissioning::{
    handle_addressing, handle_commissioning_step, handle_identify_device,
    handle_replace_device,
};
pub use counters::DaliWorkerCounters;
use discovery::handle_discover_devices;
use membership::handle_program_group_membership;
use memory_bank::{handle_read_memory_bank, publish_memory_bank_preset};
use program_outcome::{ProgramOutcomeSink, resolve_membership_short};
pub use publish::DALI_WORKER_REQUIRED_EVENTS;
use publish::{
    emit_execution_failed, emit_execution_failed_with_product_error,
    publish_confirmation_ok, publish_confirmation_typed, publish_event_required,
    publish_event_typed, publish_operation_worker_signal, publish_worker_failed,
    publish_worker_started, publish_worker_succeeded,
};
use scene::{
    handle_program_scene, handle_recall_last_active_level, handle_recall_scene,
    scene_levels_fixed,
};
use scheduling::run_command_loop;
use stop_fade::handle_stop_fade;
use target_state::handle_set_target_state;
use wire::process_wire_payload;
use write_attributes::handle_write_attributes;

pub fn spawn_dali_worker<C: DaliApplicationController + Send + 'static>(
    cmd_rx: BusSubscriberRx,
    controller: C,
    runtime_config: DaliRuntimeConfig,
    read_port: Arc<dyn RegistryReadPort>,
    publisher: BusPublisher,
    adapter_id: BusId,
    counters: Arc<DaliWorkerCounters>,
    interactive: Arc<dali2rust_platform::dali::WireActivity>,
    correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
) -> std::thread::JoinHandle<()> {
    esp_thread::spawn_named_stack(
        c"dali_worker",
        std_thread_stack::COMMAND_WORKER_STACK,
        move || {
            run_command_loop(
                &cmd_rx,
                controller,
                runtime_config,
                read_port.as_ref(),
                &publisher,
                &interactive,
                adapter_id,
                &counters,
                correlation.as_ref(),
            );
        },
    )
}

fn refused_by_gates(
    ce: &dali2rust_contracts::msg::CommandEnvelope,
    read_port: &dyn RegistryReadPort,
    adapter_id: BusId,
    correlation_id: u64,
    publisher: &BusPublisher,
    counters: &DaliWorkerCounters,
) -> bool {
    controller_passive_blocked(ce, read_port, adapter_id, correlation_id, publisher, counters)
        || adapter_command_blocked(ce, read_port, adapter_id, correlation_id, publisher, counters)
}

fn process_one(
    frame: BusFrame,
    controller: &mut impl DaliApplicationController,
    runtime_config: DaliRuntimeConfig,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    interactive: &dali2rust_platform::dali::WireActivity,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
    correlation: &dali2rust_bus::CorrelationIdAllocator,
) {
    let BusFrame::Command(ce_arc) = frame else {
        return;
    };
    let ce = ce_arc.as_ref();
    let meta = &ce.meta;
    if BusId(meta.target_adapter_id) != adapter_id {
        return;
    }
    let correlation_id = meta.correlation_id;
    let origin = meta.origin;
    if crate::runtime::priority::wire_priority(ce.payload.variant_name(), origin)
        .is_some_and(|priority| priority != WirePriority::Unattended)
    {
        interactive.note_activity();
    }
    if refused_by_gates(ce, read_port, adapter_id, correlation_id, publisher, counters) {
        return;
    }

    counters.commands_handled.fetch_add(1, Ordering::Relaxed);
    dispatch_dali_command(
        &ce.payload,
        controller,
        runtime_config,
        read_port,
        publisher,
        adapter_id,
        correlation_id,
        origin,
        counters,
        correlation,
    );
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const DALI_WORKER_HANDLED_COMMANDS;
    fn dispatch_dali_command(
        payload: &BusCommandPayload,
        controller: &mut impl DaliApplicationController,
        runtime_config: DaliRuntimeConfig,
        read_port: &dyn RegistryReadPort,
        publisher: &BusPublisher,
        adapter_id: BusId,
        correlation_id: u64,
        origin: Origin,
        counters: &DaliWorkerCounters,
        correlation: &dali2rust_bus::CorrelationIdAllocator,
    );
    payload = payload;
    ignored = { counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    DaliCommandPayload(pl) => process_wire_payload(
        controller,
        publisher,
        correlation_id,
        pl.wire_address,
        pl.command,
        pl.repeat_count,
        pl.raw_mode,
        pl.raw_expects_backward,
        counters,
    ),
    DaliDiscoverDevicesCommand(dc) => handle_discover_devices(
        controller,
        runtime_config,
        read_port,
        publisher,
        adapter_id,
        correlation_id,
        dc,
        counters,
        correlation,
    ),
    DaliReadAttributesCommand(ar) => handle_read_attributes(
        controller,
        runtime_config,
        publisher,
        adapter_id,
        correlation_id,
        origin,
        ar,
        counters,
    ),
    DaliReadMemoryBankCommand(mb) => handle_read_memory_bank(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        mb,
        counters,
    ),
    DaliProgramGroupMembershipCommand(command) => handle_program_group_membership(
        controller,
        read_port,
        publisher,
        adapter_id,
        correlation_id,
        command,
        counters,
    ),
    DaliProgramSceneCommand(command) => handle_program_scene(
        controller,
        read_port,
        publisher,
        adapter_id,
        correlation_id,
        command,
        counters,
    ),
    DaliRecallSceneCommand(command) => handle_recall_scene(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        command,
        counters,
    ),
    DaliRecallLastActiveLevelCommand(command) => handle_recall_last_active_level(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        origin,
        command,
        counters,
    ),
    DaliStopFadeCommand(command) => handle_stop_fade(
        controller,
        read_port,
        publisher,
        correlation_id,
        command,
        counters,
    ),
    DaliSetTargetStateCommand(ts) => handle_set_target_state(
        controller,
        runtime_config,
        read_port,
        publisher,
        adapter_id,
        correlation_id,
        origin,
        ts,
        counters,
    ),
    DaliWriteAttributesCommand(w) => handle_write_attributes(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        w,
        counters,
    ),
    DaliReplaceDeviceCommand(rp) => handle_replace_device(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        rp,
        counters,
    ),
    DaliCommissioningStepCommand(st) => handle_commissioning_step(
        controller,
        publisher,
        correlation_id,
        st,
        counters,
    ),
    DaliAddressingCommand(ac) => handle_addressing(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        ac,
        counters,
    ),
    DaliIdentifyDeviceCommand(id) => handle_identify_device(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        id,
        counters,
    ),
    Dali103HandoverCommand(h) => handle_handover(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        h,
        counters,
    ),
    Dali103ArbitrationProbeCommand(probe) => handle_arbitration_probe(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        probe,
        counters,
    ),
    DaliBusHealthProbeCommand(probe) => handle_bus_health_probe(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        probe,
        counters,
    ),
    Dali103ScanCommand(sc) => handle_103_scan(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        sc,
        counters,
    ),
    Dali103CommissionCommand(cm) => handle_103_commission(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        cm,
        counters,
    ),
    Dali103InstanceConfigureCommand(cfg) => handle_103_instance_configure(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        cfg,
        counters,
    ),
    Dali103FeedbackConfigureCommand(fb) => dev103::handle_103_feedback_configure(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        fb,
        counters,
    ),
    Dali103FeedbackDriveCommand(fd) => dev103::handle_103_feedback_drive(
        controller,
        publisher,
        correlation_id,
        fd,
        counters,
    ),
    Dali103IdentifyCommand(id) => handle_103_identify(
        controller,
        publisher,
        adapter_id,
        correlation_id,
        id,
        counters,
    ),
}

fn handle_bus_health_probe(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::DaliBusHealthProbeCommand,
    counters: &DaliWorkerCounters,
) {
    let Ok(probe) = probe_bus_health(controller) else {
        counters.bus_health_probe_failed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Poller),
        dali2rust_contracts::msg::DaliBusHealthProbedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            control_answered: probe.control_answered,
            lamp_failure: probe.lamp_failure,
        },
    );
    publish_event_typed(publisher, ev);
}

fn handle_handover(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::Dali103HandoverCommand,
    counters: &DaliWorkerCounters,
) {
    if let Err(error) = arbitration::enable_peer_controller(controller, cmd.peer_short_address) {
        counters.execution_failed.fetch_add(1, Ordering::Relaxed);
        emit_execution_failed_with_product_error(
            publisher,
            correlation_id,
            counters,
            error.code(),
            error.message(),
        );
        return;
    }
    let sent = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Poller),
        dali2rust_contracts::msg::Dali103HandoverSentEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            peer_short_address: cmd.peer_short_address,
        },
    );
    publish_event_typed(publisher, sent);
    stand_down_after_handover(publisher, adapter_id, counters);
    publish_confirmation_ok(publisher, correlation_id, counters);
}

fn stand_down_after_handover(
    publisher: &BusPublisher,
    adapter_id: BusId,
    counters: &DaliWorkerCounters,
) {
    let cmd = dali2rust_contracts::msg::DaliSettingsUpdateCommand {
        patch_mask: dali2rust_contracts::msg::DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE,
        dt8_auto_activation_repair: false,
        dt8_rgbwaf_control_assert: false,
        application_active: false,
        device_short_address: u8::MAX,
    };
    let ce = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        dali2rust_contracts::CORRELATION_NONE,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Poller),
        cmd,
    );
    if publisher.try_publish(
        dali2rust_bus::BusChannel::Commands,
        dali2rust_bus::BusFrame::command(ce),
    ) != dali2rust_bus::PublishResult::Queued
    {
        counters
            .handover_stand_down_failed
            .fetch_add(1, Ordering::Relaxed);
    }
}

fn handle_arbitration_probe(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    cmd: &dali2rust_contracts::msg::Dali103ArbitrationProbeCommand,
    counters: &DaliWorkerCounters,
) {
    let Ok(owned) = arbitration::probe_application_controller(controller) else {
        counters
            .arbitration_probe_failed
            .fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Poller),
        dali2rust_contracts::msg::Dali103ArbitrationProbedEvent {
            registry_adapter_id: cmd.registry_adapter_id,
            owned,
        },
    );
    publish_event_typed(publisher, ev);
}

#[cfg(test)]
mod tests;
