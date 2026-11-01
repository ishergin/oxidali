use super::*;
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

const SWITCHING_ARM_UNCONFIRMED: &str = "dt7_switching_arm_unconfirmed";

fn threshold_slot(write: &mut Dt7SwitchingWrite, threshold: Threshold) -> &mut Option<u8> {
    match threshold {
        Threshold::UpSwitchOn => &mut write.up_switch_on_threshold,
        Threshold::UpSwitchOff => &mut write.up_switch_off_threshold,
        Threshold::DownSwitchOn => &mut write.down_switch_on_threshold,
        Threshold::DownSwitchOff => &mut write.down_switch_off_threshold,
    }
}

fn switching_step(store: Dt7Command, query: Dt7Command) -> ExtendedStore {
    ExtendedStore {
        store: ExtendedCommand::Dt7(store),
        query: ExtendedCommand::Dt7(query),
        arm_unconfirmed: SWITCHING_ARM_UNCONFIRMED,
    }
}

// IEC 62386-208 §11.3.4.1
pub(super) fn write_switching(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    mut switching: Dt7SwitchingWrite,
) -> Result<(), SemanticDaliError> {
    for threshold in Threshold::ALL {
        let Some(value) = *threshold_slot(&mut switching, threshold) else {
            continue;
        };
        let step = switching_step(Dt7Command::StoreDtrAsThreshold(threshold), Dt7Command::QueryThreshold(threshold));
        let confirmed = store_extended_verified(controller, address, tally, value, &step)?;
        *threshold_slot(&mut tally.confirmed.switching, threshold) = confirmed;
    }
    let Some(hold_off) = switching.error_hold_off else {
        return Ok(());
    };
    let step = switching_step(Dt7Command::StoreDtrAsErrorHoldOffTime, Dt7Command::QueryErrorHoldOffTime);
    tally.confirmed.switching.error_hold_off = store_extended_verified(controller, address, tally, hold_off, &step)?;
    Ok(())
}
