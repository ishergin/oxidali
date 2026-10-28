use cucumber::{given, then};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::banks::luminaire_write::TEXT_TERMINATOR;
use dali2rust_domain::dali::banks::part251::{LUMINAIRE_COLOUR, YEAR_OFFSET};
use dali2rust_domain::dali::banks::{GTIN_OFFSET, IDENTITY_WINDOW_LEN};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::Value;

use crate::steps::frames::{dt8_raw_query_frame, special_frame, standard_frame};
use crate::DaliWorld;

use super::attribute_read_scripts::{script_attribute_read_identity_queries, ScriptedReply};
use super::memory_bank_scripts::{
    bank1_part251_bytes, script_memory_bank_read, script_memory_bank_short_read,
    script_memory_bank_window, BANK0_BYTES, READ_MEMORY_LOCATION_OPCODE,
};
use super::read_model::wait_for_physical_device;
use super::TEST_SHORT_ADDRESS;

const IDENTITY_BANK: u8 = 0;
const LUMINAIRE_BANK: u8 = 1;
const LOCK_BYTE_OFFSET: u8 = 0x02;
const UNLOCKED: u8 = 0x55;
const HEADER_LEN: usize = 0x13;
const UNIT_BYTES: usize = 3;
const UNITS_PER_SESSION: usize = 2;
const WRITTEN_YEAR: u8 = 24;
const VENDOR_FORMAT_ID: [u8; 2] = [0x00, 0x42];
const FORMAT_ID_OFFSET: usize = 0x11;
const BANK_META_LEN: usize = 3;
const PHYSICAL_MINIMUM: u8 = 0x01;
const LOGICAL_GEAR_INDEX_OFFSET: usize = 0x1A;
const IDENTIFICATION_NUMBER: std::ops::RangeInclusive<usize> = 0x0B..=0x12;
const MASK: u8 = 0xFF;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GearReply {
    Echo,
    Silent,
}

fn identity_window() -> Vec<u8> {
    let start = usize::from(GTIN_OFFSET);
    BANK0_BYTES[start..start + usize::from(IDENTITY_WINDOW_LEN)].to_vec()
}

fn script_identity(mock: &MockDaliTransport, window: &[u8]) {
    script_memory_bank_window(mock, IDENTITY_BANK, GTIN_OFFSET, window);
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
        if reply == GearReply::Silent {
            return;
        }
    }
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(offset + bytes.len() as u8),
    );
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

fn text_units(offset: u8, stored: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let (head, tail) = stored.split_first().expect("a text writes at least its terminator");
    let mut units = vec![(offset, vec![TEXT_TERMINATOR])];
    units.extend(units_of(offset + 1, tail));
    units.push((offset, vec![*head]));
    units
}

fn script_field_write(mock: &MockDaliTransport, lock: u8, units: &[(u8, Vec<u8>)], reply: GearReply) {
    for session in units.chunks(UNITS_PER_SESSION) {
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

fn script_luminaire_write(
    world: &mut DaliWorld,
    (offset, bytes): (u8, &[u8]),
    units: &[(u8, Vec<u8>)],
    kept: bool,
    reply: GearReply,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let before = bank1_part251_bytes();
    let lock = before[usize::from(LOCK_BYTE_OFFSET)];
    assert_ne!(lock, UNLOCKED, "the golden bank must start locked");
    script_identity(&mock, &identity_window());
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &before[..HEADER_LEN]);
    script_field_write(&mock, lock, units, reply);
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
    let year = (YEAR_OFFSET, [WRITTEN_YEAR].as_slice());
    script_luminaire_write(world, year, &units_of(YEAR_OFFSET, year.1), true, GearReply::Echo);
}

// PD-293
#[given("a memory-luminaire write script for short address 0 storing year 24 that the gear reads back unchanged")]
async fn given_year_write_lost(world: &mut DaliWorld) {
    assert_ne!(bank1_part251_bytes()[usize::from(YEAR_OFFSET)], WRITTEN_YEAR);
    let year = (YEAR_OFFSET, [WRITTEN_YEAR].as_slice());
    script_luminaire_write(world, year, &units_of(YEAR_OFFSET, year.1), false, GearReply::Echo);
}

// PD-294
#[given("a memory-luminaire write script for short address 0 storing year 24 that the gear does not confirm")]
async fn given_year_write_unconfirmed(world: &mut DaliWorld) {
    let year = (YEAR_OFFSET, [WRITTEN_YEAR].as_slice());
    script_luminaire_write(world, year, &units_of(YEAR_OFFSET, year.1), false, GearReply::Silent);
}

// PD-292
#[given(regex = r#"^a memory-luminaire write script for short address 0 storing the colour "([^"]+)" that the gear keeps$"#)]
async fn given_colour_write_kept(world: &mut DaliWorld, colour: String) {
    let mut bytes = colour.into_bytes();
    bytes.push(0);
    let units = text_units(LUMINAIRE_COLOUR.offset, &bytes);
    script_luminaire_write(world, (LUMINAIRE_COLOUR.offset, &bytes), &units, true, GearReply::Echo);
}

// PD-299
#[given("a memory-luminaire write script for short address 0 whose bank 1 header now declares a vendor format")]
async fn given_vendor_header(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_identity(&mock, &identity_window());
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

// PD-300
#[given("a memory-luminaire write script for short address 0 whose bank 0 now holds another identification number")]
async fn given_replaced_gear(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let mut window = identity_window();
    let last_number_byte = *IDENTIFICATION_NUMBER.end() - usize::from(GTIN_OFFSET);
    window[last_number_byte] ^= 0x01;
    script_identity(&mock, &window);
}

fn script_part251_attribute_read(world: &mut DaliWorld, bank0: impl FnOnce(&MockDaliTransport)) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let reply = ScriptedReply { value: Some(PHYSICAL_MINIMUM), contended: false };
    script_attribute_read_identity_queries(&mock, &[reply]);
    script_memory_bank_read(&mock, IDENTITY_BANK, &BANK0_BYTES[..BANK_META_LEN]);
    bank0(&mock);
    let bank1 = bank1_part251_bytes();
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &bank1[..1]);
    script_memory_bank_read(&mock, LUMINAIRE_BANK, &bank1);
}

// PD-301
#[given("a golden attribute-read script with DiiA Part 251 luminaire data whose bank 0 stops before its logical unit index for short address 0")]
async fn given_attribute_read_identity_cut(world: &mut DaliWorld) {
    script_part251_attribute_read(world, |mock| {
        script_memory_bank_short_read(mock, IDENTITY_BANK, &BANK0_BYTES[..LOGICAL_GEAR_INDEX_OFFSET]);
    });
}

// PD-302
#[given("a golden attribute-read script with DiiA Part 251 luminaire data whose bank 0 identification number is unprogrammed for short address 0")]
async fn given_attribute_read_identity_unprogrammed(world: &mut DaliWorld) {
    let mut bank0 = BANK0_BYTES;
    bank0[IDENTIFICATION_NUMBER].fill(MASK);
    script_part251_attribute_read(world, |mock| script_memory_bank_read(mock, IDENTITY_BANK, &bank0));
}
