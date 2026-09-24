use super::*;

pub(super) fn handle_program_scene(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    command: &dali2rust_contracts::msg::DaliProgramSceneCommand,
    counters: &DaliWorkerCounters,
) {
    let registry_adapter_id = command.registry_adapter_id;
    let publish_outcome = |short, scene_level, error| {
        publish_scene_programmed_outcome(publisher, correlation_id, adapter_id, command, short, scene_level, error, counters);
    };
    let sink: ProgramOutcomeSink<'_, u8> = ProgramOutcomeSink {
        read_port,
        publisher,
        correlation_id,
        registry_adapter_id,
        target: command.target,
        counters,
        publish_outcome: &publish_outcome,
    };
    match resolve_membership_short(read_port, registry_adapter_id, command.target) {
        Some(short_address) => program_scene_and_publish(
            controller,
            publisher,
            adapter_id,
            correlation_id,
            command,
            short_address,
            counters,
            &sink,
        ),
        None => sink.unbound(),
    }
    counters
        .semantic_program_scene_handled
        .fetch_add(1, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments, reason = "mirrors the outcome-publisher signature")]
fn program_scene_and_publish(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    command: &dali2rust_contracts::msg::DaliProgramSceneCommand,
    short_address: u8,
    counters: &DaliWorkerCounters,
    sink: &ProgramOutcomeSink<'_, u8>,
) {
    let result = crate::runtime::executor::scene::program_scene_row(
        controller,
        short_address,
        command.scene_id,
        command.action,
        command.target_state.as_ref(),
    );
    match result {
        Ok(scene_level) => {
            publish_scene_colour_verify(
                controller,
                publisher,
                adapter_id,
                correlation_id,
                command,
                short_address,
                counters,
            );
            sink.succeeded(short_address, scene_level);
        }
        Err(error) => sink.failed(&error),
    }
}

#[allow(clippy::too_many_arguments, reason = "mirrors the outcome-publisher signature")]
fn publish_scene_colour_verify(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    command: &dali2rust_contracts::msg::DaliProgramSceneCommand,
    short_address: u8,
    counters: &DaliWorkerCounters,
) {
    let Ok(chunk) = crate::runtime::executor::read_scene_colour_readback(
        controller,
        short_address,
        command.scene_id,
    ) else {
        return;
    };
    let ev = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliAttributesReadEvent {
            registry_adapter_id: command.registry_adapter_id,
            short_address,
            last_chunk: true,
            chunk,
        },
    );
    publish_event_required(publisher, ev, counters, PublishKind::Series, "attributes-read-chunk");
}

pub(super) fn handle_recall_scene(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    command: &dali2rust_contracts::msg::DaliRecallSceneCommand,
    counters: &DaliWorkerCounters,
) {
    let result = execute_recall_scene(controller, command);
    match result {
        Ok(()) => {
            publish_scene_recalled_outcome(publisher, correlation_id, adapter_id, command, None, counters);
            publish_confirmation_ok(publisher, correlation_id, counters);
        }
        Err(error) => {
            publish_scene_recalled_outcome(
                publisher,
                correlation_id,
                adapter_id,
                command,
                Some((error.code(), error.message())),
                counters,
            );
            emit_execution_failed_with_product_error(
                publisher,
                correlation_id,
                counters,
                error.code(),
                error.message(),
            );
        }
    }
    counters
        .semantic_recall_scene_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn execute_recall_scene(
    controller: &mut impl DaliApplicationController,
    command: &dali2rust_contracts::msg::DaliRecallSceneCommand,
) -> Result<(), SemanticDaliError> {
    let address = match command.scope {
        DaliTargetScope::Broadcast => DaliAddress::Broadcast,
        DaliTargetScope::Group => group_address(command.group_id)?,
        _ => return Err(SemanticDaliError::Conflict("invalid_recall_scope")),
    };
    if command.scene_id > 15 {
        return Err(SemanticDaliError::Conflict("invalid_scene_id"));
    }
    send_standard(
        controller,
        address,
        StandardCommand::GoToScene {
            scene: command.scene_id,
        },
    )?;
    Ok(())
}

pub(super) fn handle_recall_last_active_level(
    controller: &mut impl DaliApplicationController,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    command: &DaliRecallLastActiveLevelCommand,
    counters: &DaliWorkerCounters,
) {
    match execute_recall_last_active_level(controller, command) {
        Ok(()) => {
            publish_recall_last_active_applied(
                publisher,
                adapter_id,
                correlation_id,
                origin,
                command,
                counters,
            );
            publish_confirmation_ok(publisher, correlation_id, counters);
        }
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
    counters
        .semantic_recall_last_active_level_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn publish_recall_last_active_applied(
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    origin: Origin,
    command: &DaliRecallLastActiveLevelCommand,
    counters: &DaliWorkerCounters,
) {
    let (scope, group_id) = match command.scope {
        HclTargetScope::Broadcast => (DaliTargetScope::Broadcast, None),
        HclTargetScope::Group => (DaliTargetScope::Group, command.group_id),
    };
    let applied = super::target_state::target_state_applied_event(
        command.registry_adapter_id,
        scope,
        None,
        None,
        group_id,
        &LightSetpoint {
            power: dali2rust_contracts::msg::PowerState::On,
            level: 0,
            color: None,
        },
        RuntimeSource::from_origin(origin).unwrap_or(RuntimeSource::Api),
    );
    let env = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(Origin::Internal),
        applied,
    );
    let _ = publish_event_required(
        publisher,
        env,
        counters,
        PublishKind::Series,
        "recall-last-active-applied",
    );
}

fn execute_recall_last_active_level(
    controller: &mut impl DaliApplicationController,
    command: &DaliRecallLastActiveLevelCommand,
) -> Result<(), SemanticDaliError> {
    let address = recall_last_active_address(command)?;
    send_standard(controller, address, StandardCommand::GoToLastActiveLevel)?;
    Ok(())
}

fn recall_last_active_address(
    command: &DaliRecallLastActiveLevelCommand,
) -> Result<DaliAddress, SemanticDaliError> {
    match command.scope {
        HclTargetScope::Broadcast => Ok(DaliAddress::Broadcast),
        HclTargetScope::Group => {
            let group_id = command
                .group_id
                .ok_or(SemanticDaliError::Conflict("missing_group_id"))?;
            DaliAddress::group(group_id)
                .map_err(|_| SemanticDaliError::Conflict("invalid_group_id"))
        }
    }
}

fn publish_scene_recalled_outcome(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    command: &dali2rust_contracts::msg::DaliRecallSceneCommand,
    error: Option<(ErrorCode, &'static str)>,
    counters: &DaliWorkerCounters,
) {
    let event = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliSceneRecalledEvent {
            registry_adapter_id: command.registry_adapter_id,
            scope: command.scope,
            short_address: command.short_address,
            group_id: command.group_id,
            scene_id: command.scene_id,
            error: error
                .map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)),
            recalled_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
        },
    );
    publish_event_required(publisher, event, counters, PublishKind::Singleton, "scene-recalled");
}

#[allow(clippy::too_many_arguments, reason = "mirrors the membership outcome publisher")]
fn publish_scene_programmed_outcome(
    publisher: &BusPublisher,
    correlation_id: u64,
    adapter_id: BusId,
    command: &dali2rust_contracts::msg::DaliProgramSceneCommand,
    physical_short_address: Option<u8>,
    scene_level: Option<u8>,
    error: Option<(ErrorCode, &'static str)>,
    counters: &DaliWorkerCounters,
) {
    let target_state = if error.is_none() {
        command.target_state
    } else {
        None
    };
    let event = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        adapter_id.0,
        Some(dali2rust_contracts::msg::Origin::Internal),
        dali2rust_contracts::msg::DaliSceneProgrammedEvent {
            registry_adapter_id: command.registry_adapter_id,
            target: command.target,
            scene_id: command.scene_id,
            action: command.action,
            physical_short_address,
            target_state,
            scene_level,
            error: error
                .map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)),
        },
    );
    publish_event_required(publisher, event, counters, PublishKind::Singleton, "scene-programmed");
}

pub(super) fn scene_levels_fixed(levels: Option<&[u8]>) -> Option<[u8; 16]> {
    let src = levels?;
    let mut out = [0u8; 16];
    let take = src.len().min(16);
    out[..take].copy_from_slice(&src[..take]);
    Some(out)
}
