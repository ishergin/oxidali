use dali2rust_contracts::msg::LuminaireNumbers as N;
use dali2rust_domain::dali::banks::luminaire_write::{number_patch, planned_writes};
use dali2rust_domain::dali::banks::part251::YEAR_OFFSET;

use super::*;

const FOUND_LOCK: u8 = 0x06;
const YEAR: u8 = 24;
const OLD_YEAR: u8 = 23;
const BANK_LEN: usize = 0x78;

fn year() -> Vec<FieldWrite> {
    planned_writes(&number_patch(&[(N::PATCH_YEAR, u32::from(YEAR))]), None)
}

fn bank(lock: u8, year: u8) -> MemoryReadExecution {
    let mut bytes = vec![0u8; BANK_LEN];
    bytes[usize::from(BANK_LOCK_BYTE)] = lock;
    bytes[usize::from(YEAR_OFFSET)] = year;
    MemoryReadExecution { bytes, indicator_unread: false, error: None }
}

fn contended() -> MemoryReadExecution {
    let error = SemanticDaliError::OperationFailed(BUS_CONTENDED);
    MemoryReadExecution { bytes: Vec::new(), indicator_unread: false, error: Some(error) }
}

fn failed(code: ErrorCode, subject: &'static str) -> Result<(), LuminaireWriteFailure> {
    Err(LuminaireWriteFailure::new(code, subject))
}

#[test]
fn a_bank_holding_every_byte_and_the_found_lock_proves_the_write() {
    assert_eq!(verdict(Ok(()), &bank(FOUND_LOCK, YEAR), &year(), FOUND_LOCK), Ok(()));
}

#[test]
fn a_field_the_read_back_does_not_hold_fails_by_name() {
    let readback = bank(FOUND_LOCK, OLD_YEAR);
    assert_eq!(verdict(Ok(()), &readback, &year(), FOUND_LOCK), failed(ErrorCode::VerifyFailed, "year"));
}

#[test]
fn a_lock_left_other_than_found_outranks_every_field() {
    let readback = bank(BANK_UNLOCKED, OLD_YEAR);
    let written = failed(ErrorCode::VerifyUnanswered, "year");
    let lock_lost = failed(ErrorCode::VerifyFailed, LOCK_SUBJECT);
    assert_eq!(verdict(written, &readback, &year(), FOUND_LOCK), lock_lost);
    assert_eq!(verdict(Ok(()), &readback, &year(), FOUND_LOCK), lock_lost, "a held field is no excuse");
}

#[test]
fn a_lock_that_failed_on_the_wire_outranks_the_read_back() {
    let written = failed(ErrorCode::VerifyContended, LOCK_SUBJECT);
    assert_eq!(verdict(written, &bank(BANK_UNLOCKED, OLD_YEAR), &year(), FOUND_LOCK), written);
    assert_eq!(verdict(written, &contended(), &year(), FOUND_LOCK), written);
}

#[test]
fn a_field_that_failed_on_the_wire_outranks_an_unread_bank() {
    let written = failed(ErrorCode::VerifyUnanswered, "year");
    assert_eq!(verdict(written, &contended(), &year(), FOUND_LOCK), written);
}

#[test]
fn an_unread_bank_proves_nothing_and_says_why() {
    let unproved = failed(ErrorCode::VerifyContended, "year");
    assert_eq!(verdict(Ok(()), &contended(), &year(), FOUND_LOCK), unproved);
}
