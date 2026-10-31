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
    command: &dali2rust_contracts::msg::DaliStopFadeCommand,
) -> Result<(), SemanticDaliError> {
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
    Ok(())
}
