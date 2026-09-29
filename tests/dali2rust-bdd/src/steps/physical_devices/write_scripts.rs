use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::dt8_color::Dt8Command;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::steps::frames::{extended_frame, special_frame, standard_frame};

use super::attribute_read_scripts::script_dt8_colour_value;
use super::TEST_SHORT_ADDRESS;

pub(crate) fn script_staged_dtrs(mock: &MockDaliTransport, short: u8, values: &[u8]) {
    const DTR_WRITE: [fn(u8) -> SpecialCommand; 3] = [
        SpecialCommand::Dtr0,
        SpecialCommand::Dtr1,
        SpecialCommand::Dtr2,
    ];
    const DTR_QUERY: [StandardCommand; 3] = [
        StandardCommand::QueryContentDtr0,
        StandardCommand::QueryContentDtr1,
        StandardCommand::QueryContentDtr2,
    ];
    for (write, value) in DTR_WRITE.iter().zip(values) {
        mock.expect_forward_frame(special_frame(write(*value)));
    }
    for (query, value) in DTR_QUERY.iter().zip(values) {
        mock.expect_forward_frame_with_backward(standard_frame(short, *query), Some(*value));
    }
}

pub(super) fn script_fade_time_write(mock: &MockDaliTransport, dtr0: u8) {
    script_fade_time_write_answered(mock, dtr0, Some(dtr0 << 4));
}

pub(super) fn script_fade_time_write_answered(mock: &MockDaliTransport, dtr0: u8, answer: Option<u8>) {
    mock.clear();
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(dtr0)));
    let set_fade = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::SetFadeTime);
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryFadeTimeFadeRate),
        answer,
    );
}

fn script_level_bound_triple(
    mock: &MockDaliTransport,
    set: StandardCommand,
    query: StandardCommand,
    dtr0: u8,
    answer: u8,
) {
    script_addressed_config_triple(mock, TEST_SHORT_ADDRESS, set, query, dtr0, answer);
}

pub(crate) fn script_addressed_config_triple(
    mock: &MockDaliTransport,
    short: u8,
    set: StandardCommand,
    query: StandardCommand,
    dtr0: u8,
    answer: u8,
) {
    script_addressed_config_triple_answered(mock, short, set, query, dtr0, Some(answer));
}

pub(crate) fn script_addressed_config_triple_answered(
    mock: &MockDaliTransport,
    short: u8,
    set: StandardCommand,
    query: StandardCommand,
    dtr0: u8,
    answer: Option<u8>,
) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(dtr0)));
    let set_frame = standard_frame(short, set);
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame_with_backward(standard_frame(short, query), answer);
}

pub(super) fn script_min_triple(mock: &MockDaliTransport, dtr0: u8, answer: u8) {
    script_level_bound_triple(
        mock,
        StandardCommand::SetMinLevel,
        StandardCommand::QueryMinLevel,
        dtr0,
        answer,
    );
}

pub(super) fn script_max_triple(mock: &MockDaliTransport, dtr0: u8, answer: u8) {
    script_level_bound_triple(
        mock,
        StandardCommand::SetMaxLevel,
        StandardCommand::QueryMaxLevel,
        dtr0,
        answer,
    );
}

pub(super) fn script_dimming_curve_write(mock: &MockDaliTransport, curve: u8, answer: Option<u8>) {
    mock.clear();
    script_dimming_curve_write_no_clear(mock, curve, answer);
}

pub(super) fn script_dimming_curve_write_no_clear(mock: &MockDaliTransport, curve: u8, answer: Option<u8>) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(curve)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(curve),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(6)));
    let select = extended_frame(
        TEST_SHORT_ADDRESS,
        ExtendedCommand::Dt6(Dt6Command::SelectDimmingCurve),
    );
    mock.expect_forward_frame(select);
    mock.expect_forward_frame(select);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(6)));
    mock.expect_forward_frame_with_backward(
        extended_frame(
            TEST_SHORT_ADDRESS,
            ExtendedCommand::Dt6(Dt6Command::QueryDimmingCurve),
        ),
        answer,
    );
}

pub(super) fn script_tc_limit_physical_pair(mock: &MockDaliTransport) {
    script_dt8_colour_value(mock, 129, 0x00, 153);
    script_dt8_colour_value(mock, 131, 0x01, 0x72);
}

pub(super) fn script_tc_limit_store_frames(mock: &MockDaliTransport, selector: u8, mirek: u16) {
    let proofs = [
        (
            special_frame(SpecialCommand::Dtr0(mirek as u8)),
            StandardCommand::QueryContentDtr0,
            mirek as u8,
        ),
        (
            special_frame(SpecialCommand::Dtr1((mirek >> 8) as u8)),
            StandardCommand::QueryContentDtr1,
            (mirek >> 8) as u8,
        ),
        (
            special_frame(SpecialCommand::Dtr2(selector)),
            StandardCommand::QueryContentDtr2,
            selector,
        ),
    ];
    for (arm, prove, echo) in proofs {
        mock.expect_forward_frame(arm);
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, prove),
            Some(echo),
        );
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    let store = extended_frame(
        TEST_SHORT_ADDRESS,
        ExtendedCommand::Dt8(Dt8Command::StoreColourTemperatureTcLimit),
    );
    mock.expect_forward_frame(store);
    mock.expect_forward_frame(store);
}
