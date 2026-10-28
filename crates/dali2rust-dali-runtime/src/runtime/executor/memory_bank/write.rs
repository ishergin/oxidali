use dali2rust_contracts::msg::ErrorCode;
use dali2rust_domain::dali::banks::luminaire_write::{
    check_writable, Bank1Layout, Bank1WriteRefusal, FieldWrite, WriteStage, BANK1_HEADER_LEN,
    BANK_UNLOCKED,
};
use dali2rust_domain::dali::banks::{
    IdentityRead, BANK_LOCK_BYTE, BANK_LUMINAIRE_INFO, GTIN_OFFSET, IDENTITY_BANK, IDENTITY_WINDOW_LEN,
};
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
    dali_short_address, send_raw_query_once_response, send_special, send_special_response, send_standard_response,
    send_standard, SemanticDaliError, READ_MEMORY_LOCATION_OPCODE,
};

const BANK1: u8 = BANK_LUMINAIRE_INFO;

// IEC 62386-101 §9.2
const UNIT_BYTES: usize = 3;

const UNITS_PER_SESSION: usize = 2;

pub const LOCK_SUBJECT: &str = "bank1_lock_byte";

pub const READBACK_SUBJECT: &str = "bank1_readback";

pub const RELOCK_SUBJECT: &str = "bank1_lock_not_restored";

pub const IDENTITY_SUBJECT: &str = "device_identity";

pub const HEADER_SUBJECT: &str = "bank1_header";

const READ_ENDED_EARLY: &str = "memory_bank_read_ended_early";

pub const IDENTITY_CHANGED: &str = "device_identity_changed";

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

    // IEC 62386-102 §9.10.4
    fn unread(error: SemanticDaliError, subject: &'static str) -> Self {
        let code = match error.message() {
            MEMORY_POINTER_UNCONFIRMED | READ_ENDED_EARLY => ErrorCode::DeviceAbsent,
            BUS_CONTENDED | MEMORY_LATCH_LOST => ErrorCode::ReadContended,
            _ => error.code(),
        };
        Self::new(code, subject)
    }

    // IEC 62386-102 §9.10.5
    fn unproved(error: SemanticDaliError, subject: &'static str) -> Self {
        let code = match error.message() {
            MEMORY_POINTER_UNCONFIRMED | READ_ENDED_EARLY => ErrorCode::VerifyUnanswered,
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

fn units(stages: &[WriteStage]) -> Vec<Unit<'_>> {
    stages
        .iter()
        .flat_map(|stage| {
            stage.bytes.chunks(UNIT_BYTES).enumerate().map(move |(index, bytes)| Unit {
                subject: stage.field.name,
                offset: unit_offset(stage.offset, index),
                bytes,
            })
        })
        .collect()
}

fn unit_offset(stage_offset: u8, index: usize) -> u8 {
    let skipped = index.saturating_mul(UNIT_BYTES);
    u8::try_from(usize::from(stage_offset).saturating_add(skipped)).unwrap_or(u8::MAX)
}

#[derive(Debug, Clone, Copy)]
struct Stop {
    error: SemanticDaliError,
    reached: bool,
}

fn armed<C: DaliApplicationController, R>(
    controller: &mut C,
    address: DaliAddress,
    offset: u8,
    mut act: impl FnMut(&mut C) -> Result<R, SemanticDaliError>,
) -> Result<R, Stop> {
    let mut proof = PointerProof::Unanswered;
    let mut reached = false;
    for _ in 0..=BANK_ARM_RETRIES {
        let outcome = controller
            .unit(|c| arm_then(c, (address, offset), &mut reached, &mut act))
            .map_err(|error| Stop { error, reached })?;
        match outcome {
            Ok(done) => return Ok(done),
            Err(other) => proof = other,
        }
    }
    Err(Stop { error: proof.failure(MEMORY_MISALIGNED), reached })
}

fn arm_then<C: DaliApplicationController, R>(
    controller: &mut C,
    (address, offset): (DaliAddress, u8),
    reached: &mut bool,
    act: &mut impl FnMut(&mut C) -> Result<R, SemanticDaliError>,
) -> Result<Result<R, PointerProof>, SemanticDaliError> {
    send_special(controller, SpecialCommand::Dtr1(BANK1))?;
    send_special(controller, SpecialCommand::Dtr0(offset))?;
    match pointer_proof(controller, address, BANK1, (offset, false))? {
        PointerProof::Held => {
            *reached = true;
            act(controller).map(Ok)
        }
        other => Ok(Err(other)),
    }
}

struct UnitEcho {
    answers: Vec<DaliResponse>,
    pointer: DaliResponse,
}

// IEC 62386-102 §9.10.5
fn write_bytes(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    bytes: &[u8],
) -> Result<UnitEcho, SemanticDaliError> {
    send_standard(controller, address, StandardCommand::EnableWriteMemory)?;
    let mut answers = Vec::with_capacity(bytes.len());
    for byte in bytes {
        let answer = send_special_response(controller, SpecialCommand::WriteMemoryLocation(*byte))?;
        answers.push(answer);
        if answer != DaliResponse::Answer(*byte) {
            return Ok(UnitEcho { answers, pointer: DaliResponse::NoAnswer });
        }
    }
    let pointer = send_standard_response(controller, address, StandardCommand::QueryContentDtr0)?;
    Ok(UnitEcho { answers, pointer })
}

fn confirm(seen: &[DaliResponse], expected: &[u8], subject: &'static str) -> Result<(), LuminaireWriteFailure> {
    expected.iter().enumerate().try_for_each(|(index, byte)| {
        let answer = seen.get(index).copied().unwrap_or(DaliResponse::NoAnswer);
        verify_readback(answer, |value| value == *byte)
            .map(|_| ())
            .map_err(|error| LuminaireWriteFailure::unproved(error, subject))
    })
}

// IEC 62386-102 §9.10.5
fn confirm_unit(echo: &UnitEcho, unit: Unit<'_>) -> Result<(), LuminaireWriteFailure> {
    confirm(&echo.answers, unit.bytes, unit.subject)?;
    let advanced = u8::try_from(unit.bytes.len()).map_or(u8::MAX, |len| unit.offset.saturating_add(len));
    verify_readback(echo.pointer, |value| value == advanced)
        .map(|_| ())
        .map_err(|error| LuminaireWriteFailure::unproved(error, unit.subject))
}

fn write_unit(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    unit: Unit<'_>,
) -> Result<(), LuminaireWriteFailure> {
    let echo = armed(controller, address, unit.offset, |c| write_bytes(c, address, unit.bytes))
        .map_err(|stop| LuminaireWriteFailure::unproved(stop.error, unit.subject))?;
    confirm_unit(&echo, unit)
}

fn write_lock_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    value: u8,
) -> Result<(), LuminaireWriteFailure> {
    write_unit(controller, address, Unit { subject: LOCK_SUBJECT, offset: BANK_LOCK_BYTE, bytes: &[value] })
}

#[derive(Debug, Clone, Copy)]
struct Unopened {
    failure: LuminaireWriteFailure,
    reached: bool,
}

impl Unopened {
    const fn reached(failure: LuminaireWriteFailure) -> Self {
        Self { failure, reached: true }
    }

    // IEC 62386-102 §9.10.5
    fn of(stop: Stop) -> Self {
        let failure = if stop.reached { LuminaireWriteFailure::unproved } else { LuminaireWriteFailure::unread };
        Self { failure: failure(stop.error, LOCK_SUBJECT), reached: stop.reached }
    }
}

// IEC 62386-102 §9.10.2, Table 8
fn open_lock(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
) -> Result<(), Unopened> {
    let unit = Unit { subject: LOCK_SUBJECT, offset: BANK_LOCK_BYTE, bytes: &[BANK_UNLOCKED] };
    let echo = armed(controller, address, unit.offset, |c| write_bytes(c, address, unit.bytes)).map_err(Unopened::of)?;
    confirm_unit(&echo, unit).map_err(Unopened::reached)?;
    let (seen, contended) = armed(controller, address, unit.offset, |c| {
        send_raw_query_once_response(c, address, READ_MEMORY_LOCATION_OPCODE)
    })
    .map_err(|stop| Unopened::reached(LuminaireWriteFailure::unproved(stop.error, LOCK_SUBJECT)))?;
    let seen = if contended { DaliResponse::Violation } else { seen };
    confirm(&[seen], &[BANK_UNLOCKED], LOCK_SUBJECT).map_err(Unopened::reached)
}

fn write_units(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    session: &[Unit<'_>],
) -> Result<(), LuminaireWriteFailure> {
    session.iter().try_for_each(|unit| write_unit(controller, address, *unit))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WireOutcome {
    stopped: Option<LuminaireWriteFailure>,
    unrestored: Option<LuminaireWriteFailure>,
}

impl WireOutcome {
    fn stopped_at(written: Result<(), LuminaireWriteFailure>) -> Self {
        Self { stopped: written.err(), unrestored: None }
    }

    fn preempted(self) -> Option<LuminaireWriteFailure> {
        self.stopped.filter(|failure| failure.is_preempted())
    }

    fn named_after(self, unit: Option<&Unit<'_>>) -> Self {
        let stopped = match (self.preempted(), unit) {
            (Some(failure), Some(unit)) => Some(LuminaireWriteFailure::new(failure.code, unit.subject)),
            _ => self.stopped,
        };
        Self { stopped, ..self }
    }
}

fn write_session(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    lock: u8,
    session: &[Unit<'_>],
) -> WireOutcome {
    if lock == BANK_UNLOCKED {
        return WireOutcome::stopped_at(write_units(controller, address, session));
    }
    let written = match open_lock(controller, address) {
        Err(unopened) if !unopened.reached => return WireOutcome::stopped_at(Err(unopened.failure)),
        opened => opened.map_err(|unopened| unopened.failure),
    };
    let written = written.and_then(|()| write_units(controller, address, session));
    if written.is_err_and(LuminaireWriteFailure::is_preempted) {
        return WireOutcome::stopped_at(written);
    }
    let restored = write_lock_byte(controller, address, lock);
    let unrestored = restored.err().map(|failure| LuminaireWriteFailure::new(failure.code, RELOCK_SUBJECT));
    WireOutcome { stopped: written.err(), unrestored }
}

fn write_sessions(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    lock: u8,
    writes: &[FieldWrite],
) -> WireOutcome {
    let stages: Vec<WriteStage> = writes.iter().flat_map(FieldWrite::stages).collect();
    for session in units(&stages).chunks(UNITS_PER_SESSION) {
        let outcome = controller.step(|c| write_session(c, address, lock, session));
        if outcome != WireOutcome::default() {
            return outcome.named_after(session.first());
        }
    }
    WireOutcome::default()
}

// IEC 62386-102 §9.10.4
fn whole(read: &MemoryReadExecution, length: usize) -> Result<&[u8], SemanticDaliError> {
    match read.error {
        Some(error) => Err(error),
        None if read.bytes.len() < length => Err(SemanticDaliError::OperationFailed(READ_ENDED_EARLY)),
        None => Ok(&read.bytes),
    }
}

// IEC 62386-102 §9.10.6, Table 9
fn prove_identity(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    registered: IdentityRead,
) -> Result<(), LuminaireWriteFailure> {
    let identity = registered.known().map_err(|cause| LuminaireWriteFailure::new(ErrorCode::Conflict, cause))?;
    let window = read_memory_bank(controller, short_address, IDENTITY_BANK, u16::from(GTIN_OFFSET), IDENTITY_WINDOW_LEN);
    let unread = |error| LuminaireWriteFailure::unread(error, IDENTITY_SUBJECT);
    if let Some(error) = window.error {
        return Err(unread(error));
    }
    match IdentityRead::of_window(&window.bytes) {
        IdentityRead::Known(seen) if seen == identity => Ok(()),
        IdentityRead::Known(_) | IdentityRead::Unprovable => {
            Err(LuminaireWriteFailure::new(ErrorCode::Conflict, IDENTITY_CHANGED))
        }
        IdentityRead::Unread => Err(unread(SemanticDaliError::OperationFailed(READ_ENDED_EARLY))),
    }
}

// IEC 62386-102 §9.10.4
fn header_len(bytes: &[u8]) -> usize {
    let declared = |last: &u8| usize::from(*last) + 1;
    bytes.first().map_or(1, declared).min(usize::from(BANK1_HEADER_LEN))
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
    let read = read_memory_bank(controller, short_address, BANK1, 0, BANK1_HEADER_LEN);
    let header = whole(&read, header_len(&read.bytes))
        .map_err(|error| LuminaireWriteFailure::unread(error, HEADER_SUBJECT))?;
    let layout = Bank1Layout::from_header(header);
    for write in writes {
        check_writable(&layout, write.field).map_err(LuminaireWriteFailure::refused)?;
    }
    let lock = header.get(usize::from(BANK_LOCK_BYTE)).copied();
    match (lock, layout.last_offset) {
        (Some(lock), Some(last_offset)) => Ok(Header { lock, last_offset }),
        _ => Err(LuminaireWriteFailure::refused(Bank1WriteRefusal::NotPart251)),
    }
}

fn verdict(
    wire: WireOutcome,
    readback: &MemoryReadExecution,
    writes: &[FieldWrite],
    header: Header,
) -> Result<(), LuminaireWriteFailure> {
    let bank = match whole(readback, usize::from(header.last_offset) + 1) {
        Ok(bank) => bank,
        Err(error) => {
            let unread = LuminaireWriteFailure::unproved(error, READBACK_SUBJECT);
            return Err(wire.unrestored.or(wire.stopped).unwrap_or(unread));
        }
    };
    if bank.get(usize::from(BANK_LOCK_BYTE)) != Some(&header.lock) {
        return Err(LuminaireWriteFailure::new(ErrorCode::VerifyFailed, RELOCK_SUBJECT));
    }
    match writes.iter().find(|write| !write.held_by(bank)) {
        Some(write) => Err(wire.stopped.unwrap_or(LuminaireWriteFailure::new(ErrorCode::VerifyFailed, write.field.name))),
        None => Ok(()),
    }
}

// IEC 62386-102 §9.10.5
// DiiA 251 §9.2.3
pub fn write_memory_luminaire(
    controller: &mut impl DaliApplicationController,
    (short_address, identity): (u8, IdentityRead),
    writes: &[FieldWrite],
    on_readback: impl FnOnce(&MemoryReadExecution),
) -> Result<(), LuminaireWriteFailure> {
    let address = dali_short_address(short_address).map_err(LuminaireWriteFailure::of)?;
    prove_identity(controller, short_address, identity)?;
    let header = read_header(controller, short_address, writes)?;
    let wire = write_sessions(controller, address, header.lock, writes);
    if let Some(preempted) = wire.preempted() {
        return Err(preempted);
    }
    let length = u16::from(header.last_offset) + 1;
    let readback = read_memory_bank(controller, short_address, BANK1, 0, length);
    on_readback(&readback);
    verdict(wire, &readback, writes, header)
}

#[cfg(test)]
mod tests;
