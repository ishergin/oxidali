use super::*;
use dali2rust_contracts::msg::{
    DaliDimCommand, DaliLevelTransitionAppliedEvent, DaliRecallLevelPresetCommand, DimDirection, LevelPreset,
    LevelTransition,
};

pub(super) struct LevelTransitionRequest {
    registry_adapter_id: u8,
    scope: DaliTargetScope,
    virtual_lamp_id: u8,
    short_address: u8,
    group_id: u8,
    hold_hcl: bool,
    command: StandardCommand,
    transition: LevelTransition,
}

// IEC 62386-102 §11.3.7, §11.3.8
impl From<&DaliRecallLevelPresetCommand> for LevelTransitionRequest {
    fn from(command: &DaliRecallLevelPresetCommand) -> Self {
        Self {
            registry_adapter_id: command.registry_adapter_id,
            scope: command.scope,
            virtual_lamp_id: command.virtual_lamp_id,
            short_address: command.short_address,
            group_id: command.group_id,
            hold_hcl: command.hold_hcl,
            command: match command.preset {
                LevelPreset::MaxLevel => StandardCommand::RecallMaxLevel,
                LevelPreset::MinLevel => StandardCommand::RecallMinLevel,
            },
            transition: command.preset.transition(),
        }
    }
}

// IEC 62386-102 §11.3.3, §11.3.4
impl From<&DaliDimCommand> for LevelTransitionRequest {
    fn from(command: &DaliDimCommand) -> Self {
        Self {
            registry_adapter_id: command.registry_adapter_id,
            scope: command.scope,
            virtual_lamp_id: command.virtual_lamp_id,
            short_address: command.short_address,
            group_id: command.group_id,
            hold_hcl: command.hold_hcl,
            command: match command.direction {
                DimDirection::Up => StandardCommand::Up,
                DimDirection::Down => StandardCommand::Down,
            },
            transition: command.direction.transition(),
        }
    }
}

pub(super) fn handle_level_transition(
    controller: &mut impl DaliApplicationController,
    env: &WorkerEnv<'_>,
    correlation_id: u64,
    origin: Origin,
    command: &LevelTransitionRequest,
) {
    let WorkerEnv { read_port, publisher, adapter_id, counters, .. } = *env;
    match execute_level_transition(controller, read_port, command) {
        Ok(address) => {
            let applied = level_transition_applied_event(adapter_id, correlation_id, origin, command, address);
            if publish_closing_fact_or_fail(publisher, applied, counters, PublishKind::Singleton, "level-transition-applied")
                && !command.scope.names_one_lamp()
            {
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

fn execute_level_transition(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    command: &LevelTransitionRequest,
) -> Result<DaliAddress, SemanticDaliError> {
    let address = super::target_address::scoped_address(
        read_port,
        command.registry_adapter_id,
        command.scope,
        command.virtual_lamp_id,
        command.short_address,
        command.group_id,
    )?;
    send_standard(controller, address, command.command)?;
    Ok(address)
}

fn level_transition_applied_event(
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    command: &LevelTransitionRequest,
    address: DaliAddress,
) -> EventEnvelope {
    let scope = command.scope;
    let applied = DaliLevelTransitionAppliedEvent {
        registry_adapter_id: command.registry_adapter_id,
        scope,
        virtual_lamp_id: (scope == DaliTargetScope::VirtualLamp).then_some(command.virtual_lamp_id),
        short_address: match address {
            DaliAddress::Short(short) => Some(short),
            _ => None,
        },
        group_id: (scope == DaliTargetScope::Group).then_some(command.group_id),
        transition: command.transition,
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
