use super::*;
use dali2rust_contracts::msg::Dt7ReadSnapshot;
use dali2rust_domain::dali::devices::dt7_switching::{Dt7Command, Threshold};

pub(super) fn read_dt7_snapshot(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    device_types: &mut DeclaredTypes,
) -> Result<Option<Dt7ReadSnapshot>, SemanticDaliError> {
    let declared = device_types.resolve(controller, address, content_confirm)?;
    if declared.is_some_and(|types| !types.contains(DeviceType::Switching.code())) {
        return Ok(None);
    }
    let mut query = |cmd| {
        send_extended_query_stable(controller, content_confirm, address, ExtendedCommand::Dt7(cmd))
    };
    Ok(Some(Dt7ReadSnapshot {
        features: query(Dt7Command::QueryFeatures)?,
        switch_status: query(Dt7Command::QuerySwitchStatus)?,
        up_switch_on_threshold: query(Dt7Command::QueryThreshold(Threshold::UpSwitchOn))?,
        up_switch_off_threshold: query(Dt7Command::QueryThreshold(Threshold::UpSwitchOff))?,
        down_switch_on_threshold: query(Dt7Command::QueryThreshold(Threshold::DownSwitchOn))?,
        down_switch_off_threshold: query(Dt7Command::QueryThreshold(Threshold::DownSwitchOff))?,
        error_hold_off: query(Dt7Command::QueryErrorHoldOffTime)?,
        gear_type: query(Dt7Command::QueryGearType)?,
    }))
}
