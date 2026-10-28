use cucumber::{given, then};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::banks::part251::{LUMINAIRE_COLOUR, YEAR_OFFSET};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::Value;

use crate::steps::frames::{dt8_raw_query_frame, special_frame, standard_frame};
use crate::DaliWorld;

use super::memory_bank_scripts::{
    bank1_part251_bytes, script_memory_bank_read, READ_MEMORY_LOCATION_OPCODE,
};
use super::read_model::wait_for_physical_device;
use super::TEST_SHORT_ADDRESS;

const LUMINAIRE_BANK: u8 = 1;
const LOCK_BYTE_OFFSET: u8 = 0x02;
const UNLOCKED: u8 = 0x55;
const HEADER_LEN: usize = 0x13;
const UNIT_BYTES: usize = 3;
const UNITS_PER_SESSION: usize = 2;
const WRITTEN_YEAR: u8 = 24;
const VENDOR_FORMAT_ID: [u8; 2] = [0x00, 0x42];
const FORMAT_ID_OFFSET: usize = 0x11;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GearReply {
    Echo,
    Silent,
}

fn script_armed_pointer(mock: &MockDaliTransport, offset: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr1(LUMINAIRE_BANK)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(offset)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr1),
        Some(LUMINAIRE_BANK),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(offset),
    );
}

fn script_write_unit(mock: &MockDaliTransport, offset: u8, bytes: &[u8], reply: GearReply) {
    script_armed_pointer(mock, offset);
    let enable = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::EnableWriteMemory);
    mock.expect_forward_frame(enable);
    mock.expect_forward_frame(enable);
    for byte in bytes {
        let answer = (reply == GearReply::Echo).then_some(*byte);
        mock.expect_forward_frame_with_backward(
            special_frame(SpecialCommand::WriteMemoryLocation(*byte)),
            answer,
        );
    }
}

fn script_unlock_and_prove(mock: &MockDaliTransport) {
    script_write_unit(mock, LOCK_BYTE_OFFSET, &[UNLOCKED], GearReply::Echo);
    script_armed_pointer(mock, LOCK_BYTE_OFFSET);
    mock.expect_forward_frame_with_backward(
        dt8_raw_query_frame(TEST_SHORT_ADDRESS, READ_MEMORY_LOCATION_OPCODE),
        Some(UNLOCKED),
    );
}

fn units_of(offset: u8, bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    bytes
        .chunks(UNIT_BYTES)
        .enumerate()
        .map(|(index, chunk)| (offset + (index * UNIT_BYTES) as u8, chunk.to_vec()))
        .collect()
}

fn script_field_write(mock: &MockDaliTransport, lock: u8, offset: u8, bytes: &[u8], reply: GearReply) {
    for session in units_of(offset, bytes).chunks(UNITS_PER_SESSION) {
        script_unlock_and_prove(mock);
        for (unit_offset, unit) in session {
            script_write_unit(mock, *unit_offset, unit, reply);
            if reply == GearReply::Silent {
                break;
            }
        }
        script_write_unit(mock, LOCK_BYTE_OFFSET, &[lock], GearReply::Echo);
        if reply == GearReply::Silent {
            return;
        }
    }
}

fn script_luminaire_write(world: &mut DaliWorld, offset: u8, bytes: &[u8], kept: bool, reply: GearReply) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let before = bank1_part251_bytes();
    let lock = before[usize::from(LOCK_BYTE_OFFSET)];
    assert_ne!(lock, UNLOCKED, "the golden bank must start locked");
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &before[..HEADER_LEN]);
    script_field_write(&mock, lock, offset, bytes, reply);
    let mut after = before;
    if kept {
        let start = usize::from(offset);
        after[start..start + bytes.len()].copy_from_slice(bytes);
    }
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &after);
}

// PD-291
#[given("a memory-luminaire write script for short address 0 storing year 24 that the gear keeps")]
async fn given_year_write_kept(world: &mut DaliWorld) {
    script_luminaire_write(world, YEAR_OFFSET, &[WRITTEN_YEAR], true, GearReply::Echo);
}

// PD-293
#[given("a memory-luminaire write script for short address 0 storing year 24 that the gear reads back unchanged")]
async fn given_year_write_lost(world: &mut DaliWorld) {
    assert_ne!(bank1_part251_bytes()[usize::from(YEAR_OFFSET)], WRITTEN_YEAR);
    script_luminaire_write(world, YEAR_OFFSET, &[WRITTEN_YEAR], false, GearReply::Echo);
}

// PD-294
#[given("a memory-luminaire write script for short address 0 storing year 24 that the gear does not confirm")]
async fn given_year_write_unconfirmed(world: &mut DaliWorld) {
    script_luminaire_write(world, YEAR_OFFSET, &[WRITTEN_YEAR], false, GearReply::Silent);
}

// PD-292
#[given(regex = r#"^a memory-luminaire write script for short address 0 storing the colour "([^"]+)" that the gear keeps$"#)]
async fn given_colour_write_kept(world: &mut DaliWorld, colour: String) {
    let mut bytes = colour.into_bytes();
    bytes.push(0);
    script_luminaire_write(world, LUMINAIRE_COLOUR.offset, &bytes, true, GearReply::Echo);
}

// PD-299
#[given("a memory-luminaire write script for short address 0 whose bank 1 header now declares a vendor format")]
async fn given_vendor_header(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let mut header = bank1_part251_bytes()[..HEADER_LEN].to_vec();
    header[FORMAT_ID_OFFSET..FORMAT_ID_OFFSET + 2].copy_from_slice(&VENDOR_FORMAT_ID);
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &header);
}

// PD-291
#[then("adapter 0 physical device 0 eventually exposes luminaire year 24 with the rest of bank 1 intact")]
async fn then_year_written(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/memory_luminaire/year/value/value").and_then(Value::as_u64)
            == Some(u64::from(WRITTEN_YEAR))
    });
    let colour = json.pointer("/attributes/memory_luminaire/luminaire_colour/value");
    assert_eq!(colour.and_then(Value::as_str), Some("warm white"), "the read-back covers the whole bank");
    let ident = json
        .pointer("/attributes/memory_luminaire/luminaire_identification/value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(ident.starts_with("DALI2RUST BENCH LUMINAIRE"), "identification kept, got {ident:?}");
    let lock = json.pointer("/attributes/memory_profile/bank1_lock_byte/value").and_then(Value::as_u64);
    let golden_lock = bank1_part251_bytes()[usize::from(LOCK_BYTE_OFFSET)];
    assert_eq!(lock, Some(u64::from(golden_lock)), "the lock byte reads back as it was found");
}

// PD-292
#[then(regex = r#"^adapter 0 physical device 0 eventually exposes the luminaire colour "([^"]+)"$"#)]
async fn then_colour_written(world: &mut DaliWorld, colour: String) {
    wait_for_physical_device(world, |body| {
        body.pointer("/attributes/memory_luminaire/luminaire_colour/value").and_then(Value::as_str)
            == Some(colour.as_str())
    });
}
