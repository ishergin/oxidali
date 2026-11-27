use super::*;
use dali2rust_contracts::msg::{DaliStopFadeAppliedEvent, DaliStopFadeCommand};
use dali2rust_domain::dali::device::ARC_POWER_LEVEL_MASK;

pub(super) fn handle_stop_fade(
    controller: &mut impl DaliApplicationController,
    env: &WorkerEnv<'_>,
    correlation_id: u64,
    origin: Origin,
    command: &DaliStopFadeCommand,
) {
    let WorkerEnv { read_port, publisher, adapter_id, counters, .. } = *env;
    match execute_stop_fade(controller, read_port, command) {
        Ok(address) => {
            let applied = stop_fade_applied_event(adapter_id, correlation_id, origin, command, address);
            if publish_closing_fact_or_fail(publisher, applied, counters, PublishKind::Singleton, "stop-fade-applied") {
                publish_confirmation_ok(publisher, correlation_id, counters);
            }
        }
        Err(error) => {
            count_failure(counters, error);
            emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                error.code(),
                error.message(),
            );
        }
    }
}

// IEC 62386-102 §9.5.9
fn execute_stop_fade(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    command: &DaliStopFadeCommand,
) -> Result<DaliAddress, SemanticDaliError> {
    let address = super::target_address::scoped_address(
        read_port,
        command.registry_adapter_id,
        command.scope,
        command.virtual_lamp_id,
        command.short_address,
        command.group_id,
    )?;
    send_standard(
        controller,
        address,
        StandardCommand::DirectArcPower {
            level: ARC_POWER_LEVEL_MASK,
        },
    )?;
    Ok(address)
}

fn stop_fade_applied_event(
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    command: &DaliStopFadeCommand,
    address: DaliAddress,
) -> EventEnvelope {
    let scope = command.scope;
    let applied = DaliStopFadeAppliedEvent {
        registry_adapter_id: command.registry_adapter_id,
        scope,
        virtual_lamp_id: (scope == DaliTargetScope::VirtualLamp).then_some(command.virtual_lamp_id),
        short_address: match address {
            DaliAddress::Short(short) => Some(short),
            _ => None,
        },
        group_id: (scope == DaliTargetScope::Group).then_some(command.group_id),
        source: RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api),
        applied_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
    };
    dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(Origin::Internal),
        applied,
    )
}
