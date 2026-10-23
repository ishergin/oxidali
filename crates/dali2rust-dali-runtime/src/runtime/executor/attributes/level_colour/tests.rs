use super::*;
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, expect_narrow_colour_value_sample, priorities_of, setup_controller,
    short_address,
};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::commands::DaliCommand;
use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_TC;
use dali2rust_domain::dali::ses::DaliPriority;
use std::sync::{Arc, Mutex};

const SHORT: u8 = 9;
const LEVEL: u8 = 120;
const OLD_LEVEL: u8 = 254;
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
    for supported in [Some(colour_gear()), None] {
        let mock = MockDaliTransport::new();
        for _ in 0..COLOUR_VALUE_ASKS {
            expect_temporary_type(&mock, None);
        }

        let (execution, transport) = write_level_on(mock, POWER_ON, supported);

        assert_eq!(
            execution.error,
            Some(SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNANSWERED)),
            "{supported:?}: silence proves no colour absent"
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
        expect_stored_type(&mock, stored, COLOUR_TYPE_BYTE_TC);
        expect_dt8_command(&mock, Dt8Command::CopyReportToTemporary);
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
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}

#[test]
fn a_level_query_that_loads_no_report_stores_nothing() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    mock.expect_forward_frame_with_backward(standard_frame(POWER_ON.query), None);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_eq!(execution.error, Some(SemanticDaliError::OperationFailed(LEVEL_COLOUR_UNCONFIRMED)));
    assert_eq!(execution.confirmed.power_on_level, None);
    assert_script_consumed(&transport);
}

#[test]
fn a_collision_on_the_recharge_reruns_the_unit_from_the_probe() {
    let mock = MockDaliTransport::new();
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_type(&mock, POWER_ON, COLOUR_TYPE_BYTE_TC);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_collision(dt8_frame(Dt8Command::CopyReportToTemporary));
    expect_temporary_type(&mock, Some(COLOUR_TYPE_BYTE_TC));
    expect_stored_type(&mock, POWER_ON, COLOUR_TYPE_BYTE_TC);
    expect_dt8_command(&mock, Dt8Command::CopyReportToTemporary);
    expect_store(&mock, POWER_ON);

    let (execution, transport) = write_level(mock, POWER_ON);

    assert_level_confirmed(&execution, POWER_ON);
    assert_script_consumed(&transport);
}
