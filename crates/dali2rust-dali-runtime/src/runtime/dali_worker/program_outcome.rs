use super::*;

pub(super) fn resolve_membership_short(
    read_port: &dyn RegistryReadPort,
    registry_adapter_id: u8,
    target: DaliProgramTarget,
) -> Option<u8> {
    match target {
        DaliProgramTarget::Short { short_address } => Some(short_address),
        DaliProgramTarget::VirtualLamp { virtual_lamp_id } => {
            read_port.virtual_lamp_binding_short(registry_adapter_id, virtual_lamp_id)
        }
    }
}

pub(super) type ProgramOutcomeEvent<'a, V> =
    dyn Fn(Option<u8>, Option<V>, Option<(ErrorCode, &'static str)>) + 'a;

pub(super) struct ProgramOutcomeSink<'a, V> {
    pub(super) read_port: &'a dyn RegistryReadPort,
    pub(super) publisher: &'a BusPublisher,
    pub(super) correlation_id: u64,
    pub(super) registry_adapter_id: u8,
    pub(super) target: DaliProgramTarget,
    pub(super) counters: &'a DaliWorkerCounters,
    pub(super) publish_outcome: &'a ProgramOutcomeEvent<'a, V>,
}

impl<V> ProgramOutcomeSink<'_, V> {
    pub(super) fn succeeded(&self, physical_short_address: u8, readback: Option<V>) {
        (self.publish_outcome)(Some(physical_short_address), readback, None);
        publish_confirmation_ok(self.publisher, self.correlation_id, self.counters);
    }

    pub(super) fn failed(&self, error: &SemanticDaliError) {
        let short = resolve_membership_short(self.read_port, self.registry_adapter_id, self.target);
        self.fail(short, error.code(), error.message());
    }

    pub(super) fn unbound(&self) {
        self.fail(None, ErrorCode::VlUnbound, "vl_unbound");
    }

    fn fail(&self, physical_short_address: Option<u8>, code: ErrorCode, message: &'static str) {
        (self.publish_outcome)(physical_short_address, None, Some((code, message)));
        emit_execution_failed_with_product_error(
            self.publisher,
            self.correlation_id,
            self.counters,
            code,
            message,
        );
    }
}
