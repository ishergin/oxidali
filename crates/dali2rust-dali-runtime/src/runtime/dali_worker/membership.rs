use super::*;

pub(super) fn handle_program_group_membership(
    controller: &mut impl DaliApplicationController,
    read_port: &dyn RegistryReadPort,
    publisher: &BusPublisher,
    adapter_id: BusId,
    correlation_id: u64,
    command: &DaliProgramGroupMembershipCommand,
    counters: &DaliWorkerCounters,
) {
    let registry_adapter_id = command.registry_adapter_id;
    let outcome_event = |short, membership, error| {
        group_membership_event(correlation_id, adapter_id, registry_adapter_id, command, short, membership, error)
    };
    let sink: ProgramOutcomeSink<'_, u16> = ProgramOutcomeSink {
        read_port,
        publisher,
        correlation_id,
        registry_adapter_id,
        target: command.target,
        counters,
        outcome_label: "group-membership-programmed",
        outcome_event: &outcome_event,
    };
    match resolve_membership_short(read_port, registry_adapter_id, command.target) {
        Some(short_address) => {
            run_program_group_membership(controller, short_address, command, &sink)
        }
        None => sink.unbound(),
    }
    counters
        .semantic_program_group_membership_handled
        .fetch_add(1, Ordering::Relaxed);
}

fn run_program_group_membership(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    command: &DaliProgramGroupMembershipCommand,
    sink: &ProgramOutcomeSink<'_, u16>,
) {
    match program_group_membership(controller, short_address, command.group_id, command.action) {
        Ok(membership) => sink.succeeded(short_address, membership),
        Err(error) => sink.failed(&error),
    }
}

fn group_membership_event(
    correlation_id: u64,
    adapter_id: BusId,
    registry_adapter_id: u8,
    command: &DaliProgramGroupMembershipCommand,
    physical_short_address: Option<u8>,
    membership: Option<u16>,
    error: Option<(ErrorCode, &'static str)>,
) -> EventEnvelope {
    dali2rust_contracts::bus::event_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, adapter_id.0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::DaliGroupMembershipProgrammedEvent { registry_adapter_id, target: command.target, group_id: command.group_id, action: command.action, physical_short_address, membership, error: (error).map(|(code, message)| dali2rust_contracts::msg::CompactErrorPayload::new(code, message)) })
}
