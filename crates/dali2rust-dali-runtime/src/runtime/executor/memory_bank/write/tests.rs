use dali2rust_contracts::msg::LuminaireNumbers as N;
use dali2rust_domain::dali::banks::luminaire_write::{
    number_patch, planned_writes, text_patch, LuminaireField,
};
use dali2rust_domain::dali::banks::part251::YEAR_OFFSET;

use super::*;
use crate::runtime::executor::helpers::PREEMPTED_MESSAGE;
use dali2rust_domain::dali::banks::part251::CONTENT_FORMAT_ID_OFFSET;

const FOUND_LOCK: u8 = 0x06;
const YEAR: u8 = 24;
const OLD_YEAR: u8 = 23;
const BANK_LAST: u8 = 0x77;
const BANK_LEN: usize = BANK_LAST as usize + 1;
const FOUND: Header = Header { lock: FOUND_LOCK, last_offset: BANK_LAST };
const LIGHT_OUTPUT: u32 = 0x01_02_03;

fn year() -> Vec<FieldWrite> {
    planned_writes(&number_patch(&[(N::PATCH_YEAR, u32::from(YEAR))]), None)
}

fn bank(lock: u8, year: u8) -> MemoryReadExecution {
    let mut bytes = vec![0u8; BANK_LEN];
    bytes[usize::from(BANK_LOCK_BYTE)] = lock;
    bytes[usize::from(YEAR_OFFSET)] = year;
    MemoryReadExecution { bytes, indicator_unread: false, error: None }
}

fn holding(writes: &[FieldWrite]) -> MemoryReadExecution {
    let mut read = bank(FOUND_LOCK, OLD_YEAR);
    for write in writes {
        let start = usize::from(write.field.offset);
        read.bytes[start..write.end()].copy_from_slice(&write.bytes);
    }
    read
}

fn contended() -> MemoryReadExecution {
    let error = SemanticDaliError::OperationFailed(BUS_CONTENDED);
    MemoryReadExecution { bytes: Vec::new(), indicator_unread: false, error: Some(error) }
}

fn failure(code: ErrorCode, subject: &'static str) -> LuminaireWriteFailure {
    LuminaireWriteFailure::new(code, subject)
}

fn failed(code: ErrorCode, subject: &'static str) -> Result<(), LuminaireWriteFailure> {
    Err(failure(code, subject))
}

fn stopped(code: ErrorCode, subject: &'static str) -> WireOutcome {
    WireOutcome { stopped: Some(failure(code, subject)), unrestored: None }
}

fn unrestored(code: ErrorCode, stopped: Option<LuminaireWriteFailure>) -> WireOutcome {
    WireOutcome { stopped, unrestored: Some(failure(code, RELOCK_SUBJECT)) }
}

const CLEAN: WireOutcome = WireOutcome { stopped: None, unrestored: None };

#[test]
fn a_bank_holding_every_byte_and_the_found_lock_proves_the_write() {
    assert_eq!(verdict(CLEAN, &bank(FOUND_LOCK, YEAR), &year(), FOUND), Ok(()));
}

#[test]
fn a_field_the_read_back_does_not_hold_fails_by_name() {
    let readback = bank(FOUND_LOCK, OLD_YEAR);
    assert_eq!(verdict(CLEAN, &readback, &year(), FOUND), failed(ErrorCode::VerifyFailed, "year"));
}

#[test]
fn a_lock_read_back_other_than_found_is_a_lock_not_restored_whatever_the_fields_say() {
    let readback = bank(BANK_UNLOCKED, YEAR);
    let not_restored = failed(ErrorCode::VerifyFailed, RELOCK_SUBJECT);
    assert_eq!(verdict(CLEAN, &readback, &year(), FOUND), not_restored, "a held field is no excuse");
    let wire = stopped(ErrorCode::VerifyUnanswered, "year");
    assert_eq!(verdict(wire, &bank(BANK_UNLOCKED, OLD_YEAR), &year(), FOUND), not_restored);
}

#[test]
fn the_read_back_is_the_truth_for_what_is_written() {
    let lost_answer = stopped(ErrorCode::VerifyUnanswered, "year");
    assert_eq!(verdict(lost_answer, &bank(FOUND_LOCK, YEAR), &year(), FOUND), Ok(()));
    let lost_restore = unrestored(ErrorCode::VerifyUnanswered, None);
    assert_eq!(
        verdict(lost_restore, &bank(FOUND_LOCK, YEAR), &year(), FOUND),
        Ok(()),
        "a restore the read-back finds in place landed"
    );
}

#[test]
fn where_the_wire_stopped_names_the_failure_when_the_read_back_does_not_prove_the_write() {
    let lock_shut = stopped(ErrorCode::VerifyFailed, LOCK_SUBJECT);
    let both = WireOutcome { unrestored: Some(failure(ErrorCode::VerifyUnanswered, RELOCK_SUBJECT)), ..lock_shut };
    let as_found = bank(FOUND_LOCK, OLD_YEAR);
    assert_eq!(verdict(lock_shut, &as_found, &year(), FOUND), failed(ErrorCode::VerifyFailed, LOCK_SUBJECT));
    assert_eq!(
        verdict(both, &as_found, &year(), FOUND),
        failed(ErrorCode::VerifyFailed, LOCK_SUBJECT),
        "the lock read back as found, so the lost restore landed and the open failure is the cause"
    );
}

#[test]
fn a_failed_restore_the_read_back_cannot_check_says_the_bank_may_be_open() {
    let open_failed = Some(failure(ErrorCode::VerifyFailed, LOCK_SUBJECT));
    let wire = unrestored(ErrorCode::VerifyContended, open_failed);
    assert_eq!(verdict(wire, &contended(), &year(), FOUND), failed(ErrorCode::VerifyContended, RELOCK_SUBJECT));
    let alone = unrestored(ErrorCode::VerifyUnanswered, None);
    assert_eq!(verdict(alone, &contended(), &year(), FOUND), failed(ErrorCode::VerifyUnanswered, RELOCK_SUBJECT));
}

#[test]
fn a_field_that_failed_on_the_wire_outranks_an_unread_bank() {
    let wire = stopped(ErrorCode::VerifyUnanswered, "year");
    assert_eq!(verdict(wire, &contended(), &year(), FOUND), failed(ErrorCode::VerifyUnanswered, "year"));
}

#[test]
fn an_unread_bank_proves_nothing_and_names_the_read_back() {
    let unproved = failed(ErrorCode::VerifyContended, READBACK_SUBJECT);
    assert_eq!(verdict(CLEAN, &contended(), &year(), FOUND), unproved);
}

#[test]
fn a_read_back_that_yields_names_the_read_back_not_a_field() {
    let error = SemanticDaliError::OperationFailed(PREEMPTED_MESSAGE);
    let yielded = MemoryReadExecution { bytes: Vec::new(), indicator_unread: false, error: Some(error) };
    let unproved = failed(ErrorCode::Preempted, READBACK_SUBJECT);
    assert_eq!(verdict(CLEAN, &yielded, &year(), FOUND), unproved, "every field went out; none is proved");
}

#[test]
fn a_three_byte_number_off_in_its_last_byte_fails_its_field() {
    let writes = planned_writes(&number_patch(&[(N::PATCH_NOMINAL_LIGHT_OUTPUT_LM, LIGHT_OUTPUT)]), None);
    let mut readback = holding(&writes);
    assert_eq!(verdict(CLEAN, &readback, &writes, FOUND), Ok(()));
    readback.bytes[usize::from(writes[0].field.last_offset())] ^= 0x01;
    let off = failed(ErrorCode::VerifyFailed, "nominal_light_output_lm");
    assert_eq!(verdict(CLEAN, &readback, &writes, FOUND), off);
}

#[test]
fn a_text_that_diverges_after_its_first_byte_fails_its_field() {
    let field = LuminaireField::named("luminaire_colour").expect("text field");
    let colour = text_patch(field, "warm").expect("printable");
    let writes = planned_writes(&N::default(), Some(&colour));
    let mut readback = holding(&writes);
    assert_eq!(verdict(CLEAN, &readback, &writes, FOUND), Ok(()));
    readback.bytes[usize::from(field.offset) + 2] = b'x';
    assert_eq!(verdict(CLEAN, &readback, &writes, FOUND), failed(ErrorCode::VerifyFailed, "luminaire_colour"));
}

#[test]
fn a_read_back_that_ends_before_the_last_offset_the_header_declared_proves_nothing() {
    let mut readback = bank(FOUND_LOCK, YEAR);
    readback.bytes.truncate(BANK_LEN - 1);
    let unproved = failed(ErrorCode::VerifyUnanswered, READBACK_SUBJECT);
    assert_eq!(verdict(CLEAN, &readback, &year(), FOUND), unproved, "every byte it did read holds the write");
    let lost_restore = unrestored(ErrorCode::VerifyUnanswered, None);
    assert_eq!(
        verdict(lost_restore, &readback, &year(), FOUND),
        failed(ErrorCode::VerifyUnanswered, RELOCK_SUBJECT),
        "a lost restore an incomplete read-back cannot check says the bank may be open"
    );
}

#[test]
fn a_header_is_whole_at_its_own_length_or_where_its_bank_ends() {
    let full = usize::from(BANK1_HEADER_LEN);
    let long_bank = [BANK_LAST];
    assert_eq!(header_len(&long_bank), full, "a long bank owes the whole header");
    let format_cut = CONTENT_FORMAT_ID_OFFSET - 1;
    assert_eq!(header_len(&[format_cut]), usize::from(CONTENT_FORMAT_ID_OFFSET), "a bank ending before its format ends there");
    assert_eq!(header_len(&[]), 1, "a header without its first byte declares nothing");
}

#[test]
fn a_read_before_any_write_reports_what_the_wire_did_not_a_verification() {
    let read_failure = |message| LuminaireWriteFailure::unread(SemanticDaliError::OperationFailed(message), HEADER_SUBJECT);
    assert_eq!(read_failure(MEMORY_POINTER_UNCONFIRMED), failure(ErrorCode::DeviceAbsent, HEADER_SUBJECT));
    assert_eq!(read_failure(READ_ENDED_EARLY), failure(ErrorCode::DeviceAbsent, HEADER_SUBJECT));
    assert_eq!(read_failure(BUS_CONTENDED), failure(ErrorCode::ReadContended, HEADER_SUBJECT));
    assert_eq!(read_failure(MEMORY_LATCH_LOST), failure(ErrorCode::ReadContended, HEADER_SUBJECT));
    assert_eq!(read_failure(PREEMPTED_MESSAGE), failure(ErrorCode::Preempted, HEADER_SUBJECT));
}
