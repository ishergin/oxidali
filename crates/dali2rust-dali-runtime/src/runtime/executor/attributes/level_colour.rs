use super::*;
use super::dt8::{armed_colour_value_query, ArmedValue};
use super::write::verified_store;
use crate::runtime::executor::helpers::DT8_DEVICE_TYPE;
use crate::runtime::executor::target_state::stage_dtrs_verified;
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::devices::dt8_color::{
    COLOUR_TYPE_BYTE_MASK, COLOUR_TYPE_BYTE_PRIMARY_N, COLOUR_VALUE_REPORT_FREECOLOUR,
    COLOUR_VALUE_REPORT_PRIMARY_N_0, COLOUR_VALUE_REPORT_PRIMARY_N_5,
    COLOUR_VALUE_REPORT_RGBWAF_CONTROL, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE,
    COLOUR_VALUE_TEMPORARY_FREECOLOUR, COLOUR_VALUE_TEMPORARY_PRIMARY_N_0,
    COLOUR_VALUE_TEMPORARY_PRIMARY_N_5, COLOUR_VALUE_TEMPORARY_RED,
    COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL, COLOUR_VALUE_TEMPORARY_TC, COLOUR_VALUE_TEMPORARY_X,
    COLOUR_VALUE_TEMPORARY_Y,
};

const LEVEL_COLOUR_UNCONFIRMED: &str = "dt8_level_colour_unconfirmed";
const LEVEL_COLOUR_UNANSWERED: &str = "dt8_level_colour_unanswered";
const LEVEL_COLOUR_CONTENDED: &str = "dt8_level_colour_contended";
const DEVICE_TYPES_UNKNOWN: &str = "types_unknown_read_or_override";
const COLOUR_VALUE_ASKS: u8 = 3;
const RECHARGE_RETRIES: u8 = 2;
const PRIMARY_N_COUNT: u8 = 6;
const MASK_WORD: [u8; 2] = [DT8_COLOUR_VALUE_LEVEL_MASK; 2];
const MASK_TRIO: [u8; 3] = [DT8_COLOUR_VALUE_LEVEL_MASK; 3];
const MASK_BYTE: [u8; 1] = [DT8_COLOUR_VALUE_LEVEL_MASK];

type Staging<'a> = (Dt8Command, &'a [u8]);

#[derive(Debug, Clone, Copy)]
pub(super) struct StoredLevel {
    store: StandardCommand,
    query: StandardCommand,
}

pub(super) const POWER_ON: StoredLevel = StoredLevel {
    store: StandardCommand::SetPowerOnLevel,
    query: StandardCommand::QueryPowerOnLevel,
};

pub(super) const SYSTEM_FAILURE: StoredLevel = StoredLevel {
    store: StandardCommand::SetSystemFailureLevel,
    query: StandardCommand::QuerySystemFailureLevel,
};

const fn unconfirmed() -> SemanticDaliError {
    SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNCONFIRMED)
}

// IEC 62386-102 §3.13
fn types_unknown_when_silent(error: SemanticDaliError) -> SemanticDaliError {
    match error {
        SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNANSWERED) => {
            SemanticDaliError::OperationFailed(DEVICE_TYPES_UNKNOWN)
        }
        other => other,
    }
}

// IEC 62386-102 §9.18
pub(super) fn store_level<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    level: u8,
    (stored, types): (StoredLevel, Option<DeviceTypeSet>),
) -> Result<ReadBack, SemanticDaliError> {
    if types.is_some_and(|set| !set.contains(DT8_DEVICE_TYPE)) {
        let verify = |c: &mut C| send_standard_response(c, address, stored.query);
        return send_dtr0_config_verified(controller, address, level, stored.store, verify);
    }
    store_level_keeping_colour(controller, address, level, (stored, types.is_some()))
}

// IEC 62386-209 §11.2.2, Table 6
fn store_level_keeping_colour<C: DaliApplicationController>(
    controller: &mut C,
    address: DaliAddress,
    level: u8,
    (stored, types_known): (StoredLevel, bool),
) -> Result<ReadBack, SemanticDaliError> {
    verified_store(controller, level, |controller: &mut C| {
        controller.transaction_exempt(|controller| {
            controller.unit(|controller| {
                keep_stored_colour(controller, address, (stored, types_known))?;
                send_dtr0_backed_standard(controller, address, level, stored.store)
            })?;
            send_standard_response(controller, address, stored.query)
        })
    })
}

// IEC 62386-209 §9.2.2, Table 1, §9.3.2, Table 2
fn keep_stored_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    (stored, types_known): (StoredLevel, bool),
) -> Result<(), SemanticDaliError> {
    let temporary = colour_byte(controller, address, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE)
        .map_err(|error| if types_known { error } else { types_unknown_when_silent(error) })?;
    if temporary == COLOUR_TYPE_BYTE_MASK {
        return Ok(());
    }
    for _ in 0..=RECHARGE_RETRIES {
        let charged = recharge(controller, address, stored, temporary)?;
        if temporary_holds(controller, address, charged)? {
            return Ok(());
        }
    }
    Err(unconfirmed())
}

// IEC 62386-209 §9.12.7, §11.3.2
fn recharge(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    stored: StoredLevel,
    temporary: u8,
) -> Result<Charge, SemanticDaliError> {
    answer_of(send_standard_response(controller, address, stored.query)?)?;
    let reported = colour_byte(controller, address, DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE)?;
    if reported == COLOUR_TYPE_BYTE_MASK {
        charge_mask_colour(controller, address, temporary)?;
        let selectors = proof_selectors(temporary).ok_or_else(unconfirmed)?;
        return Ok((temporary, selectors.map(|(temporary_value, _)| mask_of(temporary_value))));
    }
    let selectors = proof_selectors(reported).ok_or_else(unconfirmed)?;
    let mut values = [None; PROOF_VALUES];
    for (slot, (_, report_value)) in values.iter_mut().zip(selectors) {
        *slot = report_value.map(|value| colour_value(controller, address, value)).transpose()?;
    }
    send_dt8(controller, address, Dt8Command::CopyReportToTemporary)?;
    Ok((reported, values))
}

fn temporary_holds(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    (colour_type, values): Charge,
) -> Result<bool, SemanticDaliError> {
    let selectors = proof_selectors(colour_type).ok_or_else(unconfirmed)?;
    if colour_byte(controller, address, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE)? != colour_type {
        return Ok(false);
    }
    for ((temporary_value, _), expected) in selectors.into_iter().zip(values) {
        let Some(selector) = temporary_value else { continue };
        if Some(colour_value(controller, address, selector)?) != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

const PROOF_VALUES: usize = 3;

const fn pair(temporary: u8, report: u8) -> ProofSelector {
    (Some(temporary), Some(report))
}

type ProofSelector = (Option<u8>, Option<u8>);

type Charge = (u8, [Option<u16>; PROOF_VALUES]);

// IEC 62386-209 §9.12.4, Table 11
const fn proof_selectors(colour_type: u8) -> Option<[ProofSelector; PROOF_VALUES]> {
    const NONE: ProofSelector = (None, None);
    Some(match colour_type {
        DT8_COLOUR_TYPE_XY => [
            pair(COLOUR_VALUE_TEMPORARY_X, DT8_COLOUR_VALUE_REPORT_X),
            pair(COLOUR_VALUE_TEMPORARY_Y, DT8_COLOUR_VALUE_REPORT_Y),
            NONE,
        ],
        DT8_COLOUR_TYPE_TC => [pair(COLOUR_VALUE_TEMPORARY_TC, DT8_COLOUR_VALUE_REPORT_TC), NONE, NONE],
        COLOUR_TYPE_BYTE_PRIMARY_N => [
            pair(COLOUR_VALUE_TEMPORARY_PRIMARY_N_0, COLOUR_VALUE_REPORT_PRIMARY_N_0),
            pair(COLOUR_VALUE_TEMPORARY_PRIMARY_N_5, COLOUR_VALUE_REPORT_PRIMARY_N_5),
            NONE,
        ],
        DT8_COLOUR_TYPE_RGBWAF => [
            pair(COLOUR_VALUE_TEMPORARY_RED, DT8_COLOUR_VALUE_REPORT_RED),
            pair(COLOUR_VALUE_TEMPORARY_FREECOLOUR, COLOUR_VALUE_REPORT_FREECOLOUR),
            pair(COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL, COLOUR_VALUE_REPORT_RGBWAF_CONTROL),
        ],
        _ => return None,
    })
}

const fn mask_of(selector: Option<u8>) -> Option<u16> {
    match selector {
        Some(selector) if colour_value_is_wide(selector) => Some(DT8_COLOUR_VALUE_MASK),
        Some(_) => Some(DT8_COLOUR_VALUE_LEVEL_MASK as u16),
        None => None,
    }
}

// IEC 62386-209 §9.9
fn colour_value(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
) -> Result<u16, SemanticDaliError> {
    let high = colour_byte(controller, address, value_id)?;
    if !colour_value_is_wide(value_id) {
        return Ok(u16::from(high));
    }
    let low = answer_of(send_standard_response(controller, address, StandardCommand::QueryContentDtr0)?)?;
    Ok((u16::from(high) << 8) | u16::from(low))
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §3.13
const fn answer_of(response: DaliResponse) -> Result<u8, SemanticDaliError> {
    match response {
        DaliResponse::Answer(value) => Ok(value),
        DaliResponse::NoAnswer => Err(SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNANSWERED)),
        DaliResponse::Violation => Err(SemanticDaliError::OperationFailed(LEVEL_COLOUR_CONTENDED)),
    }
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §3.13
fn colour_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value_id: u8,
) -> Result<u8, SemanticDaliError> {
    let mut worst = Unheard::Silent;
    for _ in 0..COLOUR_VALUE_ASKS {
        let unheard = match controller.unit(|c| armed_colour_value_query(c, address, value_id))? {
            ArmedValue::Answered(DaliResponse::Answer(value), false) => return Ok(value),
            ArmedValue::Answered(response, contended) => Unheard::of_answer(response, contended),
            ArmedValue::Unarmed(echo) => Unheard::of_echo(echo),
        };
        worst = worst.max(unheard);
    }
    Err(SemanticDaliError::OperationFailed(worst.outcome()))
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §3.13
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unheard {
    Silent,
    Contended,
    WrongEcho,
}

impl Unheard {
    const fn of_answer(response: DaliResponse, contended: bool) -> Self {
        match (response, contended) {
            (DaliResponse::NoAnswer, false) => Self::Silent,
            _ => Self::Contended,
        }
    }

    const fn of_echo(echo: DaliResponse) -> Self {
        match echo {
            DaliResponse::Answer(_) => Self::WrongEcho,
            DaliResponse::Violation => Self::Contended,
            DaliResponse::NoAnswer => Self::Silent,
        }
    }

    const fn outcome(self) -> &'static str {
        match self {
            Self::Silent => LEVEL_COLOUR_UNANSWERED,
            Self::Contended => LEVEL_COLOUR_CONTENDED,
            Self::WrongEcho => LEVEL_COLOUR_UNCONFIRMED,
        }
    }
}

// IEC 62386-209 §9.12.4, §11.3.4.1, Table 8
fn charge_mask_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    colour_type: u8,
) -> Result<(), SemanticDaliError> {
    if colour_type == COLOUR_TYPE_BYTE_PRIMARY_N {
        return (0..PRIMARY_N_COUNT).try_for_each(|primary| {
            let operands = [DT8_COLOUR_VALUE_LEVEL_MASK, DT8_COLOUR_VALUE_LEVEL_MASK, primary];
            stage_and_set(controller, address, (Dt8Command::SetTemporaryPrimaryNDimLevel, &operands))
        });
    }
    mask_stagings(colour_type)
        .ok_or_else(unconfirmed)?
        .iter()
        .try_for_each(|staging| stage_and_set(controller, address, *staging))
}

fn mask_stagings(colour_type: u8) -> Option<&'static [Staging<'static>]> {
    match colour_type {
        DT8_COLOUR_TYPE_XY => Some(&[
            (Dt8Command::SetTemporaryXCoordinate, &MASK_WORD),
            (Dt8Command::SetTemporaryYCoordinate, &MASK_WORD),
        ]),
        DT8_COLOUR_TYPE_TC => Some(&[(Dt8Command::SetTemporaryColourTemperature, &MASK_WORD)]),
        DT8_COLOUR_TYPE_RGBWAF => Some(&[
            (Dt8Command::SetTemporaryRgbDimLevel, &MASK_TRIO),
            (Dt8Command::SetTemporaryWafDimLevel, &MASK_TRIO),
            (Dt8Command::SetTemporaryRgbwafControl, &MASK_BYTE),
        ]),
        _ => None,
    }
}

fn stage_and_set(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    (command, operands): Staging<'_>,
) -> Result<(), SemanticDaliError> {
    stage_dtrs_verified(controller, address, operands)?;
    send_dt8(controller, address, command)
}

fn send_dt8(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    command: Dt8Command,
) -> Result<(), SemanticDaliError> {
    send_extended_command(controller, address, ExtendedCommand::Dt8(command)).map(drop)
}

#[cfg(test)]
mod tests;
