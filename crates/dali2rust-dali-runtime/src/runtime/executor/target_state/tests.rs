use super::*;
use crate::runtime::executor::helpers::DT8_ACTIVATE;
use dali2rust_domain::dali::devices::dt8_color::{
    RGBWAF_CONTROL_NORMALISED, RGBWAF_CONTROL_POWER_UP_DEFAULT,
};
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, setup_controller, short_address, short_raw_query_frame,
};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::commands::DaliCommand;

fn expect_staged_dtrs_unproved(mock: &MockDaliTransport, values: &[u8]) {
    for (write, value) in DTR_WRITE.iter().zip(values) {
        mock.expect_forward_frame(DaliCommand::Special(write(*value)).to_forward_frame().raw());
    }
}

fn expect_staged_dtrs(mock: &MockDaliTransport, short: u8, values: &[u8]) {
    expect_staged_dtrs_unproved(mock, values);
    for (query, value) in DTR_QUERY.iter().zip(values) {
        expect_dtr_readback(mock, short, *query, *value);
    }
}

fn expect_dtr_readback(
    mock: &MockDaliTransport,
    short: u8,
    query: StandardCommand,
    answer: u8,
) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: query,
        }
        .to_forward_frame()
        .raw(),
        Some(answer),
    );
}

#[test]
fn a_lost_staging_special_is_repaired_before_the_dt8_command() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs_unproved(&mock, &[250, 0]);
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 99);
    expect_staged_dtrs(&mock, short, &[250, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPERATURE_TC,
    ));

    let (transport, mut controller) = setup_controller(mock);
    apply_dt8_cct(&mut controller, short_address(short), 4000).expect("cct");

    assert_script_consumed(&transport);
}

#[test]
fn staging_that_never_confirms_never_sends_the_dt8_command() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..=DT8_STAGE_ARM_RETRIES {
        expect_staged_dtrs_unproved(&mock, &[250, 0]);
        expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 99);
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = apply_dt8_cct(&mut controller, short_address(short), 4000)
        .expect_err("unconfirmed staging must not reach the gear");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("dt8_staging_unconfirmed")
    );
    assert_script_consumed(&transport);
}

#[test]
fn target_state_level_sends_direct_arc_power() {
    let mock = MockDaliTransport::new();
    let expected = DaliCommand::Standard {
        address: short_address(17),
        command: StandardCommand::DirectArcPower { level: 180 },
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(expected);

    let (transport, mut controller) = setup_controller(mock);

    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: 180,
        ..Default::default()
    };
    apply_short_target_state(&mut controller, 17, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn target_state_rgb_uses_temporary_rgb_dimlevels() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_RGB_DIMLEVEL,
    ));
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_WAF_DIMLEVEL,
    ));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );

    let (transport, mut controller) = setup_controller(mock);

    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Rgb,
        r: 255,
        g: 0,
        b: 0,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn target_state_cct_uses_dtr_pair_then_dt8_temperature_command() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs(&mock, short, &[250, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPERATURE_TC,
    ));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_TC_ACTIVE),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 4000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn a_level_carrying_colour_write_never_sends_activate() {
    for mode in [ColorMode::Cct, ColorMode::Xy, ColorMode::Rgb] {
        let mock = MockDaliTransport::new();
        mock.set_persistent_response(DT8_STATUS_TC_ACTIVE | DT8_STATUS_XY_ACTIVE
            | DT8_STATUS_RGB_ACTIVE);
        let (transport, mut controller) = setup_controller(mock);
        let mut setpoint = LightSetpoint::default();
        let color = dali2rust_contracts::msg::ColorValue {
            mode,
            color_temperature_kelvin: 4000,
            ..Default::default()
        };
        setpoint.color = Some(color);
        setpoint.power = PowerState::On;
        setpoint.level = 200;
        apply_short_target_state(&mut controller, 17, &setpoint, ColorWritePolicy::NONE).expect("target-state");

        let frames = transport.lock().unwrap().sent_frames();
        let activate = short_raw_query_frame(short_address(17), DT8_ACTIVATE);
        assert!(
            !frames.contains(&activate),
            "{mode:?}: ACTIVATE ({activate:#06X}) is on the wire: {frames:04X?}",
        );
    }
}

#[test]
fn colour_without_power_does_not_switch_an_off_gear_on() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.set_persistent_response(0);
    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 4000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    setpoint.power = PowerState::Unknown;
    setpoint.level = 0;
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    let frames = transport.lock().unwrap().sent_frames();
    let go_last = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::GoToLastActiveLevel,
    }
    .to_forward_frame()
    .raw();
    assert!(
        !frames.contains(&go_last),
        "a colour-only change turned the lamp on: {frames:04X?}",
    );
    assert!(
        frames.contains(&short_raw_query_frame(short_address(short), DT8_ACTIVATE)),
        "the staged colour was left in the temporaries: {frames:04X?}",
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame >> 8 != u16::from(short_address(short).encode_address_byte())),
        "an arc-power frame reached a lamp the operator left off: {frames:04X?}",
    );
}

#[test]
fn a_setpoint_that_states_nothing_sends_no_frames() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let (transport, mut controller) = setup_controller(mock);
    let setpoint = LightSetpoint::default();
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE)
        .expect("target-state");
    assert!(
        transport.lock().unwrap().sent_frames().is_empty(),
        "a setpoint stating nothing put frames on the wire",
    );
}

#[test]
fn a_level_only_setpoint_leaves_automatic_activation_alone_issue117() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let (transport, mut controller) = setup_controller(mock);
    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: 200,
        color: Some(ColorValue::default()),
    };
    let policy = ColorWritePolicy {
        auto_activation: RepairAutoActivation::Yes,
        rgbwaf_control: AssertRgbwafControl::No,
    };
    apply_short_target_state(&mut controller, short, &setpoint, policy).expect("target-state");
    let frames = transport.lock().unwrap().sent_frames();
    let enable_dt8 = DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
        .to_forward_frame()
        .raw();
    assert!(
        !frames.contains(&enable_dt8),
        "a setpoint stating no colour repaired the activation bit: {frames:04X?}",
    );
    assert!(!frames.is_empty(), "the level never reached the wire");
}

#[test]
fn target_state_color_only_power_on_activates_then_switches_on() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs(&mock, short, &[252, 0, 147]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_RGB_DIMLEVEL,
    ));
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_WAF_DIMLEVEL,
    ));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(short_address(short), DT8_ACTIVATE));
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::GoToLastActiveLevel,
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint {
        power: PowerState::On,
        ..Default::default()
    };
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Rgb,
        r: 254,
        g: 0,
        b: 200,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn a_colour_only_power_on_asks_the_gear_nothing_whatever_it_is_doing() {
    let mock = MockDaliTransport::new();
    let short = 9;
    mock.set_persistent_response(DT8_STATUS_TC_ACTIVE);
    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint {
        power: PowerState::On,
        ..Default::default()
    };
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 4000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE)
        .expect("target-state");

    let frames = transport.lock().unwrap().sent_frames();
    assert!(
        !frames.contains(&standard_query_frame_for(short, StandardCommand::QueryStatus)),
        "the status gate is back: {frames:04X?}",
    );
    assert!(
        frames.contains(&short_raw_query_frame(short_address(short), DT8_ACTIVATE)),
        "the staged colour was never activated: {frames:04X?}",
    );
    let go_last = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::GoToLastActiveLevel,
    }
    .to_forward_frame()
    .raw();
    assert!(
        frames.contains(&go_last),
        "`power = On` did not switch the light on: {frames:04X?}",
    );
}

#[test]
fn target_state_group_color_only_power_on_sends_no_queries() {
    let mock = MockDaliTransport::new();
    let group = DaliAddress::group(3).expect("valid group");
    expect_staged_dtrs_unproved(&mock, &[1, 2, 3]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_SET_TEMPORARY_RGB_DIMLEVEL));
    expect_staged_dtrs_unproved(&mock, &[0, 0, 0]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_SET_TEMPORARY_WAF_DIMLEVEL));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_ACTIVATE));
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: group,
            command: StandardCommand::GoToLastActiveLevel,
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint {
        power: PowerState::On,
        ..Default::default()
    };
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Rgb,
        r: 10,
        g: 20,
        b: 30,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_group_target_state(&mut controller, 3, &setpoint).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn group_color_only_without_power_activates_via_dt8_activate() {
    let mock = MockDaliTransport::new();
    let group = DaliAddress::group(7).expect("valid group");
    expect_staged_dtrs_unproved(&mock, &[(333u16 & 0x00FF) as u8, (333u16 >> 8) as u8]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_SET_TEMPERATURE_TC));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_ACTIVATE));

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    assert_eq!(setpoint.power, PowerState::Unknown);
    assert_eq!(setpoint.level, 0);
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 3000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_group_target_state(&mut controller, 7, &setpoint).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn broadcast_color_only_without_power_activates_via_dt8_activate() {
    let mock = MockDaliTransport::new();
    let address = DaliAddress::Broadcast;
    expect_staged_dtrs_unproved(&mock, &[(333u16 & 0x00FF) as u8, (333u16 >> 8) as u8]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(address, DT8_SET_TEMPERATURE_TC));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(address, DT8_ACTIVATE));

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 3000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_broadcast_target_state(&mut controller, &setpoint).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn group_color_with_level_activates_by_dapc_alone() {
    let mock = MockDaliTransport::new();
    let group = DaliAddress::group(7).expect("valid group");
    expect_staged_dtrs_unproved(&mock, &[(333u16 & 0x00FF) as u8, (333u16 >> 8) as u8]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(group, DT8_SET_TEMPERATURE_TC));
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: group,
            command: StandardCommand::DirectArcPower { level: 180 },
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint {
        power: PowerState::On,
        level: 180,
        ..Default::default()
    };
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 3000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_group_target_state(&mut controller, 7, &setpoint).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn target_state_xy_sends_temporary_x_then_y_then_activate() {
    let mock = MockDaliTransport::new();
    let short = 18;
    expect_staged_dtrs(&mock, short, &[0x34, 0x12]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_X_COORDINATE,
    ));
    expect_staged_dtrs(&mock, short, &[0x78, 0x56]);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(
        short_address(short),
        DT8_SET_TEMPORARY_Y_COORDINATE,
    ));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_XY_ACTIVE),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Xy,
        x: 0x1234,
        y: 0x5678,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");

    assert_script_consumed(&transport);
}

#[test]
fn target_state_cct_redrives_once_when_colour_type_verify_fails() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let stage = |mock: &MockDaliTransport| {
        expect_staged_dtrs(mock, short, &[250, 0]);
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame().raw(),
        );
        mock.expect_forward_frame(short_raw_query_frame(short_address(short), DT8_SET_TEMPERATURE_TC));
    };
    stage(&mock);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame().raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    stage(&mock);

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 4000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn target_state_cct_redrives_when_value_reads_back_out_of_range() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let stage = |mock: &MockDaliTransport| {
        expect_staged_dtrs(mock, short, &[250, 0]);
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame().raw(),
        );
        mock.expect_forward_frame(short_raw_query_frame(short_address(short), DT8_SET_TEMPERATURE_TC));
    };
    stage(&mock);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8)).to_forward_frame().raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_TC_ACTIVE | DT8_STATUS_TC_OUT_OF_RANGE),
    );
    stage(&mock);

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = dali2rust_contracts::msg::ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 4000,
        ..Default::default()
    };
    setpoint.color = Some(color);
    apply_short_target_state(&mut controller, short, &setpoint, ColorWritePolicy::NONE).expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn target_state_power_off_fades_to_zero_with_dapc() {
    let mock = MockDaliTransport::new();
    let expected = DaliCommand::Standard {
        address: short_address(5),
        command: StandardCommand::DirectArcPower { level: 0 },
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(expected);

    let (transport, mut controller) = setup_controller(mock);
    let setpoint = LightSetpoint {
        power: PowerState::Off,
        level: 0,
        ..Default::default()
    };
    apply_short_target_state(&mut controller, 5, &setpoint, ColorWritePolicy::NONE).expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn target_state_on_with_zero_level_sends_go_to_last_active_level() {
    let mock = MockDaliTransport::new();
    let expected = DaliCommand::Standard {
        address: short_address(6),
        command: StandardCommand::GoToLastActiveLevel,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(expected);

    let (transport, mut controller) = setup_controller(mock);
    let setpoint = LightSetpoint {
        power: PowerState::On,
        level: 0,
        ..Default::default()
    };
    apply_short_target_state(&mut controller, 6, &setpoint, ColorWritePolicy::NONE).expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn target_state_invalid_short_address_returns_conflict() {
    let mock = MockDaliTransport::new();
    let (transport, mut controller) = setup_controller(mock);
    let setpoint = LightSetpoint::default();
    assert_eq!(
        apply_short_target_state(&mut controller, 64, &setpoint, ColorWritePolicy::NONE),
        Err(SemanticDaliError::Conflict("invalid_short_address"))
    );
    assert_script_consumed(&transport);
}

#[test]
fn sequence_retry_reruns_only_contended_aborts_within_budget() {
    let mut attempts = 0;
    let result = apply_with_sequence_retry(1, || {
        attempts += 1;
        if attempts == 1 {
            Err(SemanticDaliError::OperationFailed("bus_contended"))
        } else {
            Ok(attempts)
        }
    });
    assert_eq!(result, Ok(2));

    let mut attempts = 0;
    let exhausted = apply_with_sequence_retry(1, || {
        attempts += 1;
        Err::<(), _>(SemanticDaliError::OperationFailed("bus_contended"))
    });
    assert_eq!(
        exhausted,
        Err(SemanticDaliError::OperationFailed("bus_contended"))
    );
    assert_eq!(attempts, 2, "budget bounds the re-runs");

    let mut attempts = 0;
    let other = apply_with_sequence_retry(3, || {
        attempts += 1;
        Err::<(), _>(SemanticDaliError::OperationFailed("dali_transport_error"))
    });
    assert_eq!(
        other,
        Err(SemanticDaliError::OperationFailed("dali_transport_error"))
    );
    assert_eq!(attempts, 1, "non-contended errors are not re-run");
}

#[test]
fn an_rgb_write_asserts_the_control_byte_when_the_gear_is_not_already_there() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, Some(RGBWAF_CONTROL_POWER_UP_DEFAULT));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0x80))
            .to_forward_frame()
            .raw(),
    );
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x80);
    expect_dt8(&mock, short, Dt8Command::SetTemporaryRgbwafControl);
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );
    expect_rgbwaf_control_read(&mock, short, Some(0x80));

    let (transport, mut controller) = setup_controller(mock);
    apply_short_target_state(&mut controller, short, &rgb_setpoint(255, 0, 0, 200), assert_policy())
        .expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn a_gear_already_in_normalised_control_is_not_written_to() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, Some(0x80));
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );
    expect_rgbwaf_control_read(&mock, short, Some(0x80));

    let (transport, mut controller) = setup_controller(mock);
    apply_short_target_state(&mut controller, short, &rgb_setpoint(255, 0, 0, 200), assert_policy())
        .expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn a_gear_that_stays_linked_fails_the_write() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, Some(RGBWAF_CONTROL_POWER_UP_DEFAULT));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0x80))
            .to_forward_frame()
            .raw(),
    );
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x80);
    expect_dt8(&mock, short, Dt8Command::SetTemporaryRgbwafControl);
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );
    expect_rgbwaf_control_read(&mock, short, Some(RGBWAF_CONTROL_POWER_UP_DEFAULT));

    let (transport, mut controller) = setup_controller(mock);
    let error = apply_short_target_state(
        &mut controller,
        short,
        &rgb_setpoint(255, 0, 0, 200),
        assert_policy(),
    )
    .expect_err("a refused control byte is a failed colour write");
    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("dt8_rgbwaf_control_refused")
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_silent_gear_is_neither_written_to_nor_failed() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, None);
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );
    expect_rgbwaf_control_read(&mock, short, None);

    let (transport, mut controller) = setup_controller(mock);
    apply_short_target_state(&mut controller, short, &rgb_setpoint(255, 0, 0, 200), assert_policy())
        .expect("silence is not a refusal");
    assert_script_consumed(&transport);
}

#[test]
fn a_cct_write_does_not_touch_the_control_byte() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs(&mock, short, &[0x9A, 0x01]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPERATURE_TC);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_TC_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = LightSetpoint::default();
    let color = ColorValue {
        mode: ColorMode::Cct,
        color_temperature_kelvin: 2439,
        ..Default::default()
    };
    setpoint.color = Some(color);
    setpoint.power = PowerState::On;
    setpoint.level = 200;
    apply_short_target_state(&mut controller, short, &setpoint, assert_policy())
        .expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn a_colour_only_write_to_a_dark_gear_is_activated_and_verified() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, Some(RGBWAF_CONTROL_POWER_UP_DEFAULT));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0x80))
            .to_forward_frame()
            .raw(),
    );
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x80);
    expect_dt8(&mock, short, Dt8Command::SetTemporaryRgbwafControl);
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    expect_dt8_raw(&mock, short, DT8_ACTIVATE);
    expect_rgbwaf_control_read(&mock, short, Some(RGBWAF_CONTROL_NORMALISED));

    let (transport, mut controller) = setup_controller(mock);
    let mut setpoint = rgb_setpoint(255, 0, 0, 0);
    setpoint.power = PowerState::Unknown;
    apply_short_target_state(&mut controller, short, &setpoint, assert_policy())
        .expect("a staged colour on a dark gear is not a failure");
    assert_script_consumed(&transport);
}

#[test]
fn the_auto_activation_restore_arms_proves_writes_and_reads_back() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0x01))
            .to_forward_frame()
            .raw(),
    );
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x01);
    expect_dt8(&mock, short, Dt8Command::StoreGearFeaturesStatus);
    mock.expect_forward_frame(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::StoreGearFeaturesStatus),
        }
        .to_forward_frame()
        .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryGearFeaturesStatus),
        }
        .to_forward_frame()
        .raw(),
        Some(0xC1),
    );

    let (transport, mut controller) = setup_controller(mock);
    ensure_automatic_activation(&mut controller, short_address(short))
        .expect("the bit goes back");
    assert_script_consumed(&transport);
}

#[test]
fn an_unrestorable_activation_bit_fails_after_the_repair_budget() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..3 {
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::Dtr0(0x01))
                .to_forward_frame()
                .raw(),
        );
        expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x01);
        expect_dt8(&mock, short, Dt8Command::StoreGearFeaturesStatus);
        mock.expect_forward_frame(
            DaliCommand::Extended {
                address: short_address(short),
                command: ExtendedCommand::Dt8(Dt8Command::StoreGearFeaturesStatus),
            }
            .to_forward_frame()
            .raw(),
        );
        mock.expect_forward_frame(
            DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
                .to_forward_frame()
                .raw(),
        );
        mock.expect_forward_frame_with_backward(
            DaliCommand::Extended {
                address: short_address(short),
                command: ExtendedCommand::Dt8(Dt8Command::QueryGearFeaturesStatus),
            }
            .to_forward_frame()
            .raw(),
            Some(0x00),
        );
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = ensure_automatic_activation(&mut controller, short_address(short))
        .expect_err("a bit that will not go back is a failed colour write");
    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("dt8_auto_activation_unrestored")
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_six_channel_write_judges_the_control_byte_by_all_six_links() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_rgbwaf_control_read(&mock, short, Some(0xB8));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0x80))
            .to_forward_frame()
            .raw(),
    );
    expect_dtr_readback(&mock, short, StandardCommand::QueryContentDtr0, 0x80);
    expect_dt8(&mock, short, Dt8Command::SetTemporaryRgbwafControl);
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[10, 20, 30]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );
    expect_rgbwaf_control_read(&mock, short, Some(0x80));

    let (transport, mut controller) = setup_controller(mock);
    apply_short_target_state(
        &mut controller,
        short,
        &rgbwaf_setpoint(255, 0, 0, 55, 79, 96, 200),
        assert_policy(),
    )
    .expect("target-state");
    assert_script_consumed(&transport);
}

#[test]
fn an_rgb_write_without_the_assert_permission_still_zeroes_the_waf_channels() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_staged_dtrs(&mock, short, &[254, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_RGB_DIMLEVEL);
    expect_staged_dtrs(&mock, short, &[0, 0, 0]);
    expect_dt8_raw(&mock, short, DT8_SET_TEMPORARY_WAF_DIMLEVEL);
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_STATUS),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
    mock.expect_forward_frame(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::DirectArcPower { level: 200 },
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    apply_short_target_state(
        &mut controller,
        short,
        &rgb_setpoint(255, 0, 0, 200),
        ColorWritePolicy::NONE,
    )
    .expect("target-state");
    assert_script_consumed(&transport);
}

fn standard_query_frame_for(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

fn assert_policy() -> ColorWritePolicy {
    ColorWritePolicy {
        auto_activation: RepairAutoActivation::No,
        rgbwaf_control: AssertRgbwafControl::Yes,
    }
}

fn rgb_setpoint(r: u8, g: u8, b: u8, level: u8) -> LightSetpoint {
    let mut setpoint = LightSetpoint::default();
    let color = ColorValue {
        mode: ColorMode::Rgb,
        r,
        g,
        b,
        ..Default::default()
    };
    setpoint.color = Some(color);
    setpoint.power = PowerState::On;
    setpoint.level = level;
    setpoint
}

fn rgbwaf_setpoint(r: u8, g: u8, b: u8, w: u8, a: u8, f: u8, level: u8) -> LightSetpoint {
    let mut setpoint = rgb_setpoint(r, g, b, level);
    let color = setpoint.color.as_mut().expect("rgb_setpoint sets a colour");
    color.mode = ColorMode::Rgbwaf;
    color.w = w;
    color.a = a;
    color.f = f;
    setpoint
}

fn expect_rgbwaf_control_read(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryRgbwafControl),
        }
        .to_forward_frame()
        .raw(),
        answer,
    );
}

fn expect_dt8(mock: &MockDaliTransport, short: u8, command: Dt8Command) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(command),
        }
        .to_forward_frame()
        .raw(),
    );
}

fn expect_dt8_raw(mock: &MockDaliTransport, short: u8, opcode: u8) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(short_raw_query_frame(short_address(short), opcode));
}
