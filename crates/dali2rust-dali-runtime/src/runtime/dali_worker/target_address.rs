use super::*;

pub(super) fn scoped_address(
    read_port: &dyn RegistryReadPort,
    registry_adapter_id: u8,
    scope: DaliTargetScope,
    virtual_lamp_id: u8,
    short_address: u8,
    group_id: u8,
) -> Result<DaliAddress, SemanticDaliError> {
    match scope {
        DaliTargetScope::Broadcast => Ok(DaliAddress::Broadcast),
        DaliTargetScope::Group => DaliAddress::group(group_id)
            .map_err(|_| SemanticDaliError::Conflict("invalid_group_id")),
        DaliTargetScope::Short => DaliAddress::short(short_address)
            .map_err(|_| SemanticDaliError::Conflict("invalid_short_address")),
        DaliTargetScope::VirtualLamp => {
            let short = read_port
                .virtual_lamp_binding_short(registry_adapter_id, virtual_lamp_id)
                .ok_or(SemanticDaliError::Conflict(VL_UNBOUND_MESSAGE))?;
            DaliAddress::short(short)
                .map_err(|_| SemanticDaliError::Conflict("invalid_short_address"))
        }
        DaliTargetScope::AddressRange => {
            Err(SemanticDaliError::Conflict("unsupported_scope"))
        }
    }
}
