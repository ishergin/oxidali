use super::*;
use dali2rust_domain::dali::device::ARC_POWER_LEVEL_MASK;

pub(super) fn handle_stop_fade(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    correlation_id: u64,
    command: &dali2rust_contracts::msg::DaliStopFadeCommand,
    counters: &DaliWorkerCounters,
) {
    match execute_stop_fade(controller, read_port, command) {
        Ok(()) => publish_confirmation_ok(publisher, correlation_id, counters),
        Err(error) => {
            counters.execution_failed.fetch_add(1, Ordering::Relaxed);
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
    command: &dali2rust_contracts::msg::DaliStopFadeCommand,
) -> Result<(), SemanticDaliError> {
    let address = stop_fade_address(read_port, command)?;
    send_standard(
        controller,
        address,
        StandardCommand::DirectArcPower {
            level: ARC_POWER_LEVEL_MASK,
        },
    )?;
    Ok(())
}

fn stop_fade_address(
    read_port: &dyn RegistryReadPort,
    command: &dali2rust_contracts::msg::DaliStopFadeCommand,
) -> Result<DaliAddress, SemanticDaliError> {
    match command.scope {
        DaliTargetScope::Broadcast => Ok(DaliAddress::Broadcast),
        DaliTargetScope::Group => DaliAddress::group(command.group_id)
            .map_err(|_| SemanticDaliError::Conflict("invalid_group_id")),
        DaliTargetScope::Short => DaliAddress::short(command.short_address)
            .map_err(|_| SemanticDaliError::Conflict("invalid_short_address")),
        DaliTargetScope::VirtualLamp => {
            let short = read_port
                .virtual_lamp_binding_short(command.registry_adapter_id, command.virtual_lamp_id)
                .ok_or(SemanticDaliError::Conflict("vl_unbound"))?;
            DaliAddress::short(short)
                .map_err(|_| SemanticDaliError::Conflict("invalid_short_address"))
        }
        DaliTargetScope::AddressRange => {
            Err(SemanticDaliError::Conflict("unsupported_scope"))
        }
    }
}
