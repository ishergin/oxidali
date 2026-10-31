use super::*;
use dali2rust_contracts::msg::{DaliLevelTransitionAppliedEvent, DaliRecallLevelPresetCommand, LevelPreset};

pub(super) fn handle_recall_level_preset(
    controller: &mut impl DaliApplicationController,
    env: &WorkerEnv<'_>,
    correlation_id: u64,
    origin: Origin,
    command: &DaliRecallLevelPresetCommand,
) {
    let WorkerEnv { read_port, publisher, adapter_id, counters, .. } = *env;
    match execute_recall_level_preset(controller, read_port, command) {
        Ok(()) => {
            let applied = level_transition_applied_event(adapter_id, correlation_id, origin, command);
            if publish_closing_fact_or_fail(publisher, applied, counters, PublishKind::Singleton, "level-transition-applied") {
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

// IEC 62386-102 §11.3.5, §11.3.6
fn execute_recall_level_preset(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    command: &DaliRecallLevelPresetCommand,
) -> Result<(), SemanticDaliError> {
    let address = super::target_address::scoped_address(
        read_port,
        command.registry_adapter_id,
        command.scope,
        command.virtual_lamp_id,
        command.short_address,
        command.group_id,
    )?;
    let recall = match command.preset {
        LevelPreset::MaxLevel => StandardCommand::RecallMaxLevel,
        LevelPreset::MinLevel => StandardCommand::RecallMinLevel,
    };
    send_standard(controller, address, recall)?;
    Ok(())
}

fn level_transition_applied_event(
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    command: &DaliRecallLevelPresetCommand,
) -> EventEnvelope {
    let scope = command.scope;
    let applied = DaliLevelTransitionAppliedEvent {
        registry_adapter_id: command.registry_adapter_id,
        scope,
        virtual_lamp_id: (scope == DaliTargetScope::VirtualLamp).then_some(command.virtual_lamp_id),
        short_address: (scope == DaliTargetScope::Short).then_some(command.short_address),
        group_id: (scope == DaliTargetScope::Group).then_some(command.group_id),
        transition: command.preset.transition(),
        source: RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api),
        applied_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
        hold_hcl: command.hold_hcl,
    };
    dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(Origin::Internal),
        applied,
    )
}
