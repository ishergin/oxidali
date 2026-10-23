use super::*;
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, expect_narrow_colour_value_sample, expect_wide_colour_value_sample,
    priorities_of, setup_controller, short_address,
};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_contracts::msg::{CompactErrorPayload, ErrorCode};
use dali2rust_domain::dali::commands::DaliCommand;
use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_TC;
use dali2rust_domain::dali::ses::DaliPriority;
use std::sync::{Arc, Mutex};

const SHORT: u8 = 9;
const LEVEL: u8 = 120;
const OLD_LEVEL: u8 = 254;
const STORED_MIREK: (u8, u8) = (0x01, 0x2C);
const STRAY_MIREK: (u8, u8) = (0x00, 0xDE);
const MASK_WORD_ANSWER: (u8, u8) = (0xFF, 0xFF);
const LEVEL_STORES: [StoredLevel; 2] = [POWER_ON, SYSTEM_FAILURE];

fn standard_frame(command: StandardCommand) -> u16 {
    DaliCommand::Standard { address: short_address(SHORT), command }.to_forward_frame().raw()
}

fn dt8_frame(command: Dt8Command) -> u16 {
    DaliCommand::Extended { address: short_address(SHORT), command: ExtendedCommand::Dt8(command) }
        .to_forward_frame()
        .raw()
}

fn special_frame(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

fn expect_dt8_command(mock: &MockDaliTransport, command: Dt8Command) {
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_frame(command));
}

fn expect_violated_sample(mock: &MockDaliTransport, selector: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(selector)));
    mock.expect_forward_frame_with_backward(standard_frame(StandardCommand::QueryContentDtr0), Some(selector));
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(DT8_DEVICE_TYPE)));
    mock.expect_forward_frame_corrupted_in_window(dt8_frame(Dt8Command::QueryColourValue));
}

fn expect_temporary_type(mock: &MockDaliTransport, answer: Option<u8>) {
    expect_narrow_colour_value_sample(mock, SHORT, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE, answer);
}

fn expect_stored_type(mock: &MockDaliTransport, stored: StoredLevel, answer: u8) {
    mock.expect_forward_frame_with_backward(standard_frame(stored.query), Some(OLD_LEVEL));
    expect_narrow_colour_value_sample(mock, SHORT, DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE, Some(answer));
}

fn expect_tc_proof(mock: &MockDaliTransport, temporary_tc: (u8, u8)) {
    expect_temporary_type(mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_wide_colour_value_sample(mock, SHORT, COLOUR_VALUE_TEMPORARY_TC, temporary_tc);
}

fn expect_stored_tc_copied(mock: &MockDaliTransport, stored: StoredLevel, temporary_tc: (u8, u8)) {
    expect_stored_type(mock, stored, COLOUR_TYPE_BYTE_TC);
    expect_wide_colour_value_sample(mock, SHORT, DT8_COLOUR_VALUE_REPORT_TC, STORED_MIREK);
    expect_dt8_command(mock, Dt8Command::CopyReportToTemporary);
    expect_tc_proof(mock, temporary_tc);
}

fn expect_store(mock: &MockDaliTransport, stored: StoredLevel) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(LEVEL)));
    mock.expect_forward_frame(standard_frame(stored.store));
    mock.expect_forward_frame(standard_frame(stored.store));
    mock.expect_forward_frame_with_backward(standard_frame(stored.query), Some(LEVEL));
}

type Written = (WriteAttributesExecution, Arc<Mutex<MockDaliTransport>>);

fn colour_gear() -> DeviceTypeSet {
    [DT8_DEVICE_TYPE].into_iter().collect()
}

fn led_gear() -> DeviceTypeSet {
    [DeviceType::Led.code()].into_iter().collect()
}

fn write_level_on(mock: MockDaliTransport, stored: StoredLevel, supported: Option<DeviceTypeSet>) -> Written {
    let (power_on, failure) = if stored.store == POWER_ON.store { (Some(LEVEL), None) } else { (None, Some(LEVEL)) };
    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, SHORT, None, None, power_on, failure, None, (None, None), (None, None), None, supported,
    );
    (execution, transport)
}

fn write_level(mock: MockDaliTransport, stored: StoredLevel) -> Written {
    write_level_on(mock, stored, Some(colour_gear()))
}

fn assert_level_confirmed(execution: &WriteAttributesExecution, stored: StoredLevel) {
    assert_eq!(execution.error, None);
    let confirmed = if stored.store == POWER_ON.store {
        execution.confirmed.power_on_level
    } else {
        execution.confirmed.system_failure_level
    };
    assert_eq!(confirmed, Some(LEVEL));
}

// IEC 62386-102 §9.18
#[test]
fn a_gear_known_without_dt8_is_written_without_a_probe() {
    for stored in LEVEL_STORES {
        let mock = MockDaliTransport::new();
        expect_store(&mock, stored);

        let (execution, transport) = write_level_on(mock, stored, Some(led_gear()));

        assert_level_confirmed(&execution, stored);
        assert_script_consumed(&transport);
    }
}

#[test]
fn mask_temporaries_store_the_level_alone_after_the_probe() {
    for stored in LEVEL_STORES {
        let mock = MockDaliTransport::new();
        expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_MASK));
        expect_store(&mock, stored);

        let (execution, transport) = write_level(mock, stored);

        assert_level_confirmed(&execution, stored);
        assert_script_consumed(&transport);
    }
}

// IEC 62386-102 §3.13
#[test]
fn a_probe_that_stays_silent_fails_the_write_before_anything_is_stored() {
    for (types, outcome) in [(Some(colour_gear()), LEVEL_COLOUR_UNANSWERED), (None, DEVICE_TYPES_UNKNOWN)] {
        let mock = MockDaliTransport::new();
        for _ in 0..COLOUR_VALUE_ASKS {
            expect_temporary_type(&mock, None);
        }

        let (execution, transport) = write_level_on(mock, POWER_ON, types);

        assert_eq!(
            execution.error,
            Some(SemanticDaliError::OperationFailed(outcome)),
            "{types:?}: silence proves no colour absent, and with no types known it names the remedy"
        );
        assert_eq!(execution.confirmed.power_on_level, None);
        assert_script_consumed(&transport);
    }
}

// IEC 62386-101 §8.2.5
#[test]
fn a_probe_answered_by_a_violation_is_contended_not_silent() {
    let mock = MockDaliTransport::new();
    for _ in 0..COLOUR_VALUE_ASKS {
        expect_violated_sample(&mock, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE);
    }

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(LEVEL_COLOUR_CONTENDED)));
    assert_script_consumed(&transport);
}

// IEC 62386-209 §9.12.7, §11.2.2, §11.3.2
#[test]
fn a_charged_temporary_is_replaced_by_the_stored_colour_inside_one_transaction() {
    for stored in LEVEL_STORES {
        let mock = MockDaliTransport::new();
        expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
        expect_stored_tc_copied(&mock, stored, STORED_MIREK);
        expect_store(&mock, stored);

        let (execution, transport) = write_level(mock, stored);

        assert_level_confirmed(&execution, stored);
        let priorities = priorities_of(&transport.lock().expect("mock lock").sent_frame_settle_us());
        assert!(
            priorities[1..].iter().all(|p| *p == DaliPriority::Transaction),
            "probe, recharge, store and read-back are one transaction: {priorities:?}"
        );
        assert_script_consumed(&transport);
    }
}

// IEC 62386-209 §9.2.1, §9.12.4, Table 8
#[test]
fn a_stored_mask_colour_is_kept_by_charging_mask_of_the_temporary_type() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_type(&mock, POWER_ON, COLOUR_TYPE_BYTE_MASK);
    for dtr in [SpecialCommand::Dtr0(0xFF), SpecialCommand::Dtr1(0xFF)] {
        mock.expect_forward_frame(special_frame(dtr));
    }
    for query in [StandardCommand::QueryContentDtr0, StandardCommand::QueryContentDtr1] {
        mock.expect_forward_frame_with_backward(standard_frame(query), Some(0xFF));
    }
    expect_dt8_command(&mock, Dt8Command::SetTemporaryColourTemperature);
    expect_tc_proof(&mock, MASK_WORD_ANSWER);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §3.13
#[test]
fn a_level_query_that_loads_no_report_names_silence_and_a_violation_apart() {
    for (violated, outcome) in [(false, LEVEL_COLOUR_UNANSWERED), (true, LEVEL_COLOUR_CONTENDED)] {
        let mock = MockDaliTransport::new();
        expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
        let query = standard_frame(POWER_ON.query);
        if violated {
            mock.expect_forward_frame_corrupted_in_window(query);
        } else {
            mock.expect_forward_frame_with_backward(query, None);
        }

        let (execution, transport) = write_level(mock, POWER_ON);

        assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(outcome)));
        assert_eq!(execution.confirmed.power_on_level, None);
        assert_script_consumed(&transport);
    }
}

#[test]
fn a_report_type_that_stays_silent_stores_nothing() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    mock.expect_forward_frame_with_backward(standard_frame(POWER_ON.query), Some(OLD_LEVEL));
    for _ in 0..COLOUR_VALUE_ASKS {
        expect_narrow_colour_value_sample(&mock, SHORT, DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE, None);
    }

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNANSWERED)));
    assert_eq!(execution.confirmed.power_on_level, None);
    assert_script_consumed(&transport);
}

// IEC 62386-209 §11.3.4.1, Table 8, Table 11
#[test]
fn a_stored_mask_colour_under_a_primary_n_temporary_masks_all_six_primaries() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_PRIMARY_N));
    expect_stored_type(&mock, POWER_ON, COLOUR_TYPE_BYTE_MASK);
    for primary in 0..PRIMARY_N_COUNT {
        let operands = [DT8_COLOUR_VALUE_LEVEL_MASK, DT8_COLOUR_VALUE_LEVEL_MASK, primary];
        let dtrs = [SpecialCommand::Dtr0(operands[0]), SpecialCommand::Dtr1(operands[1]), SpecialCommand::Dtr2(primary)];
        for dtr in dtrs {
            mock.expect_forward_frame(special_frame(dtr));
        }
        let queries = [StandardCommand::QueryContentDtr0, StandardCommand::QueryContentDtr1, StandardCommand::QueryContentDtr2];
        for (query, operand) in queries.into_iter().zip(operands) {
            mock.expect_forward_frame_with_backward(standard_frame(query), Some(operand));
        }
        expect_dt8_command(&mock, Dt8Command::SetTemporaryPrimaryNDimLevel);
    }
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_PRIMARY_N));
    for primary in [COLOUR_VALUE_TEMPORARY_PRIMARY_N_0, COLOUR_VALUE_TEMPORARY_PRIMARY_N_5] {
        expect_wide_colour_value_sample(&mock, SHORT, primary, MASK_WORD_ANSWER);
    }
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

#[test]
fn a_collision_on_the_recharge_reruns_the_unit_from_the_probe() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_type(&mock, POWER_ON, COLOUR_TYPE_BYTE_TC);
    expect_wide_colour_value_sample(&mock, SHORT, DT8_COLOUR_VALUE_REPORT_TC, STORED_MIREK);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_collision(dt8_frame(Dt8Command::CopyReportToTemporary));
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_tc_copied(&mock, POWER_ON, STORED_MIREK);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

// IEC 62386-209 §9.12.7
#[test]
fn a_copy_the_gear_missed_is_caught_before_the_store_and_charged_again() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_tc_copied(&mock, POWER_ON, STRAY_MIREK);
    expect_stored_tc_copied(&mock, POWER_ON, STORED_MIREK);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

#[test]
fn a_temporary_that_never_holds_the_stored_colour_stores_nothing() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    for _ in 0..=RECHARGE_RETRIES {
        expect_stored_tc_copied(&mock, POWER_ON, STRAY_MIREK);
    }

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNCONFIRMED)));
    assert_eq!(execution.confirmed.power_on_level, None);
    assert_script_consumed(&transport);
}

#[test]
fn every_level_colour_outcome_fits_the_compact_error_payload() {
    let outcomes = [LEVEL_COLOUR_UNCONFIRMED, LEVEL_COLOUR_UNANSWERED, LEVEL_COLOUR_CONTENDED, DEVICE_TYPES_UNKNOWN];
    for outcome in outcomes {
        let payload = CompactErrorPayload::new(ErrorCode::OperationFailed, outcome);
        assert_eq!(payload.message.as_str(), outcome, "an operator reads the whole name");
    }
}

#[derive(Clone, Copy, Debug)]
enum Echo {
    Silent,
    Violated,
    Wrong,
}

fn expect_unarmed_selector(mock: &MockDaliTransport, selector: u8, echo: Echo) {
    const FOREIGN_SELECTOR: u8 = 7;
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(selector)));
    let echo_query = standard_frame(StandardCommand::QueryContentDtr0);
    match echo {
        Echo::Silent => mock.expect_forward_frame_with_backward(echo_query, None),
        Echo::Violated => mock.expect_forward_frame_corrupted_in_window(echo_query),
        Echo::Wrong => mock.expect_forward_frame_with_backward(echo_query, Some(FOREIGN_SELECTOR)),
    }
}

// IEC 62386-101 §8.2.5
// IEC 62386-102 §3.13
#[test]
fn a_selector_that_never_proves_names_silence_a_violation_and_a_wrong_echo_apart() {
    let cases = [
        (Echo::Silent, LEVEL_COLOUR_UNANSWERED),
        (Echo::Violated, LEVEL_COLOUR_CONTENDED),
        (Echo::Wrong, LEVEL_COLOUR_UNCONFIRMED),
    ];
    for (echo, outcome) in cases {
        let mock = MockDaliTransport::new();
        for _ in 0..COLOUR_VALUE_ASKS {
            expect_unarmed_selector(&mock, COLOUR_VALUE_TEMPORARY_COLOUR_TYPE, echo);
        }

        let (execution, transport) = write_level(mock, POWER_ON);

        assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(outcome)), "{echo:?}");
        assert_script_consumed(&transport);
    }
}

const STORED_RGBWAF: [u8; 3] = [0x40, 0x00, 0x80];
const STRAY_CONTROL: u8 = 0x3F;

fn expect_rgbwaf_values(mock: &MockDaliTransport, selectors: [u8; 3], values: [u8; 3]) {
    for (selector, value) in selectors.into_iter().zip(values) {
        expect_narrow_colour_value_sample(mock, SHORT, selector, Some(value));
    }
}

fn expect_rgbwaf_copied(mock: &MockDaliTransport, temporary: [u8; 3]) {
    expect_stored_type(mock, POWER_ON, DT8_COLOUR_TYPE_RGBWAF);
    let report = [DT8_COLOUR_VALUE_REPORT_RED, COLOUR_VALUE_REPORT_FREECOLOUR, COLOUR_VALUE_REPORT_RGBWAF_CONTROL];
    expect_rgbwaf_values(mock, report, STORED_RGBWAF);
    expect_dt8_command(mock, Dt8Command::CopyReportToTemporary);
    expect_temporary_type(mock, Some(DT8_COLOUR_TYPE_RGBWAF));
    let staged = [COLOUR_VALUE_TEMPORARY_RED, COLOUR_VALUE_TEMPORARY_FREECOLOUR, COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL];
    expect_rgbwaf_values(mock, staged, temporary);
}

// IEC 62386-209 §9.12.7, Table 11
#[test]
fn a_copy_that_left_a_stray_rgbwaf_control_is_caught_past_the_first_value() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(DT8_COLOUR_TYPE_RGBWAF));
    expect_rgbwaf_copied(&mock, [STORED_RGBWAF[0], STORED_RGBWAF[1], STRAY_CONTROL]);
    expect_rgbwaf_copied(&mock, STORED_RGBWAF);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

const STORED_XY: [(u8, u8); 2] = [(0x51, 0x00), (0x52, 0x00)];
const STRAY_Y: (u8, u8) = (0x10, 0x00);

fn expect_xy_copied(mock: &MockDaliTransport, temporary_y: (u8, u8)) {
    expect_stored_type(mock, POWER_ON, DT8_COLOUR_TYPE_XY);
    for (selector, value) in [DT8_COLOUR_VALUE_REPORT_X, DT8_COLOUR_VALUE_REPORT_Y].into_iter().zip(STORED_XY) {
        expect_wide_colour_value_sample(mock, SHORT, selector, value);
    }
    expect_dt8_command(mock, Dt8Command::CopyReportToTemporary);
    expect_temporary_type(mock, Some(DT8_COLOUR_TYPE_XY));
    expect_wide_colour_value_sample(mock, SHORT, COLOUR_VALUE_TEMPORARY_X, STORED_XY[0]);
    expect_wide_colour_value_sample(mock, SHORT, COLOUR_VALUE_TEMPORARY_Y, temporary_y);
}

#[test]
fn a_copy_that_left_a_stray_y_coordinate_is_caught_past_the_first_value() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(DT8_COLOUR_TYPE_XY));
    expect_xy_copied(&mock, STRAY_Y);
    expect_xy_copied(&mock, STORED_XY[1]);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}
