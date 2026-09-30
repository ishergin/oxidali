use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_gear_model::luminaire::LuminaireBank;

use crate::steps::frames::{dt8_raw_query_frame, special_frame, standard_frame};

use super::TEST_SHORT_ADDRESS;

pub(super) const READ_MEMORY_LOCATION_OPCODE: u8 = 0xC5;

pub(super) const BANK0_BYTES: [u8; 29] = [
    0x1C, 0x00, 0x01, 0x00, 0x9D, 0xAD, 0xA2, 0x1B, 0x43, 0x01, 0x00, 0x26, 0x01, 0x06, 0xFA,
    0x9E, 0x78, 0x7D, 0x9B, 0x01, 0x00, 0x08, 0x08, 0xFF, 0x00, 0x01, 0x00,
    0x02,
    0b0000_0101,
];

pub(super) const BANK0_BUS_UNIT_CONFIGURATION: u64 = 0x02;

pub(super) const BANK0_IMPLEMENTED_PARTS_RAW: u64 = 0b0000_0101;

pub(super) const BANK0_IMPLEMENTED_PARTS: [u16; 2] = [150, 152];

pub(super) const SHORT_BANK0_BYTES: usize = 20;

pub(super) const BANK1_BYTES: [u8; 17] = [
    0x10, 0x00, 0x06, 0x58, 0x23, 0x32, 0xA8, 0xFC, 0xFF, 0xF9, 0x00, 0x00, 0x00, 0xE6, 0xFF,
    0x9B, 0x01,
];

pub(super) fn bank1_part251_bytes() -> Vec<u8> {
    let bank = LuminaireBank::new(
        PART251_SEED,
        dali2rust_domain::dali::banks::part251::LuminaireFormat::V3,
        false,
    );
    let mut bytes = BANK1_BYTES.to_vec();
    bytes[0] = bank.last_accessible();
    for offset in (BANK1_BYTES.len() as u8)..=bank.last_accessible() {
        bytes.push(bank.location(offset).unwrap_or(0));
    }
    bytes
}

const PART251_SEED: u32 = 1;

pub(super) const BANK0_GTIN: u64 = 0x009D_ADA2_1B43;

pub(super) const BANK1_OEM_GTIN: u64 = 0x5823_32A8_FCFF;

pub(super) const BANK1_OEM_ID: u64 = 0xF900_0000_E6FF_9B01;

fn script_memory_pointer_arm(mock: &MockDaliTransport, bank: u8, offset: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr1(bank)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(offset)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr1),
        Some(bank),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(offset),
    );
}

fn script_memory_pointer_check(mock: &MockDaliTransport, offset: u8) {
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(offset),
    );
}

const MEMORY_READ_CHUNK: usize = 5;

fn chunk_starts(bank: u8, total: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0u16;
    let total = total as u16;
    while offset < total {
        starts.push(usize::from(offset));
        let taken = dali2rust_domain::dali::banks::chunk_len(
            bank,
            offset,
            total - offset,
            MEMORY_READ_CHUNK as u16,
        );
        offset += taken.max(1);
    }
    starts
}

fn script_chunk_boundary(mock: &MockDaliTransport, starts: &[usize], offset: usize) {
    if offset == 0 || !starts.contains(&offset) {
        return;
    }
    script_memory_pointer_check(mock, offset.min(255) as u8);
}

pub(super) fn script_memory_bank_read(mock: &MockDaliTransport, bank: u8, bytes: &[u8]) {
    let read = dt8_raw_query_frame(TEST_SHORT_ADDRESS, READ_MEMORY_LOCATION_OPCODE);
    script_memory_pointer_arm(mock, bank, 0);
    let starts = chunk_starts(bank, bytes.len());
    for (offset, byte) in bytes.iter().enumerate() {
        script_chunk_boundary(mock, &starts, offset);
        if offset == 1 {
            mock.expect_forward_frame_with_backward(read, None);
            script_memory_pointer_arm(mock, bank, 2);
            continue;
        }
        mock.expect_forward_frame_with_backward(read, Some(*byte));
    }
    script_memory_pointer_check(mock, bytes.len() as u8);
}

pub(super) fn script_memory_bank_short_read(mock: &MockDaliTransport, bank: u8, bytes: &[u8]) {
    const LOCATION_RETRIES: usize = 2;
    let read = dt8_raw_query_frame(TEST_SHORT_ADDRESS, READ_MEMORY_LOCATION_OPCODE);
    script_memory_pointer_arm(mock, bank, 0);
    let starts = chunk_starts(bank, usize::from(BANK0_BYTES[0]) + 1);
    for (offset, byte) in bytes.iter().enumerate() {
        script_chunk_boundary(mock, &starts, offset);
        if offset == 1 {
            mock.expect_forward_frame_with_backward(read, None);
            script_memory_pointer_arm(mock, bank, 2);
            continue;
        }
        mock.expect_forward_frame_with_backward(read, Some(*byte));
    }
    let declined = bytes.len() as u8;
    script_chunk_boundary(mock, &starts, usize::from(declined));
    for attempt in 0..=LOCATION_RETRIES {
        mock.expect_forward_frame_with_backward(read, None);
        if attempt < LOCATION_RETRIES {
            script_memory_pointer_arm(mock, bank, declined);
        }
    }
    script_memory_pointer_check(mock, declined);
}
