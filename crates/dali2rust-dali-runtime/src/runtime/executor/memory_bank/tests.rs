use super::*;
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, setup_controller, short_address, short_raw_query_frame,
};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::commands::DaliCommand;

fn special_frame(cmd: SpecialCommand) -> u16 {
    DaliCommand::Special(cmd).to_forward_frame().raw()
}

fn standard_frame(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

fn expect_arm(
    mock: &MockDaliTransport,
    short: u8,
    bank: u8,
    offset: u8,
    answers: (Option<u8>, Option<u8>),
) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr1(bank)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(offset)));
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr1),
        answers.0,
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        answers.1,
    );
}

fn expect_prepare(mock: &MockDaliTransport, short: u8, bank: u8, offset: u8) {
    expect_arm(mock, short, bank, offset, (Some(bank), Some(offset)));
}

fn expect_final_position(mock: &MockDaliTransport, short: u8, offset: Option<u8>) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryContentDtr0),
        offset,
    );
}

fn chunk_boundaries(bank: u8, length: u16) -> Vec<u16> {
    let mut offsets = Vec::new();
    let mut done = 0u16;
    while done < length {
        if done > 0 {
            offsets.push(done);
        }
        done += chunk_len(bank, done, length - done, MEMORY_READ_CHUNK).max(1);
    }
    offsets
}

fn expect_chunk_boundary(mock: &MockDaliTransport, short: u8, bank: u8, offset: u16, length: u16) {
    if !chunk_boundaries(bank, length).contains(&offset) {
        return;
    }
    expect_final_position(mock, short, Some(offset.min(255) as u8));
}

fn expect_bank_read(mock: &MockDaliTransport, short: u8, bank: u8, values: &[Option<u8>]) {
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    expect_prepare(mock, short, bank, 0);
    let length = values.len() as u16;
    for (offset, value) in values.iter().enumerate() {
        expect_chunk_boundary(mock, short, bank, offset as u16, length);
        mock.expect_forward_frame_with_backward(read, *value);
        if value.is_none() && offset as u16 == MEMORY_RESERVED_OFFSET {
            expect_prepare(mock, short, bank, (offset + 1) as u8);
        }
    }
    expect_final_position(mock, short, Some(values.len() as u8));
}

fn expect_short_bank_read(
    mock: &MockDaliTransport,
    short: u8,
    bank: u8,
    values: &[u8],
    closing: Option<u8>,
) {
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    expect_prepare(mock, short, bank, 0);
    let length = values.len() as u16 + 1;
    for (offset, value) in values.iter().enumerate() {
        expect_chunk_boundary(mock, short, bank, offset as u16, length);
        mock.expect_forward_frame_with_backward(read, Some(*value));
    }
    let declined = values.len() as u8;
    expect_chunk_boundary(mock, short, bank, u16::from(declined), length);
    for attempt in 0..=MEMORY_READ_LOCATION_RETRIES {
        mock.expect_forward_frame_with_backward(read, None);
        if attempt < MEMORY_READ_LOCATION_RETRIES {
            expect_prepare(mock, short, bank, declined);
        }
    }
    expect_final_position(mock, short, closing);
}

#[test]
fn read_memory_bank_success_reads_full_range() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_prepare(&mock, short, 1, 2);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, Some(0x11));
    mock.expect_forward_frame_with_backward(read, Some(0x22));
    expect_final_position(&mock, short, Some(4));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 1, 2, 2);

    assert_eq!(execution.bytes, vec![0x11, 0x22]);
    assert_eq!(execution.error, None);
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_rejects_range_start_beyond_u8() {
    let mock = MockDaliTransport::new();
    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, 5, 0, 300, 1);
    assert_eq!(
        execution.error,
        Some(SemanticDaliError::Conflict("memory_range_out_of_bounds"))
    );
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_rejects_invalid_short_address() {
    let mock = MockDaliTransport::new();
    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, 200, 0, 0, 1);
    assert_eq!(
        execution.error,
        Some(SemanticDaliError::Conflict("invalid_short_address"))
    );
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_stops_on_dtr1_failure() {
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_send_error(special_frame(SpecialCommand::Dtr1(0)));
    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, 5, 0, 0, 1);
    assert!(execution.error.is_some());
    assert!(execution.bytes.is_empty());
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_stops_on_dtr0_failure() {
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr1(0)));
    mock.expect_forward_frame_send_error(special_frame(SpecialCommand::Dtr0(0)));
    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, 5, 0, 0, 1);
    assert!(execution.error.is_some());
    assert!(execution.bytes.is_empty());
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_skips_reserved_no_answer_and_repositions() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_prepare(&mock, short, 0, 0);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, Some(0x1C));
    mock.expect_forward_frame_with_backward(read, None);
    expect_prepare(&mock, short, 0, 2);
    mock.expect_forward_frame_with_backward(read, Some(0x01));
    expect_final_position(&mock, short, Some(3));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 0, 0, 3);

    assert_eq!(execution.bytes, vec![0x1C, 0xFF, 0x01]);
    assert_eq!(execution.error, None);
    assert_script_consumed(&transport);
}

#[test]
fn read_memory_bank_retries_transient_no_answer_for_data_offset() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_prepare(&mock, short, 1, 2);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, None);
    expect_prepare(&mock, short, 1, 2);
    mock.expect_forward_frame_with_backward(read, Some(0x33));
    expect_final_position(&mock, short, Some(3));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 1, 2, 1);

    assert_eq!(execution.bytes, vec![0x33]);
    assert_eq!(execution.error, None);
    assert_script_consumed(&transport);
}

#[test]
fn a_location_the_gear_proves_it_declines_ends_the_bank() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_prepare(&mock, short, 1, 2);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, None);
    expect_prepare(&mock, short, 1, 2);
    mock.expect_forward_frame_with_backward(read, None);
    expect_prepare(&mock, short, 1, 2);
    mock.expect_forward_frame_with_backward(read, None);
    expect_final_position(&mock, short, Some(2));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 1, 2, 1);

    assert!(execution.bytes.is_empty(), "the bank ended before this offset");
    assert_eq!(execution.error, None, "a declined location is an answer");
    assert_script_consumed(&transport);
}

#[test]
fn a_gear_that_stops_answering_mid_bank_is_not_read_as_an_end_of_bank() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_prepare(&mock, short, 1, 4);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, Some(0x11));
    mock.expect_forward_frame_with_backward(read, None);
    for _ in 0..=BANK_ARM_RETRIES {
        expect_arm(&mock, short, 1, 5, (None, None));
    }

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 1, 4, 4);

    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(
            MEMORY_POINTER_UNCONFIRMED
        ))
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_short_read_accepts_either_pointer_reading_of_the_declined_frame() {
    for (answer, accepted) in [(Some(3), true), (Some(4), true), (Some(5), false)] {
        let mock = MockDaliTransport::new();
        let short = 5;
        expect_short_bank_read(&mock, short, 1, &[0xAA, 0xBB, 0xCC], answer);

        let (transport, mut controller) = setup_controller(mock);
        let execution = read_memory_bank(&mut controller, short, 1, 0, 8);

        if accepted {
            assert_eq!(execution.error, None, "answer {answer:?} must be accepted");
            assert_eq!(execution.bytes, vec![0xAA, 0xBB, 0xCC]);
        } else {
            assert_eq!(
                execution.error,
                Some(SemanticDaliError::OperationFailed(MEMORY_MISALIGNED)),
                "answer {answer:?} is outside the accepted set"
            );
            assert!(execution.bytes.is_empty());
        }
        assert_script_consumed(&transport);
    }
}

#[test]
fn a_mangled_answer_mid_bank_realigns_instead_of_shifting() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_prepare(&mock, short, 0, 3);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_corrupted_in_window(read);
    expect_prepare(&mock, short, 0, 3);
    mock.expect_forward_frame_with_backward(read, Some(0x06));
    mock.expect_forward_frame_with_backward(read, Some(0x58));
    expect_final_position(&mock, short, Some(5));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 0, 3, 2);

    assert_eq!(
        execution.bytes,
        vec![0x06, 0x58],
        "the retried offset must return its own value, not its neighbour's"
    );
    assert_eq!(execution.error, None);
    assert_script_consumed(&transport);
}

#[test]
fn a_stale_bank_is_caught_by_the_pointer_readback_and_repaired() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_arm(&mock, short, 0, 0, (Some(1), Some(0)));
    expect_bank_read(&mock, short, 0, &[Some(0x03), None, Some(0x00)]);
    expect_bank_read(&mock, short, 0, &[Some(0x03), None, Some(0x00), Some(0x58)]);

    let (transport, mut controller) = setup_controller(mock);
    let mut published = Vec::new();
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::Identity,
        |plan, execution| published.push((plan.length, execution.bytes.clone())),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        published,
        vec![(4, vec![0x03, 0xFF, 0x00, 0x58])],
        "the plan must be sized from the bank the gear confirmed"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_pointer_that_never_confirms_publishes_nothing() {
    let mock = MockDaliTransport::new();
    let short = 5;
    for answers in [(Some(1), Some(0)), (Some(0), Some(3)), (None, None)] {
        expect_arm(&mock, short, 0, 0, answers);
    }

    let (transport, mut controller) = setup_controller(mock);
    let mut published = 0;
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::Identity,
        |_, _| published += 1,
    );

    assert_eq!(
        result,
        Err(SemanticDaliError::OperationFailed(
            MEMORY_POINTER_UNCONFIRMED
        ))
    );
    assert_eq!(published, 0, "nothing may be published from an unproved pointer");
    assert_script_consumed(&transport);
}

#[test]
fn a_pointer_that_overran_the_read_returns_nothing() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_prepare(&mock, short, 1, 0);
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    mock.expect_forward_frame_with_backward(read, Some(0x10));
    expect_final_position(&mock, short, Some(2));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 1, 0, 1);

    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(MEMORY_MISALIGNED))
    );
    assert!(
        execution.bytes.is_empty(),
        "misaligned bytes must not reach a caller"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_complete_bank0_read_is_published_and_last_bank_zero_ends_the_preset() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_bank_read(&mock, short, 0, &[Some(0x03), None, Some(0x00)]);
    expect_bank_read(&mock, short, 0, &[Some(0x03), None, Some(0x00), Some(0x58)]);

    let (transport, mut controller) = setup_controller(mock);
    let mut published = 0;
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::Identity,
        |_, _| published += 1,
    );

    assert_eq!(result, Ok(()));
    assert_eq!(published, 1, "bank 0 published; last_bank 0 means no bank 1");
    assert_script_consumed(&transport);
}

#[test]
fn a_bank_that_ends_early_publishes_the_short_read() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_bank_read(&mock, short, 0, &[Some(0x03), None, Some(0x00)]);
    expect_short_bank_read(&mock, short, 0, &[0x03, 0xFF, 0x00], Some(3));

    let (transport, mut controller) = setup_controller(mock);
    let mut published = Vec::new();
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::Identity,
        |plan, execution| published.push((plan.length, execution.bytes.clone())),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        published,
        vec![(4, vec![0x03, 0xFF, 0x00])],
        "the gear's own end of bank is a result, not a failure"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_header_that_over_reports_the_bank_length_no_longer_fails_the_sweep() {
    let mock = MockDaliTransport::new();
    let short = 1;
    let real: Vec<u8> = (0..29).map(|i| 0xA0 + i as u8).collect();
    expect_bank_read(&mock, short, 0, &[Some(31), None, Some(1)]);
    expect_short_bank_read(&mock, short, 0, &real, Some(29));
    expect_bank_read(&mock, short, 1, &[Some(0x02)]);
    expect_bank_read(&mock, short, 1, &[Some(0x02), None, Some(0x77)]);

    let (transport, mut controller) = setup_controller(mock);
    let mut published = Vec::new();
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::All,
        |plan, execution| published.push((plan.bank, plan.length, execution.bytes.len())),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(
        published,
        vec![(0, 32, 29), (1, 3, 3)],
        "the 29 bytes the gear answered are kept, and bank 1 is reached"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_last_bank_byte_that_over_reports_ends_at_the_first_absent_bank() {
    let mock = MockDaliTransport::new();
    let short = 1;
    expect_bank_read(&mock, short, 0, &[Some(0x02), None, Some(255)]);
    expect_bank_read(&mock, short, 0, &[Some(0x02), None, Some(255)]);
    expect_bank_read(&mock, short, 1, &[Some(0x01)]);
    expect_bank_read(&mock, short, 1, &[Some(0x01), None]);
    expect_short_bank_read(&mock, short, 2, &[], Some(0));

    let (transport, mut controller) = setup_controller(mock);
    let mut banks = Vec::new();
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::All,
        |plan, _| banks.push(plan.bank),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(banks, vec![0, 1], "255 banks are not walked; the gear ends it");
    assert_script_consumed(&transport);
}

#[test]
fn a_gear_that_declines_bank_zero_entirely_is_an_error() {
    let mock = MockDaliTransport::new();
    let short = 5;
    expect_short_bank_read(&mock, short, 0, &[], Some(0));

    let (transport, mut controller) = setup_controller(mock);
    let mut published = 0;
    let result = execute_memory_bank_preset(
        &mut controller,
        short,
        MemoryBankReadPreset::Identity,
        |_, _| published += 1,
    );

    assert_eq!(
        result,
        Err(SemanticDaliError::OperationFailed(
            "memory_bank_last_address_missing"
        ))
    );
    assert_eq!(published, 0);
    assert_script_consumed(&transport);
}

#[test]
fn a_reread_inside_a_latched_value_restarts_it_from_its_first_byte() {
    let mock = MockDaliTransport::new();
    let short = 3;
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    expect_prepare(&mock, short, 202, 0x05);
    mock.expect_forward_frame_with_backward(read, Some(0xAA));
    mock.expect_forward_frame_with_backward(read, Some(0xAA));
    mock.expect_forward_frame_with_backward(read, None);
    expect_prepare(&mock, short, 202, 0x05);
    for _ in 0..6 {
        mock.expect_forward_frame_with_backward(read, Some(0x01));
    }
    expect_final_position(&mock, short, Some(0x0B));

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 202, 0x05, 6);

    assert_eq!(
        execution.bytes,
        vec![0x01; 6],
        "every byte must come from ONE latch: the 0xAA pair is the discarded snapshot"
    );
    assert_eq!(execution.error, None);
    assert_script_consumed(&transport);
}

#[test]
fn a_latched_value_that_never_completes_fails_rather_than_stitching() {
    let mock = MockDaliTransport::new();
    let short = 3;
    let read = short_raw_query_frame(short_address(short), READ_MEMORY_LOCATION_OPCODE);
    expect_prepare(&mock, short, 202, 0x05);
    for _ in 0..=MEMORY_FIELD_RESTARTS {
        mock.expect_forward_frame_with_backward(read, Some(0xAA));
        mock.expect_forward_frame_with_backward(read, Some(0xAA));
        mock.expect_forward_frame_with_backward(read, None);
        expect_prepare(&mock, short, 202, 0x05);
    }

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_memory_bank(&mut controller, short, 202, 0x05, 6);

    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(MEMORY_LATCH_LOST))
    );
    assert_script_consumed(&transport);
}

#[test]
fn rom_banks_keep_the_flat_chunk_budget() {
    assert_eq!(chunk_len(0, 3, 24, MEMORY_READ_CHUNK), MEMORY_READ_CHUNK);
    assert_eq!(chunk_len(1, 3, 24, MEMORY_READ_CHUNK), MEMORY_READ_CHUNK);
    assert_eq!(
        chunk_len(202, 0x05, 11, MEMORY_READ_CHUNK),
        6,
        "a latched value wider than the budget goes in one chunk"
    );
}

#[test]
fn part251_numeric_values_are_never_split_across_chunks() {
    use dali2rust_domain::dali::banks::part251;

    let last = 0xC9u16;
    let mut boundaries = vec![];
    let mut offset = 0u16;
    while offset <= last {
        boundaries.push(offset);
        let taken = chunk_len(1, offset, last + 1 - offset, MEMORY_READ_CHUNK);
        assert!(taken > 0, "no progress at {offset:#04x}");
        offset += taken;
    }
    for field in part251::BANK_LUMINAIRE_INFO_MAP {
        let start = u16::from(field.offset);
        let width = field.width.bytes() as u16;
        if width == 1 {
            continue;
        }
        let crossed = boundaries
            .iter()
            .any(|b| *b > start && *b < start + width);
        assert!(!crossed, "field at {start:#04x} split across a chunk");
    }

    assert_eq!(
        chunk_len(1, u16::from(part251::LUMINAIRE_IDENTIFICATION.offset), 60, MEMORY_READ_CHUNK),
        MEMORY_READ_CHUNK
    );
}
