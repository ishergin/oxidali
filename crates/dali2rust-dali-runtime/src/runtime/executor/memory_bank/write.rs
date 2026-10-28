use dali2rust_contracts::msg::ErrorCode;
use dali2rust_domain::dali::banks::luminaire_write::{
    check_writable, Bank1Layout, Bank1WriteRefusal, FieldWrite, BANK1_HEADER_LEN, BANK_UNLOCKED,
};
use dali2rust_domain::dali::banks::{BANK_LOCK_BYTE, BANK_LUMINAIRE_INFO};
use dali2rust_domain::dali::commands::DaliResponse;
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use super::super::dev103::verify_readback;
use super::{
    pointer_proof, read_memory_bank, MemoryReadExecution, PointerProof, BANK_ARM_RETRIES,
    BUS_CONTENDED, MEMORY_LATCH_LOST, MEMORY_MISALIGNED, MEMORY_POINTER_UNCONFIRMED,
};
use crate::runtime::executor::helpers::{
    dali_short_address, send_raw_query_once_response, send_special, send_special_response,
    send_standard, SemanticDaliError, READ_MEMORY_LOCATION_OPCODE,
};

const BANK1: u8 = BANK_LUMINAIRE_INFO;

// IEC 62386-101 §9.2
const UNIT_BYTES: usize = 3;

const UNITS_PER_SESSION: usize = 2;

pub const LOCK_SUBJECT: &str = "bank1_lock_byte";

pub const READBACK_SUBJECT: &str = "bank1_readback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuminaireWriteFailure {
    pub code: ErrorCode,
    pub subject: &'static str,
}

impl LuminaireWriteFailure {
    const fn new(code: ErrorCode, subject: &'static str) -> Self {
        Self { code, subject }
    }

    const fn refused(refusal: Bank1WriteRefusal) -> Self {
        Self::new(ErrorCode::Conflict, refusal.cause())
    }

    fn of(error: SemanticDaliError) -> Self {
        Self::new(error.code(), error.message())
    }

    // IEC 62386-102 §9.10.5
    fn unproved(error: SemanticDaliError, subject: &'static str) -> Self {
        let code = match error.message() {
            MEMORY_POINTER_UNCONFIRMED => ErrorCode::VerifyUnanswered,
            BUS_CONTENDED | MEMORY_LATCH_LOST => ErrorCode::VerifyContended,
            MEMORY_MISALIGNED => ErrorCode::VerifyFailed,
            _ => error.code(),
        };
        Self::new(code, subject)
    }

    fn is_preempted(self) -> bool {
        self.code == ErrorCode::Preempted
    }
}

#[derive(Debug, Clone, Copy)]
struct Unit<'a> {
    subject: &'static str,
    offset: u8,
    bytes: &'a [u8],
}

fn units(writes: &[FieldWrite]) -> Vec<Unit<'_>> {
    writes
        .iter()
        .flat_map(|write| {
            write.bytes.chunks(UNIT_BYTES).enumerate().map(move |(index, bytes)| Unit {
                subject: write.field.name,
                offset: write.field.offset + (index * UNIT_BYTES) as u8,
                bytes,
            })
        })
        .collect()
}

fn armed<C: DaliApplicationController, R>(
    controller: &mut C,
    address: DaliAddress,
    (offset, subject): (u8, &'static str),
    mut act: impl FnMut(&mut C) -> Result<R, SemanticDaliError>,
) -> Result<R, LuminaireWriteFailure> {
    let mut proof = PointerProof::Unanswered;
    for _ in 0..=BANK_ARM_RETRIES {
        let outcome = controller
            .unit(|c| {
                send_special(c, SpecialCommand::Dtr1(BANK1))?;
                send_special(c, SpecialCommand::Dtr0(offset))?;
                match pointer_proof(c, address, BANK1, (offset, false))? {
                    PointerProof::Held => act(c).map(Ok),
                    other => Ok(Err(other)),
                }
            })
            .map_err(|error| LuminaireWriteFailure::unproved(error, subject))?;
        match outcome {
            Ok(done) => return Ok(done),
            Err(other) => proof = other,
        }
    }
    Err(LuminaireWriteFailure::unproved(proof.failure(MEMORY_MISALIGNED), subject))
}

// IEC 62386-102 §9.10.5
fn write_bytes(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bytes: &[u8],
) -> Result<Vec<DaliResponse>, SemanticDaliError> {
    send_standard(controller, address, StandardCommand::EnableWriteMemory)?;
    bytes
        .iter()
        .map(|byte| send_special_response(controller, SpecialCommand::WriteMemoryLocation(*byte)))
        .collect()
}

fn confirm(seen: &[DaliResponse], expected: &[u8], subject: &'static str) -> Result<(), LuminaireWriteFailure> {
    expected.iter().enumerate().try_for_each(|(index, byte)| {
        let answer = seen.get(index).copied().unwrap_or(DaliResponse::NoAnswer);
        verify_readback(answer, |value| value == *byte)
            .map(|_| ())
            .map_err(|error| LuminaireWriteFailure::unproved(error, subject))
    })
}

fn write_lock_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value: u8,
) -> Result<(), LuminaireWriteFailure> {
    let target = (BANK_LOCK_BYTE, LOCK_SUBJECT);
    let seen = armed(controller, address, target, |c| write_bytes(c, address, &[value]))?;
    confirm(&seen, &[value], LOCK_SUBJECT)
}

// IEC 62386-102 §9.10.2, Table 8
fn open_lock(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(), LuminaireWriteFailure> {
    write_lock_byte(controller, address, BANK_UNLOCKED)?;
    let target = (BANK_LOCK_BYTE, LOCK_SUBJECT);
    let (seen, contended) = armed(controller, address, target, |c| {
        send_raw_query_once_response(c, address, READ_MEMORY_LOCATION_OPCODE)
    })?;
    let seen = if contended { DaliResponse::Violation } else { seen };
    confirm(&[seen], &[BANK_UNLOCKED], LOCK_SUBJECT)
}

fn write_units(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    session: &[Unit<'_>],
) -> Result<(), LuminaireWriteFailure> {
    for unit in session {
        let target = (unit.offset, unit.subject);
        let seen = armed(controller, address, target, |c| write_bytes(c, address, unit.bytes))?;
        confirm(&seen, unit.bytes, unit.subject)?;
    }
    Ok(())
}

fn write_session(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    lock: u8,
    session: &[Unit<'_>],
) -> Result<(), LuminaireWriteFailure> {
    if lock == BANK_UNLOCKED {
        return write_units(controller, address, session);
    }
    let written = open_lock(controller, address).and_then(|()| write_units(controller, address, session));
    let closed = write_lock_byte(controller, address, lock);
    match written {
        Err(failure) if failure.subject == LOCK_SUBJECT => Err(failure),
        written => closed.and(written),
    }
}

fn write_sessions(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    lock: u8,
    writes: &[FieldWrite],
) -> Result<(), LuminaireWriteFailure> {
    units(writes).chunks(UNITS_PER_SESSION).try_for_each(|session| {
        controller.step_boundary();
        write_session(controller, address, lock, session).map_err(|failure| match session.first() {
            Some(unit) if failure.is_preempted() => LuminaireWriteFailure::new(failure.code, unit.subject),
            _ => failure,
        })
    })
}

#[derive(Debug, Clone, Copy)]
struct Header {
    lock: u8,
    last_offset: u8,
}

fn read_header(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    writes: &[FieldWrite],
) -> Result<Header, LuminaireWriteFailure> {
    let header = read_memory_bank(controller, short_address, BANK1, 0, BANK1_HEADER_LEN);
    if let Some(error) = header.error {
        return Err(LuminaireWriteFailure::of(error));
    }
    let layout = Bank1Layout::from_header(&header.bytes);
    for write in writes {
        check_writable(&layout, write.field).map_err(LuminaireWriteFailure::refused)?;
    }
    let lock = header.bytes.get(usize::from(BANK_LOCK_BYTE)).copied();
    match (lock, layout.last_offset) {
        (Some(lock), Some(last_offset)) => Ok(Header { lock, last_offset }),
        _ => Err(LuminaireWriteFailure::refused(Bank1WriteRefusal::NotPart251)),
    }
}

fn verdict(
    written: Result<(), LuminaireWriteFailure>,
    readback: &MemoryReadExecution,
    writes: &[FieldWrite],
    lock: u8,
) -> Result<(), LuminaireWriteFailure> {
    if let Err(failure) = written {
        if failure.subject == LOCK_SUBJECT {
            return Err(failure);
        }
    }
    let lock_seen = readback.bytes.get(usize::from(BANK_LOCK_BYTE)).copied();
    if readback.error.is_none() && lock_seen != Some(lock) {
        return Err(LuminaireWriteFailure::new(ErrorCode::VerifyFailed, LOCK_SUBJECT));
    }
    written?;
    if let Some(error) = readback.error {
        return Err(LuminaireWriteFailure::unproved(error, READBACK_SUBJECT));
    }
    match writes.iter().find(|write| !write.held_by(&readback.bytes)) {
        Some(write) => Err(LuminaireWriteFailure::new(ErrorCode::VerifyFailed, write.field.name)),
        None => Ok(()),
    }
}

// IEC 62386-102 §9.10.5; DiiA 251 §9.2.3
pub fn write_luminaire_data(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    writes: &[FieldWrite],
    on_readback: impl FnOnce(&MemoryReadExecution),
) -> Result<(), LuminaireWriteFailure> {
    let address = dali_short_address(short_address).map_err(LuminaireWriteFailure::of)?;
    let header = read_header(controller, short_address, writes)?;
    let written = write_sessions(controller, address, header.lock, writes);
    if written.is_err_and(LuminaireWriteFailure::is_preempted) {
        return written;
    }
    let length = u16::from(header.last_offset) + 1;
    let readback = read_memory_bank(controller, short_address, BANK1, 0, length);
    on_readback(&readback);
    verdict(written, &readback, writes, header.lock)
}

#[cfg(test)]
mod tests;
