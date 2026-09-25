use super::*;
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, setup_controller, short_address,
};
use std::sync::{Arc, Mutex};

use crate::runtime::executor::attributes::dt8::read_dt8_tc_limits;
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::commands::DaliCommand;

#[test]
fn extended_fade_time_codec_round_trips_representable_values() {
    for (ms, byte) in [
        (0u16, 0x00u8),
        (100, 0x10),
        (500, 0x14),
        (1600, 0x1F),
        (2000, 0x21),
        (16_000, 0x2F),
        (30_000, 0x32),
        (60_000, 0x35),
    ] {
        assert_eq!(extended_fade_time_byte_from_ms(ms), byte, "{ms} ms");
        assert_eq!(extended_fade_time_ms_from_byte(byte), Some(ms), "0x{byte:02X}");
    }
}

#[test]
fn every_encoded_extended_fade_byte_decodes_within_the_contract() {
    for ms in [64_999u16, 65_000, 65_001, 65_300, u16::MAX] {
        let byte = extended_fade_time_byte_from_ms(ms);
        let decoded = extended_fade_time_ms_from_byte(byte);
        assert!(
            decoded.is_some(),
            "{ms} ms encoded to 0x{byte:02X}, which the contract cannot decode — \
             the gear would hold a value the record cannot carry"
        );
        assert_eq!(
            decoded,
            Some(60_000),
            "the top of the band clamps to the 60 s the gear will actually hold"
        );
    }
}

#[test]
fn extended_fade_time_decode_handles_reserved_and_rounding() {
    assert_eq!(
        extended_fade_time_ms_from_byte(0x0F),
        Some(0),
        "multiplier 0 = no extended fade regardless of base"
    );
    assert_eq!(extended_fade_time_ms_from_byte(0x50), None, "reserved multiplier");
    assert_eq!(extended_fade_time_byte_from_ms(10), 0x10, "sub-unit rounds up to 100 ms");
    assert_eq!(extended_fade_time_byte_from_ms(550), 0x15, "rounds to the nearest step");
    assert_eq!(extended_fade_time_ms_from_byte(0x4F), None, "16 min");
    assert_eq!(extended_fade_time_ms_from_byte(0x41), None, "2 min");
    assert_eq!(extended_fade_time_ms_from_byte(0x36), None, "70 s");
    assert_eq!(extended_fade_time_ms_from_byte(0x35), Some(60_000), "60 s fits");
}

fn groups_query_frames(address: DaliAddress) -> (u16, u16) {
    let frame = |command| {
        DaliCommand::Standard { address, command }
            .to_forward_frame()
            .raw()
    };
    (
        frame(StandardCommand::QueryGroups0To7),
        frame(StandardCommand::QueryGroups8To15),
    )
}

#[test]
fn groups_membership_doubled_byte_signature_triggers_one_reread() {
    let mock = MockDaliTransport::new();
    let (lo, hi) = groups_query_frames(short_address(9));
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x02));
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x00));

    let (transport, mut controller) = setup_controller(mock);
    let mask = read_group_membership_mask(&mut controller, 9).expect("read");
    assert_eq!(mask, Some(0x0002), "re-read replaces the doubled first pair");
    assert_script_consumed(&transport);
}

#[test]
fn groups_membership_confirmed_double_accepted_and_clean_read_stays_single() {
    let (lo, hi) = groups_query_frames(short_address(9));

    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x02));
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x02));
    let (transport, mut controller) = setup_controller(mock);
    assert_eq!(
        read_group_membership_mask(&mut controller, 9).expect("read"),
        Some(0x0202),
        "confirmed double is legal state"
    );
    assert_script_consumed(&transport);

    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(lo, Some(0x03));
    mock.expect_forward_frame_with_backward(hi, Some(0x00));
    let (transport, mut controller) = setup_controller(mock);
    assert_eq!(
        read_group_membership_mask(&mut controller, 9).expect("read"),
        Some(0x0003)
    );
    assert_script_consumed(&transport);
}

fn standard_query_frame(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

fn expect_colour_value_sample(
    mock: &MockDaliTransport,
    short: u8,
    selector: u8,
    msb: u8,
    lsb: u8,
) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(selector))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        standard_query_frame(short, StandardCommand::QueryContentDtr0),
        Some(selector),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryColourValue),
        }
        .to_forward_frame()
        .raw(),
        Some(msb),
    );
    mock.expect_forward_frame_with_backward(
        standard_query_frame(short, StandardCommand::QueryContentDtr0),
        Some(lsb),
    );
}

fn expect_narrow_colour_value_sample(
    mock: &MockDaliTransport,
    short: u8,
    selector: u8,
    answer: u8,
) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(selector))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        standard_query_frame(short, StandardCommand::QueryContentDtr0),
        Some(selector),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryColourValue),
        }
        .to_forward_frame()
        .raw(),
        Some(answer),
    );
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

fn expect_gear_features(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
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
        answer,
    );
}

#[test]
fn a_mask_colour_value_is_not_a_reading() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC, 0xFF, 0xFF);
    let (transport, mut controller) = setup_controller(mock);
    let value = read_dt8_color_value_u16(
        &mut controller,
        short_address(short),
        DT8_COLOUR_VALUE_TC,
        ContentConfirmPolicy::default(),
    )
    .expect("exchange");
    assert_eq!(value, None, "0xFFFF is 'not available', not 15 K");
    assert_script_consumed(&transport);
}

#[test]
fn a_single_point_tc_fixture_reports_its_one_temperature_as_a_range() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC_COOLEST, 0x01, 0x2C);
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC_WARMEST, 0x01, 0x2C);
    let (transport, mut controller) = setup_controller(mock);
    let limits = read_dt8_tc_limits(
        &mut controller,
        short_address(short),
        ContentConfirmPolicy::default(),
    )
    .expect("exchange");
    assert_eq!(
        limits,
        (Some(300), Some(300)),
        "one point is the fixture's whole range"
    );
    assert_script_consumed(&transport);
}

#[test]
fn an_inverted_tc_pair_is_not_a_range() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC_COOLEST, 0x01, 0x90);
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC_WARMEST, 0x01, 0x2C);
    let (transport, mut controller) = setup_controller(mock);
    let limits = read_dt8_tc_limits(
        &mut controller,
        short_address(short),
        ContentConfirmPolicy::default(),
    )
    .expect("exchange");
    assert_eq!(limits, (None, None));
    assert_script_consumed(&transport);
}

#[test]
fn a_mask_rgb_dim_level_is_not_a_reading() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_narrow_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_RED, 0xFF);
    let (transport, mut controller) = setup_controller(mock);
    let levels = read_dt8_dim_level_trio(
        &mut controller,
        short_address(short),
        ContentConfirmPolicy::default(),
        [
            DT8_COLOUR_VALUE_RED,
            DT8_COLOUR_VALUE_GREEN,
            DT8_COLOUR_VALUE_BLUE,
        ],
    )
    .expect("exchange");
    assert_eq!(levels, None, "all three channels or nothing");
    assert_script_consumed(&transport);
}

#[test]
fn a_narrow_dim_level_is_read_from_the_answer() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_narrow_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_RED, 0);
    expect_narrow_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_GREEN, 254);
    expect_narrow_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_BLUE, 0);
    let (transport, mut controller) = setup_controller(mock);
    let levels = read_dt8_dim_level_trio(
        &mut controller,
        short_address(short),
        ContentConfirmPolicy::default(),
        [
            DT8_COLOUR_VALUE_RED,
            DT8_COLOUR_VALUE_GREEN,
            DT8_COLOUR_VALUE_BLUE,
        ],
    )
    .expect("exchange");
    assert_eq!(levels, Some((0, 254, 0)));
    assert_script_consumed(&transport);
}

fn short_raw_query_frame(address: DaliAddress, opcode: u8) -> u16 {
    let frame = dali2rust_domain::dali::frame::ForwardFrame::new(
        address.encode_address_byte() | 0x01,
        opcode,
    );
    frame.raw()
}

#[test]
fn read_attributes_outcomes_classify_contended_abort_per_section() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let presence = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::QueryControlGearPresent,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_with_backward(presence, Some(0xFF));
    let version = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::QueryVersionNumber,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_collision(version);
    mock.expect_forward_frame_collision(version);
    mock.expect_forward_frame_collision(version);

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::Common102, DaliAttributeGroup::Scenes];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    let error = read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect_err("retry exhaustion must abort");

    assert_eq!(error, SemanticDaliError::OperationFailed("bus_contended"));
    assert_eq!(outcomes.identity, AttributeGroupReadOutcome::Success);
    assert_eq!(
        outcomes.common_102,
        AttributeGroupReadOutcome::ContendedAbort
    );
    assert_eq!(outcomes.scenes, AttributeGroupReadOutcome::NotAttempted);
    assert_eq!(
        outcomes.runtime_status,
        AttributeGroupReadOutcome::NotRequested
    );
    assert_eq!(
        outcomes.memory_banks,
        AttributeGroupReadOutcome::NotRequested
    );
    assert_script_consumed(&transport);
}

fn expect_fade_time_readback(mock: &MockDaliTransport, short: u8, code: u8) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryFadeTimeFadeRate,
        }
        .to_forward_frame()
        .raw(),
        Some(code << 4),
    );
}

#[test]
fn write_attributes_fade_time_confirms_the_accepted_code_not_the_request() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(1))
            .to_forward_frame()
            .raw(),
    );
    let set_fade = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::SetFadeTime,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    expect_fade_time_readback(&mock, short, 1);

    let (transport, mut controller) = setup_controller(mock);
    let execution =
        write_short_attributes(&mut controller, short, Some(500), None, None, None, None, (None, None), (None, None), None);

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.fade_time_ms, Some(700));
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_small_fade_time_never_selects_the_extended_fade_code() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(1))
            .to_forward_frame()
            .raw(),
    );
    let set_fade = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::SetFadeTime,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    expect_fade_time_readback(&mock, short, 1);

    let (transport, mut controller) = setup_controller(mock);
    let execution =
        write_short_attributes(&mut controller, short, Some(100), None, None, None, None, (None, None), (None, None), None);

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.fade_time_ms, Some(700));
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_zero_fade_time_selects_the_extended_fade_code() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(0))
            .to_forward_frame()
            .raw(),
    );
    let set_fade = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::SetFadeTime,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    expect_fade_time_readback(&mock, short, 0);

    let (transport, mut controller) = setup_controller(mock);
    let execution =
        write_short_attributes(&mut controller, short, Some(0), None, None, None, None, (None, None), (None, None), None);

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.fade_time_ms, Some(0));
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_top_of_table_survives_the_round_trip() {
    assert_eq!(fade_time_dtr0_from_ms(90_500), 15);
    assert_eq!(fade_time_ms_from_dtr0(15), 90_500);
    assert_eq!(fade_time_dtr0_from_ms(fade_time_ms_from_dtr0(15)), 15);
}

fn expect_bound_triple(
    mock: &MockDaliTransport,
    short: u8,
    set: StandardCommand,
    query: StandardCommand,
    dtr0: u8,
    answer: u8,
) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(dtr0))
            .to_forward_frame()
            .raw(),
    );
    let set_frame = DaliCommand::Standard {
        address: short_address(short),
        command: set,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame(set_frame);
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

fn expect_fade_time_write(mock: &MockDaliTransport, short: u8, code: u8, answer: Option<u8>) {
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Dtr0(code)).to_forward_frame().raw());
    let set_fade = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::SetFadeTime,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryFadeTimeFadeRate,
        }
        .to_forward_frame()
        .raw(),
        answer,
    );
}

fn dt6_frame(short: u8, command: Dt6Command) -> u16 {
    DaliCommand::Extended {
        address: short_address(short),
        command: ExtendedCommand::Dt6(command),
    }
    .to_forward_frame()
    .raw()
}

#[test]
fn an_unanswered_fade_time_read_back_is_named_and_confirms_nothing() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_fade_time_write(&mock, short, 1, None);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, Some(500), None, None, None, None, (None, None), (None, None), None,
    );

    assert_eq!(execution.confirmed.fade_time_ms, None, "a silent query proves nothing");
    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE))
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_silent_read_back_does_not_hold_back_the_fields_that_answered() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_fade_time_write(&mock, short, 1, None);
    expect_bound_triple(&mock, short, StandardCommand::SetPowerOnLevel, StandardCommand::QueryPowerOnLevel, 200, 200);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, Some(500), None, Some(200), None, None, (None, None), (None, None), None,
    );

    assert_eq!(execution.confirmed.fade_time_ms, None);
    assert_eq!(execution.confirmed.power_on_level, Some(200), "the answered write still lands");
    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE))
    );
    assert_script_consumed(&transport);
}

#[test]
fn an_unanswered_dimming_curve_read_back_is_named_and_confirms_nothing() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let curve = 1;
    let enable_dt6 = DaliCommand::Special(SpecialCommand::EnableDeviceType(6)).to_forward_frame().raw();
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Dtr0(curve)).to_forward_frame().raw());
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryContentDtr0,
        }
        .to_forward_frame()
        .raw(),
        Some(curve),
    );
    mock.expect_forward_frame(enable_dt6);
    mock.expect_forward_frame(dt6_frame(short, Dt6Command::SelectDimmingCurve));
    mock.expect_forward_frame(dt6_frame(short, Dt6Command::SelectDimmingCurve));
    mock.expect_forward_frame(enable_dt6);
    mock.expect_forward_frame_with_backward(dt6_frame(short, Dt6Command::QueryDimmingCurve), None);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (None, None), Some(curve),
    );

    assert_eq!(execution.confirmed.dimming_curve, None, "a gear without DT6 stays unconfirmed");
    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE))
    );
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_min_level_clamp_converges_to_accepted_value() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_bound_triple(&mock, short, StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel, 220, 200);
    expect_bound_triple(&mock, short, StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel, 220, 200);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (Some(220), None), None,
    );

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.min_level, Some(200), "accepted, not requested");
    assert_script_consumed(&transport);
}

#[test]
fn a_confirmed_curve_write_rereads_the_physical_minimum() {
    let mock = MockDaliTransport::new();
    mock.set_persistent_response(0);
    let short = 17;
    let phm_frame = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::QueryPhysicalMinimum,
    }
    .to_forward_frame()
    .raw();

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (None, None), Some(0),
    );

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.dimming_curve, Some(0));
    assert_eq!(
        execution.confirmed.physical_minimum_readback,
        Some(0),
        "a confirmed curve change must carry the gear's adjusted PHM"
    );
    assert!(
        transport.lock().expect("mock").sent_frames().contains(&phm_frame),
        "the PHM re-read must actually reach the wire"
    );
}

#[test]
fn write_attributes_unanswered_bound_verify_stays_unconfirmed_and_is_named() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Dtr0(100))
            .to_forward_frame()
            .raw(),
    );
    let set_frame = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::SetMinLevel,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryMinLevel,
        }
        .to_forward_frame()
        .raw(),
        None,
    );

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (Some(100), None), None,
    );

    assert_eq!(
        execution.error,
        Some(SemanticDaliError::OperationFailed(VERIFY_UNANSWERED_MESSAGE)),
        "silence is its own outcome, not a refusal and not a proof"
    );
    assert_eq!(execution.confirmed.min_level, None, "unproved stays unconfirmed");
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_max_level_alone_confirms_one_triple() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_bound_triple(&mock, short, StandardCommand::SetMaxLevel, StandardCommand::QueryMaxLevel, 180, 180);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (None, Some(180)), None,
    );

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.max_level, Some(180));
    assert_eq!(execution.confirmed.min_level, None);
    assert_script_consumed(&transport);
}

#[test]
fn write_attributes_raising_both_bounds_redrives_min_after_max() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let min = |answer| (StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel, answer);
    for (set, query, answer) in [min(100), min(100)] {
        expect_bound_triple(&mock, short, set, query, 150, answer);
    }
    expect_bound_triple(&mock, short, StandardCommand::SetMaxLevel, StandardCommand::QueryMaxLevel, 200, 200);
    expect_bound_triple(&mock, short, StandardCommand::SetMinLevel, StandardCommand::QueryMinLevel, 150, 150);

    let (transport, mut controller) = setup_controller(mock);
    let execution = write_short_attributes(
        &mut controller, short, None, None, None, None, None, (None, None), (Some(150), Some(200)), None,
    );

    assert_eq!(execution.error, None);
    assert_eq!(execution.confirmed.min_level, Some(150));
    assert_eq!(execution.confirmed.max_level, Some(200));
    assert_script_consumed(&transport);
}

#[test]
fn read_attributes_falls_back_to_dt8_queries_when_advertisement_times_out() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryControlGearPresent,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x62),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x80),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryStatus,
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryActualLevel,
        }
        .to_forward_frame()
        .raw(),
        Some(158),
    );
    for cmd in [
        StandardCommand::QueryVersionNumber,
        StandardCommand::QueryDeviceType,
        StandardCommand::QueryPhysicalMinimum,
        StandardCommand::QueryMinLevel,
        StandardCommand::QueryMaxLevel,
        StandardCommand::QueryPowerOnLevel,
        StandardCommand::QuerySystemFailureLevel,
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: cmd,
            }
            .to_forward_frame()
            .raw(),
            Some(0),
        );
    }
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryFadeTimeFadeRate,
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryLightSourceType,
        }
        .to_forward_frame()
        .raw(),
        Some(6),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    for (selector, lsb) in [(0u8, 0x10), (1, 0x11), (2, 0x12)] {
        expect_colour_value_sample(&mock, short, selector, 0, lsb);
    }
    for (selector, msb, lsb) in [(128u8, 0x00, 153), (130, 0x01, 114)] {
        expect_colour_value_sample(&mock, short, selector, msb, lsb);
    }
    expect_gear_features(&mock, short, Some(0x41));
    expect_rgbwaf_control_read(&mock, short, Some(0x80));
    for (command, reply) in [
        (StandardCommand::QueryRandomAddressH, 0x12),
        (StandardCommand::QueryRandomAddressM, 0x34),
        (StandardCommand::QueryRandomAddressL, 0x56),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(reply),
        );
    }

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_attributes(
        &mut controller,
        short,
        &[
            DaliAttributeGroup::RuntimeStatus,
            DaliAttributeGroup::Common102,
            DaliAttributeGroup::Dt8Color,
        ],
        ContentConfirmPolicy::default(),
        &mut AttributeReadOutcomes::for_request(&[], false),
    )
    .expect("read attributes");

    assert!(execution.has_dt8_color);
    assert_eq!(execution.dt8_color_mode, ColorMode::Rgb);
    assert!(execution.dt8_tc_capable);
    assert!(execution.dt8_rgb_capable);
    assert_eq!(execution.random_address, Some(0x12_34_56));
    assert_script_consumed(&transport);
}

#[test]
fn a_single_lost_presence_probe_is_repaired_by_the_re_ask() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let presence = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::QueryControlGearPresent,
    }
    .to_forward_frame()
    .raw();
    mock.expect_forward_frame_with_backward(presence, None);
    mock.expect_forward_frame_with_backward(presence, Some(0xFF));
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x62),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x80),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryStatus,
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryActualLevel,
        }
        .to_forward_frame()
        .raw(),
        Some(158),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt8(Dt8Command::QueryColourStatus),
        }
        .to_forward_frame()
        .raw(),
        Some(0),
    );
    for (selector, lsb) in [(0u8, 0x20), (1, 0x21), (2, 0x22)] {
        expect_colour_value_sample(&mock, short, selector, 0, lsb);
    }
    for (selector, msb, lsb) in [(128u8, 0x00, 153), (130, 0x01, 114)] {
        expect_colour_value_sample(&mock, short, selector, msb, lsb);
    }
    expect_gear_features(&mock, short, Some(0x41));
    expect_rgbwaf_control_read(&mock, short, Some(0x80));
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(0x11),
        );
    }

    let (transport, mut controller) = setup_controller(mock);
    let mut outcomes = AttributeReadOutcomes::for_request(&[], false);
    let execution = read_attributes(
        &mut controller,
        short,
        &[
            DaliAttributeGroup::RuntimeStatus,
            DaliAttributeGroup::Dt8Color,
        ],
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect("a repaired probe must not fail the read");

    assert_eq!(outcomes.identity, AttributeGroupReadOutcome::Success);
    assert!(execution.has_dt8_color);
    assert_eq!(execution.dt8_color_mode, ColorMode::Rgb);
    assert!(execution.dt8_tc_capable);
    assert!(execution.dt8_rgb_capable);
    assert_script_consumed(&transport);
}

#[test]
fn a_device_silent_through_the_whole_probe_budget_is_absent_not_a_success() {
    let mock = MockDaliTransport::new();
    let short = 17;
    let presence = DaliCommand::Standard {
        address: short_address(short),
        command: StandardCommand::QueryControlGearPresent,
    }
    .to_forward_frame()
    .raw();
    for _ in 0..=PRESENCE_PROBE_RETRIES {
        mock.expect_forward_frame_with_backward(presence, None);
    }

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::Common102, DaliAttributeGroup::Groups];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    let error = read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect_err("nothing answered, so nothing was read");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed(DEVICE_ABSENT_MESSAGE)
    );
    assert_eq!(
        error.code(),
        dali2rust_contracts::msg::ErrorCode::DeviceAbsent,
        "absence must not arrive as a generic operation_failed"
    );
    assert_eq!(outcomes.identity, AttributeGroupReadOutcome::DeviceAbsent);
    assert_eq!(outcomes.common_102, AttributeGroupReadOutcome::NotAttempted);
    assert_eq!(outcomes.groups, AttributeGroupReadOutcome::NotAttempted);
    assert_script_consumed(&transport);
}

#[test]
fn the_presence_budget_is_not_spent_on_a_device_that_answers() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryControlGearPresent,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );

    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), Some(0x11));
    }

    let (transport, mut controller) = setup_controller(mock);
    let mut outcomes = AttributeReadOutcomes::for_request(&[], false);
    read_attributes(
        &mut controller,
        short,
        &[],
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect("a device that answers is read");

    assert_eq!(outcomes.identity, AttributeGroupReadOutcome::Success);
    assert_script_consumed(&transport);
}

fn light_source_read(
    answer: Option<u8>,
    dtrs: &[Option<u8>],
) -> ((Option<u8>, Option<u32>), std::sync::Arc<std::sync::Mutex<MockDaliTransport>>) {
    const SHORT: u8 = 3;
    let mock = MockDaliTransport::new();
    mock.expect_forward_frame_with_backward(
        std_query_frame(SHORT, StandardCommand::QueryLightSourceType),
        answer,
    );
    for (command, value) in [
        StandardCommand::QueryContentDtr0,
        StandardCommand::QueryContentDtr1,
        StandardCommand::QueryContentDtr2,
    ]
    .into_iter()
    .zip(dtrs.iter().copied())
    {
        mock.expect_forward_frame_with_backward(std_query_frame(SHORT, command), value);
    }
    let (transport, mut controller) = setup_controller(mock);
    let read = read_light_source_type(
        &mut controller,
        short_address(SHORT),
        ContentConfirmPolicy::default(),
    )
    .expect("light source type read");
    (read, transport)
}

#[test]
fn a_concrete_light_source_type_costs_one_frame() {
    let ((answered, packed), transport) = light_source_read(Some(6), &[]);
    assert_eq!(answered, Some(6));
    assert_eq!(packed, None, "no MASK, no triple");
    assert_script_consumed(&transport);
}

#[test]
fn a_mask_light_source_type_packs_the_three_dtrs() {
    let ((answered, packed), transport) =
        light_source_read(Some(0xFF), &[Some(6), Some(2), Some(254)]);
    assert_eq!(answered, Some(0xFF));
    assert_eq!(packed, Some(0x0006_02FE));
    assert_script_consumed(&transport);
}

#[test]
fn a_mask_light_source_type_with_a_lost_dtr_reports_no_triple() {
    let ((answered, packed), transport) =
        light_source_read(Some(0xFF), &[Some(6), None, Some(254)]);
    assert_eq!(answered, Some(0xFF), "the MASK itself still stands");
    assert_eq!(packed, None);
    assert_script_consumed(&transport);
}

fn std_query_frame(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

#[test]
fn a_device_that_vanishes_mid_read_is_absent_at_the_silence_budget() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        std_query_frame(short, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for command in [
        StandardCommand::QueryVersionNumber,
        StandardCommand::QueryDeviceType,
        StandardCommand::QueryPhysicalMinimum,
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), None);
    }

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::Common102, DaliAttributeGroup::Groups];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    let error = read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect_err("three consecutive mandatory silences are a device leaving");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed(DEVICE_ABSENT_MESSAGE)
    );
    assert_eq!(outcomes.identity, AttributeGroupReadOutcome::Success);
    assert_eq!(outcomes.common_102, AttributeGroupReadOutcome::DeviceAbsent);
    assert_eq!(outcomes.groups, AttributeGroupReadOutcome::NotAttempted);
    assert_script_consumed(&transport);
}

#[test]
fn scattered_silences_answered_in_between_do_not_add_up_to_absence() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        std_query_frame(short, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for (command, answer) in [
        (StandardCommand::QueryVersionNumber, None),
        (StandardCommand::QueryDeviceType, Some(6)),
        (StandardCommand::QueryPhysicalMinimum, None),
        (StandardCommand::QueryMinLevel, Some(1)),
        (StandardCommand::QueryMaxLevel, None),
        (StandardCommand::QueryPowerOnLevel, Some(254)),
        (StandardCommand::QuerySystemFailureLevel, Some(254)),
        (StandardCommand::QueryFadeTimeFadeRate, Some(0x12)),
        (StandardCommand::QueryLightSourceType, Some(6)),
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), answer);
    }
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), Some(0x11));
    }

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::Common102];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect("scattered legal silences must not abort the read");
    assert_eq!(outcomes.common_102, AttributeGroupReadOutcome::Success);
    assert_script_consumed(&transport);
}

#[test]
fn legal_device_type_silence_never_feeds_the_breaker() {
    use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
    let mock = MockDaliTransport::new();
    let short = 17;
    let prelude = SpecialCommand::EnableDeviceType(6).to_forward_frame().raw();
    mock.expect_forward_frame_with_backward(
        std_query_frame(short, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), Some(0x11));
    }
    for command in [
        Dt6Command::QueryGearType,
        Dt6Command::QueryDimmingCurve,
        Dt6Command::QueryPossibleOperatingMode,
        Dt6Command::QueryFeatures,
        Dt6Command::QueryFailureStatus,
        Dt6Command::QueryShortCircuit,
        Dt6Command::QueryOpenCircuit,
        Dt6Command::QueryLoadDecrease,
        Dt6Command::QueryLoadIncrease,
        Dt6Command::QueryCurrentProtectorActive,
        Dt6Command::QueryThermalShutdown,
        Dt6Command::QueryThermalOverload,
        Dt6Command::QueryReferenceRunning,
        Dt6Command::QueryReferenceMeasurementFailed,
        Dt6Command::QueryCurrentProtectorEnabled,
        Dt6Command::QueryOperatingMode,
        Dt6Command::QueryFastFadeTime,
        Dt6Command::QueryMinFastFadeTime,
        Dt6Command::QueryExtendedVersionNumber,
    ] {
        mock.expect_forward_frame(prelude);
        let frame = DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt6(command),
        }
        .to_forward_frame()
        .raw();
        mock.expect_forward_frame_with_backward(frame, None);
    }

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::Dt6Led];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut outcomes,
    )
    .expect("19 legal DT6 silences are answers, not absence");
    assert_eq!(outcomes.dt6_led, AttributeGroupReadOutcome::Success);
    assert_script_consumed(&transport);
}

#[test]
fn contended_unanswered_mandatory_queries_do_not_count_toward_absence() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        std_query_frame(short, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for command in [
        StandardCommand::QueryVersionNumber,
        StandardCommand::QueryDeviceType,
        StandardCommand::QueryPhysicalMinimum,
    ] {
        mock.expect_query_no_answer_contended(std_query_frame(short, command));
    }
    for (command, answer) in [
        (StandardCommand::QueryMinLevel, Some(1)),
        (StandardCommand::QueryMaxLevel, Some(254)),
        (StandardCommand::QueryPowerOnLevel, Some(254)),
        (StandardCommand::QuerySystemFailureLevel, Some(254)),
        (StandardCommand::QueryFadeTimeFadeRate, Some(0x12)),
        (StandardCommand::QueryLightSourceType, Some(6)),
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), answer);
    }
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(std_query_frame(short, command), Some(0x11));
    }

    let transport = std::sync::Arc::new(std::sync::Mutex::new(mock));
    let mut controller = crate::runtime::controller::DaliController::with_retry_policy(
        std::sync::Arc::clone(&transport),
        Box::new(crate::runtime::clock::StdClock::new()),
        dali2rust_domain::dali::ses::RetryPolicy::default().with_query_contention_retry(false),
    );
    let requested = [DaliAttributeGroup::Common102];
    let mut outcomes = AttributeReadOutcomes::for_request(&requested, false);
    read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::new(false, 0),
        &mut outcomes,
    )
    .map_err(|e| {
        panic!(
            "contended silence is not this gear's silence: {e:?}; script_error={:?}",
            transport.lock().unwrap().script_error()
        )
    })
    .unwrap();
    assert_eq!(outcomes.common_102, AttributeGroupReadOutcome::Success);
    assert_script_consumed(&transport);
}

#[test]
fn read_attributes_confirms_contended_common102_scalar() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryControlGearPresent,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );
    for (command, reply) in [
        (StandardCommand::QueryVersionNumber, 0x08),
        (StandardCommand::QueryDeviceType, 0x08),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(reply),
        );
    }
    mock.expect_forward_frame_with_backward_contended(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryPhysicalMinimum,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );
    for _ in 0..2 {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryPhysicalMinimum,
            }
            .to_forward_frame()
            .raw(),
            Some(0x01),
        );
    }
    for (command, reply) in [
        (StandardCommand::QueryMinLevel, 0x01),
        (StandardCommand::QueryMaxLevel, 0xFE),
        (StandardCommand::QueryPowerOnLevel, 0x7F),
        (StandardCommand::QuerySystemFailureLevel, 0xFE),
        (StandardCommand::QueryFadeTimeFadeRate, 0x27),
        (StandardCommand::QueryLightSourceType, 6),
        (StandardCommand::QueryRandomAddressH, 0x12),
        (StandardCommand::QueryRandomAddressM, 0x34),
        (StandardCommand::QueryRandomAddressL, 0x56),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(reply),
        );
    }

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_attributes(
        &mut controller,
        short,
        &[DaliAttributeGroup::Common102],
        ContentConfirmPolicy::default(),
        &mut AttributeReadOutcomes::for_request(&[], false),
    )
    .expect("read attributes with content-confirm");

    assert_eq!(execution.c102.physical_minimum, Some(0x01));
    assert_eq!(execution.c102.fade_time_ms, Some(1000));
    assert_eq!(execution.c102.fade_rate, Some(7));
    assert_eq!(execution.random_address, Some(0x12_34_56));
    assert_script_consumed(&transport);
}

#[test]
fn read_attributes_does_not_fabricate_groups_or_scenes_without_readback() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryControlGearPresent,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );
    for (command, reply) in [
        (StandardCommand::QueryRandomAddressH, 0xAA),
        (StandardCommand::QueryRandomAddressM, 0xBB),
        (StandardCommand::QueryRandomAddressL, 0xCC),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(reply),
        );
    }
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryGroups0To7,
        }
        .to_forward_frame()
        .raw(),
        None,
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryGroups8To15,
        }
        .to_forward_frame()
        .raw(),
        Some(0x12),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QuerySceneLevel { scene: 0 },
        }
        .to_forward_frame()
        .raw(),
        None,
    );

    let (transport, mut controller) = setup_controller(mock);
    let execution = read_attributes(
        &mut controller,
        short,
        &[DaliAttributeGroup::Groups, DaliAttributeGroup::Scenes],
        ContentConfirmPolicy::default(),
        &mut AttributeReadOutcomes::for_request(&[], false),
    )
    .expect("read attributes");

    assert!(execution.has_groups);
    assert!(execution.has_scenes);
    assert_eq!(execution.groups_membership, None);
    assert_eq!(execution.scene_levels, None);
    assert_eq!(execution.random_address, Some(0xAA_BB_CC));
    assert_script_consumed(&transport);
}

#[test]
fn a_wide_colour_value_reads_both_halves_inside_one_transaction() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_TC, 1, 114);
    let (transport, mut controller) = setup_controller(mock);
    let counters = Arc::new(dali2rust_platform::dali::DaliWireCounters::default());
    controller.set_wire_counters(Arc::clone(&counters));

    let value = read_dt8_color_value_u16(
        &mut controller,
        short_address(short),
        DT8_COLOUR_VALUE_TC,
        ContentConfirmPolicy::default(),
    )
    .expect("exchange");

    assert_eq!(value, Some(370));
    assert_eq!(
        counters
            .transactions_completed
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the low byte left the bracket: two units means a yield window between \
         the halves of one number"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_narrow_colour_value_is_still_one_transaction_and_one_answer() {
    let mock = MockDaliTransport::new();
    let short = 4;
    expect_narrow_colour_value_sample(&mock, short, DT8_COLOUR_VALUE_RED, 254);
    let (transport, mut controller) = setup_controller(mock);
    let counters = Arc::new(dali2rust_platform::dali::DaliWireCounters::default());
    controller.set_wire_counters(Arc::clone(&counters));

    let value = read_dt8_color_value_u8(
        &mut controller,
        short_address(short),
        DT8_COLOUR_VALUE_RED,
        ContentConfirmPolicy::default(),
    )
    .expect("exchange");

    assert_eq!(value, Some(254), "a one-byte value answers whole (ISSUE-43)");
    assert_eq!(
        counters
            .transactions_completed
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_script_consumed(&transport);
}

fn expect_runtime_only_read(mock: &MockDaliTransport, short: u8, status: u8, level: u8) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryControlGearPresent,
        }
        .to_forward_frame()
        .raw(),
        Some(0xFF),
    );
    for (command, answer) in [
        (StandardCommand::QueryStatus, status),
        (StandardCommand::QueryActualLevel, level),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(answer),
        );
    }
}

fn expect_identity_triple(mock: &MockDaliTransport, short: u8) {
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(0x11),
        );
    }
}

fn expect_failure_status_query(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(6))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt6(Dt6Command::QueryFailureStatus),
        }
        .to_forward_frame()
        .raw(),
        answer,
    );
}

fn dt6_failure_frames(short: u8) -> (u16, u16) {
    (
        DaliCommand::Special(SpecialCommand::EnableDeviceType(6))
            .to_forward_frame()
            .raw(),
        DaliCommand::Extended {
            address: short_address(short),
            command: ExtendedCommand::Dt6(Dt6Command::QueryFailureStatus),
        }
        .to_forward_frame()
        .raw(),
    )
}

fn assert_no_dt6_failure_traffic(transport: &Arc<Mutex<MockDaliTransport>>, short: u8) {
    let (prelude, query) = dt6_failure_frames(short);
    let sent = transport.lock().expect("mock lock").sent_frames();
    assert!(
        !sent.contains(&prelude) && !sent.contains(&query),
        "expected no Part 207 frames, wire carried {sent:04X?}"
    );
}

fn assert_dt6_failure_traffic(transport: &Arc<Mutex<MockDaliTransport>>, short: u8) {
    let (prelude, query) = dt6_failure_frames(short);
    let sent = transport.lock().expect("mock lock").sent_frames();
    assert!(
        sent.contains(&prelude) && sent.contains(&query),
        "expected the prelude and command 241, wire carried {sent:04X?}"
    );
}

fn read_runtime_only(
    mock: MockDaliTransport,
    short: u8,
) -> (
    Arc<Mutex<MockDaliTransport>>,
    crate::runtime::executor::AttributeReadExecution,
) {
    let (transport, mut controller) = setup_controller(mock);
    let execution = read_attributes(
        &mut controller,
        short,
        &[DaliAttributeGroup::RuntimeStatus],
        ContentConfirmPolicy::default(),
        &mut AttributeReadOutcomes::for_request(&[DaliAttributeGroup::RuntimeStatus], false),
    )
    .expect("read attributes");
    assert_script_consumed(&transport);
    (transport, execution)
}

#[test]
fn a_lamp_failure_bit_escalates_to_the_part_207_failure_byte() {
    let short = 5;
    let mock = MockDaliTransport::new();
    expect_runtime_only_read(&mock, short, 0x02, 100);
    expect_identity_triple(&mock, short);
    expect_failure_status_query(&mock, short, Some(0x21));

    let (transport, execution) = read_runtime_only(mock, short);
    assert_dt6_failure_traffic(&transport, short);

    let dt6 = execution.dt6.expect("the failure byte was read");
    assert_eq!(dt6.failure_status, Some(0x21));
    assert_eq!(dt6.short_circuit, Some(0xFF), "bit 0 is a YES");
    assert_eq!(dt6.thermal_shutdown, Some(0xFF), "bit 5 is a YES");
    assert_eq!(dt6.open_circuit, Some(0), "an unset bit is the gear saying no");
    assert_eq!(
        dt6.reference_running, None,
        "249 is a state, not a fault, and the byte does not carry it — so it          must stay unread rather than be invented as a no"
    );
    assert!(
        !execution.has_dt6_led,
        "the section was not requested; only the byte was read"
    );
}

#[test]
fn a_healthy_status_spends_no_part_207_frames() {
    let short = 6;
    let mock = MockDaliTransport::new();
    expect_runtime_only_read(&mock, short, 0x04, 200);
    expect_identity_triple(&mock, short);

    let (transport, execution) = read_runtime_only(mock, short);

    assert_no_dt6_failure_traffic(&transport, short);
    assert!(execution.dt6.is_none(), "nothing was read, so nothing is published");
    let setpoint = execution.runtime_setpoint.expect("runtime section ran");
    assert_eq!(setpoint.level, 200);
    assert_eq!(setpoint.power, PowerState::On);
}

#[test]
fn a_mask_actual_level_states_no_level_and_asks_part_207_why() {
    let short = 7;
    let mock = MockDaliTransport::new();
    expect_runtime_only_read(&mock, short, 0x00, 0xFF);
    expect_identity_triple(&mock, short);
    expect_failure_status_query(&mock, short, Some(0x20));

    let (transport, execution) = read_runtime_only(mock, short);
    assert_dt6_failure_traffic(&transport, short);

    let setpoint = execution.runtime_setpoint.expect("runtime section ran");
    assert_eq!(setpoint.level, 0, "MASK is not 255 and not a level");
    assert_eq!(
        setpoint.power,
        PowerState::Unknown,
        "the registry keeps its stored level only while the setpoint states nothing"
    );
    assert!(
        execution.runtime_observation.is_some(),
        "the status byte is a real observation and must still be published"
    );
    assert_eq!(
        execution.dt6.and_then(|d| d.failure_status),
        Some(0x20),
        "thermal shut down, read through the one frame that reports it"
    );
}

#[test]
fn a_gear_that_does_not_speak_part_207_publishes_no_failure_byte() {
    let short = 8;
    let mock = MockDaliTransport::new();
    expect_runtime_only_read(&mock, short, 0x02, 100);
    expect_identity_triple(&mock, short);
    expect_failure_status_query(&mock, short, None);

    let (transport, execution) = read_runtime_only(mock, short);
    assert_dt6_failure_traffic(&transport, short);

    assert!(
        execution.dt6.is_none(),
        "silence is an answer about the device type, not about its health"
    );
}

#[test]
fn a_known_device_type_set_without_dt6_skips_the_escalation() {
    let short = 9;
    let mock = MockDaliTransport::new();
    expect_runtime_only_read(&mock, short, 0x02, 100);
    for (command, answer) in [
        (StandardCommand::QueryVersionNumber, 2u8),
        (StandardCommand::QueryDeviceType, 8),
        (StandardCommand::QueryPhysicalMinimum, 10),
        (StandardCommand::QueryMinLevel, 10),
        (StandardCommand::QueryMaxLevel, 254),
        (StandardCommand::QueryPowerOnLevel, 254),
        (StandardCommand::QuerySystemFailureLevel, 254),
        (StandardCommand::QueryFadeTimeFadeRate, 0),
        (StandardCommand::QueryLightSourceType, 6),
    ] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command,
            }
            .to_forward_frame()
            .raw(),
            Some(answer),
        );
    }
    expect_identity_triple(&mock, short);

    let (transport, mut controller) = setup_controller(mock);
    let requested = [DaliAttributeGroup::RuntimeStatus, DaliAttributeGroup::Common102];
    let execution = read_attributes(
        &mut controller,
        short,
        &requested,
        ContentConfirmPolicy::default(),
        &mut AttributeReadOutcomes::for_request(&requested, false),
    )
    .expect("read attributes");
    assert_script_consumed(&transport);

    assert_no_dt6_failure_traffic(&transport, short);
    assert!(
        execution.dt6.is_none(),
        "a lamp failure on a gear that declares only DT8 buys no Part 207 frames"
    );
}
