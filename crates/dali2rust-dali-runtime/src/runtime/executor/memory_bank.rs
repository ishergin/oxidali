use dali2rust_domain::dali::banks::{chunk_len, field_containing, is_field_boundary, BANK_META_LEN};
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use dali2rust_contracts::msg::MemoryBankReadPreset;

use crate::runtime::executor::helpers::{
    dali_short_address, send_raw_query_once, send_special, send_standard_query, SemanticDaliError,
    READ_MEMORY_LOCATION_OPCODE,
};

const BANK0_IDENTITY_LAST_OFFSET: u16 =
    dali2rust_domain::dali::banks::BANK0_EXTENDED_LAST_OFFSET as u16;
const BANK1_PROFILE_LAST_OFFSET: u16 =
    dali2rust_domain::dali::banks::BANK1_FORMAT_GATE_LAST_OFFSET as u16;
const MEMORY_RESERVED_OFFSET: u16 = 0x01;
const MEMORY_NO_ANSWER_PLACEHOLDER: u8 = 0xFF;
const MEMORY_READ_LOCATION_RETRIES: u8 = 2;
const BANK_ARM_RETRIES: u8 = 2;
const MEMORY_MISALIGNED: &str = "memory_bank_read_misaligned";
const MEMORY_POINTER_UNCONFIRMED: &str = "memory_bank_pointer_unconfirmed";
const MEMORY_LATCH_LOST: &str = "memory_bank_latch_lost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReadExecution {
    pub bytes: Vec<u8>,
    pub error: Option<SemanticDaliError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadStop {
    Planned,
    BankEnded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedMemoryBankRead {
    pub bank: u8,
    pub start: u16,
    pub length: u16,
}

pub fn read_memory_bank(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    bank: u8,
    start: u16,
    length: u16,
) -> MemoryReadExecution {
    if let Err(error) = validate_memory_read_range(start, length) {
        return MemoryReadExecution {
            bytes: Vec::new(),
            error: Some(error),
        };
    }
    let address = match dali_short_address(short_address) {
        Ok(address) => address,
        Err(error) => {
            return MemoryReadExecution {
                bytes: Vec::new(),
                error: Some(error),
            };
        }
    };
    if let Err(error) = arm_memory_pointer(controller, address, bank, start) {
        return MemoryReadExecution {
            bytes: Vec::new(),
            error: Some(error),
        };
    }
    collect_memory_bank_bytes(controller, address, bank, start, length)
}

const MEMORY_READ_CHUNK: u16 = 5;

fn validate_memory_read_range(start: u16, length: u16) -> Result<(), SemanticDaliError> {
    if start > u16::from(u8::MAX) || u32::from(start) + u32::from(length) > 256 {
        return Err(SemanticDaliError::Conflict("memory_range_out_of_bounds"));
    }
    Ok(())
}

fn arm_memory_pointer(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bank: u8,
    offset: u16,
) -> Result<(), SemanticDaliError> {
    for _ in 0..=BANK_ARM_RETRIES {
        let armed = controller.transaction(|controller| {
            send_special(controller, SpecialCommand::Dtr1(bank))?;
            send_special(controller, SpecialCommand::Dtr0(offset as u8))?;
            pointer_confirmed(controller, address, bank, offset as u8)
        })?;
        if armed {
            return Ok(());
        }
    }
    Err(SemanticDaliError::OperationFailed(
        MEMORY_POINTER_UNCONFIRMED,
    ))
}

fn pointer_confirmed(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bank: u8,
    offset: u8,
) -> Result<bool, SemanticDaliError> {
    let armed_bank = send_standard_query(controller, address, StandardCommand::QueryContentDtr1)?;
    let armed_offset = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
    Ok(armed_bank == Some(bank) && armed_offset == Some(offset))
}

// IEC 62386-102 §9.10.4
fn confirm_final_position(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    start: u16,
    read: u16,
    stop: ReadStop,
) -> Result<(), SemanticDaliError> {
    // IEC 62386-102 §11.6.4
    let counted = start.saturating_add(read).min(u16::from(u8::MAX)) as u8;
    let answered = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
    let accepted = answered == Some(counted)
        || (stop == ReadStop::BankEnded && answered == Some(counted.saturating_add(1)));
    accepted
        .then_some(())
        .ok_or(SemanticDaliError::OperationFailed(MEMORY_MISALIGNED))
}

fn collect_memory_bank_bytes(
    controller: &mut impl DaliApplicationController,
    address: dali2rust_domain::dali::net::address::DaliAddress,
    bank: u8,
    start: u16,
    length: u16,
) -> MemoryReadExecution {
    let mut bytes = Vec::with_capacity(length as usize);
    let stop = match read_in_chunks(controller, address, bank, start, length, &mut bytes) {
        Ok(stop) => stop,
        Err(error) => return MemoryReadExecution { bytes, error: Some(error) },
    };
    let read = bytes.len() as u16;
    match confirm_final_position(controller, address, start, read, stop) {
        Ok(()) => MemoryReadExecution { bytes, error: None },
        Err(error) => MemoryReadExecution {
            bytes: Vec::new(),
            error: Some(error),
        },
    }
}

fn read_in_chunks(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bank: u8,
    start: u16,
    length: u16,
    bytes: &mut Vec<u8>,
) -> Result<ReadStop, SemanticDaliError> {
    let mut done = 0u16;
    while done < length {
        let chunk_start = start + done;
        debug_assert!(
            is_field_boundary(bank, chunk_start),
            "bank {bank} chunk starting at {chunk_start} is inside a latched value"
        );
        let take = chunk_len(bank, chunk_start, length - done, MEMORY_READ_CHUNK);
        controller.step_boundary();
        let stop = controller.transaction(|controller| {
            confirm_chunk_position(controller, address, chunk_start, done)?;
            read_planned_locations(controller, address, bank, chunk_start, take, bytes)
        })?;
        if stop == ReadStop::BankEnded {
            return Ok(stop);
        }
        done += take;
    }
    Ok(ReadStop::Planned)
}

fn confirm_chunk_position(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    chunk_start: u16,
    already_read: u16,
) -> Result<(), SemanticDaliError> {
    if already_read == 0 {
        return Ok(());
    }
    let expected = chunk_start.min(u16::from(u8::MAX)) as u8;
    let answered = send_standard_query(controller, address, StandardCommand::QueryContentDtr0)?;
    (answered == Some(expected))
        .then_some(())
        .ok_or(SemanticDaliError::OperationFailed(MEMORY_MISALIGNED))
}

enum LocationRead {
    Value(u8),
    BankEnded,
    FieldRestarted { field_start: u16 },
}

const MEMORY_FIELD_RESTARTS: u8 = 2;

fn read_planned_locations(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bank: u8,
    start: u16,
    length: u16,
    bytes: &mut Vec<u8>,
) -> Result<ReadStop, SemanticDaliError> {
    let end = start + length;
    let mut offset = start;
    let mut restarts: u8 = 0;
    while offset < end {
        match read_memory_location_with_retries(controller, address, bank, offset)? {
            LocationRead::Value(value) => {
                bytes.push(value);
                offset += 1;
            }
            LocationRead::BankEnded => return Ok(ReadStop::BankEnded),
            LocationRead::FieldRestarted { field_start } => {
                restarts += 1;
                if restarts > MEMORY_FIELD_RESTARTS {
                    return Err(SemanticDaliError::OperationFailed(MEMORY_LATCH_LOST));
                }
                let stale = usize::from(offset - field_start);
                bytes.truncate(bytes.len() - stale);
                offset = field_start;
            }
        }
    }
    Ok(ReadStop::Planned)
}

fn read_memory_location_with_retries(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bank: u8,
    absolute_offset: u16,
) -> Result<LocationRead, SemanticDaliError> {
    for attempt in 0..=MEMORY_READ_LOCATION_RETRIES {
        match send_raw_query_once(controller, address, READ_MEMORY_LOCATION_OPCODE) {
            Ok(Some(value)) => return Ok(LocationRead::Value(value)),
            Ok(None) if absolute_offset == MEMORY_RESERVED_OFFSET => {
                arm_memory_pointer(controller, address, bank, absolute_offset + 1)?;
                return Ok(LocationRead::Value(MEMORY_NO_ANSWER_PLACEHOLDER));
            }
            outcome if attempt == MEMORY_READ_LOCATION_RETRIES => {
                return outcome.map(|answered| match answered {
                    Some(value) => LocationRead::Value(value),
                    None => LocationRead::BankEnded,
                })
            }
            _ => {
                if let Some(field_start) = latched_field_start(bank, absolute_offset) {
                    arm_memory_pointer(controller, address, bank, field_start)?;
                    return Ok(LocationRead::FieldRestarted { field_start });
                }
                arm_memory_pointer(controller, address, bank, absolute_offset)?;
            }
        }
    }
    Ok(LocationRead::BankEnded)
}

// DiiA 252 §9.2.2
fn latched_field_start(bank: u8, absolute_offset: u16) -> Option<u16> {
    field_containing(bank, absolute_offset)
        .map(|field| u16::from(field.offset))
        .filter(|field_start| *field_start < absolute_offset)
}

fn read_bank_meta(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    bank: u8,
    meta_len: u16,
) -> Result<Option<Vec<u8>>, SemanticDaliError> {
    let execution = read_memory_bank(controller, short_address, bank, 0, meta_len);
    if let Some(error) = execution.error {
        return Err(error);
    }
    Ok((!execution.bytes.is_empty()).then_some(execution.bytes))
}

fn read_bank_verified(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    bank: u8,
    meta_len: u16,
    length_of: impl Fn(&[u8]) -> u16,
    on_execution: &mut impl FnMut(PlannedMemoryBankRead, &MemoryReadExecution),
) -> Result<Option<Vec<u8>>, SemanticDaliError> {
    let Some(meta) = read_bank_meta(controller, short_address, bank, meta_len)? else {
        return Ok(None);
    };
    let plan = PlannedMemoryBankRead {
        bank,
        start: 0,
        length: length_of(&meta),
    };
    let execution = read_memory_bank(controller, short_address, bank, plan.start, plan.length);
    if let Some(error) = execution.error {
        return Err(error);
    }
    on_execution(plan, &execution);
    Ok(Some(meta))
}

fn metered_plan(preset: MemoryBankReadPreset) -> &'static [PlannedMemoryBankRead] {
    use dali2rust_domain::dali::banks::{
        BANK_ACTIVE_ENERGY, BANK_APPARENT_ENERGY, BANK_CONTROL_GEAR_DIAGNOSTICS,
        BANK_LIGHT_SOURCE_DIAGNOSTICS, BANK_LOADSIDE_ENERGY, BANK_LUMINAIRE_MAINTENANCE,
    };
    const POWER: (u16, u16) = (0x0B, 5);
    const ENERGY: (u16, u16) = (0x03, 8);
    match preset {
        MemoryBankReadPreset::Power => &[
            PlannedMemoryBankRead { bank: BANK_ACTIVE_ENERGY, start: POWER.0, length: POWER.1 },
            PlannedMemoryBankRead { bank: BANK_APPARENT_ENERGY, start: POWER.0, length: POWER.1 },
            PlannedMemoryBankRead { bank: BANK_LOADSIDE_ENERGY, start: POWER.0, length: POWER.1 },
        ],
        MemoryBankReadPreset::Energy => &[
            PlannedMemoryBankRead { bank: BANK_ACTIVE_ENERGY, start: ENERGY.0, length: ENERGY.1 },
            PlannedMemoryBankRead { bank: BANK_APPARENT_ENERGY, start: ENERGY.0, length: ENERGY.1 },
            PlannedMemoryBankRead { bank: BANK_LOADSIDE_ENERGY, start: ENERGY.0, length: ENERGY.1 },
        ],
        MemoryBankReadPreset::Diagnostics => &[
            PlannedMemoryBankRead { bank: BANK_CONTROL_GEAR_DIAGNOSTICS, start: 0x03, length: 26 },
            PlannedMemoryBankRead { bank: BANK_LIGHT_SOURCE_DIAGNOSTICS, start: 0x03, length: 30 },
        ],
        MemoryBankReadPreset::LuminaireData => &[PlannedMemoryBankRead {
            bank: BANK_LUMINAIRE_MAINTENANCE,
            start: 0x03,
            length: 5,
        }],
        _ => &[],
    }
}

// DiiA 252 §9.2.1
fn read_metered_bank(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    plan: PlannedMemoryBankRead,
    on_execution: &mut impl FnMut(PlannedMemoryBankRead, &MemoryReadExecution),
) -> Result<bool, SemanticDaliError> {
    if read_bank_meta(controller, short_address, plan.bank, BANK_META_LEN)?.is_none() {
        return Ok(false);
    }
    let execution = read_memory_bank(controller, short_address, plan.bank, plan.start, plan.length);
    if let Some(error) = execution.error {
        return Err(error);
    }
    on_execution(plan, &execution);
    Ok(true)
}

fn execute_metered_preset(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    preset: MemoryBankReadPreset,
    on_execution: &mut impl FnMut(PlannedMemoryBankRead, &MemoryReadExecution),
) -> Result<(), SemanticDaliError> {
    for (index, plan) in metered_plan(preset).iter().enumerate() {
        let present = read_metered_bank(controller, short_address, *plan, on_execution)?;
        if !present && index == 0 {
            return Ok(());
        }
    }
    Ok(())
}

pub fn execute_memory_bank_preset(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    preset: MemoryBankReadPreset,
    mut on_execution: impl FnMut(PlannedMemoryBankRead, &MemoryReadExecution),
) -> Result<(), SemanticDaliError> {
    if preset == MemoryBankReadPreset::None {
        return Ok(());
    }
    if !metered_plan(preset).is_empty() {
        return execute_metered_preset(controller, short_address, preset, &mut on_execution);
    }
    let meta = read_bank_verified(
        controller,
        short_address,
        0,
        3,
        |meta| bank0_length_for_preset(meta[0], preset),
        &mut on_execution,
    )?
    .ok_or(SemanticDaliError::OperationFailed(
        "memory_bank_last_address_missing",
    ))?;
    let last_bank = *meta.get(2).unwrap_or(&0);
    sweep_upper_banks(
        controller,
        short_address,
        preset,
        last_bank,
        &mut on_execution,
    )
}

fn sweep_upper_banks(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    preset: MemoryBankReadPreset,
    last_bank: u8,
    on_execution: &mut impl FnMut(PlannedMemoryBankRead, &MemoryReadExecution),
) -> Result<(), SemanticDaliError> {
    for bank in 1..=last_bank {
        let read = read_bank_verified(
            controller,
            short_address,
            bank,
            1,
            |meta| upper_bank_length(meta[0], preset),
            on_execution,
        )?;
        if read.is_none() || preset != MemoryBankReadPreset::All {
            break;
        }
    }
    Ok(())
}

fn bank0_length_for_preset(last_address: u8, preset: MemoryBankReadPreset) -> u16 {
    match preset {
        MemoryBankReadPreset::Identity => {
            (u16::from(last_address) + 1).min(BANK0_IDENTITY_LAST_OFFSET + 1)
        }
        MemoryBankReadPreset::Profile | MemoryBankReadPreset::All => u16::from(last_address) + 1,
        MemoryBankReadPreset::None
        | MemoryBankReadPreset::Power
        | MemoryBankReadPreset::Energy
        | MemoryBankReadPreset::Diagnostics
        | MemoryBankReadPreset::LuminaireData => 0,
    }
}

fn upper_bank_length(last_address: u8, preset: MemoryBankReadPreset) -> u16 {
    match preset {
        MemoryBankReadPreset::Identity => {
            (u16::from(last_address) + 1).min(BANK1_PROFILE_LAST_OFFSET + 1)
        }
        _ => u16::from(last_address) + 1,
    }
}

#[cfg(test)]
mod tests;
