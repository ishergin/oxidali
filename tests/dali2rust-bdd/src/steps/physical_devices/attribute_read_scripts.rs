use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::dt8_color::Dt8Command;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::steps::frames::{extended_frame, extended_version_query_frame, special_frame, standard_frame};

use super::discovery_scripts::{
    script_detect_dt8_cct, script_detect_dt8_with_features_and_status, DT8_FEATURES_RGB_CAPABLE,
};
use super::memory_bank_scripts::{
    script_memory_bank_read, script_memory_bank_short_read, BANK0_BYTES, BANK1_BYTES,
    SHORT_BANK0_BYTES,
};
use super::TEST_SHORT_ADDRESS;

#[derive(Clone, Copy)]
pub(super) struct ScriptedReply {
    pub(super) value: Option<u8>,
    pub(super) contended: bool,
}

fn expect_scripted_reply(mock: &MockDaliTransport, frame: u16, reply: ScriptedReply) {
    if reply.contended {
        mock.expect_forward_frame_with_backward_contended(frame, reply.value);
    } else {
        mock.expect_forward_frame_with_backward(frame, reply.value);
    }
}

pub(crate) fn script_attribute_read_prelude(mock: &MockDaliTransport, short: u8) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(short, command), Some(0xFF));
    }
}

pub(crate) fn script_groups_attribute_read(mock: &MockDaliTransport, short: u8, mask: u16) {
    mock.clear();
    script_attribute_read_prelude(mock, short);
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryGroups0To7),
        Some((mask & 0xFF) as u8),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryGroups8To15),
        Some((mask >> 8) as u8),
    );
}

fn script_attribute_read_identity_prelude(mock: &MockDaliTransport) {
    script_attribute_read_identity_queries(mock, &[ScriptedReply { value: Some(0x01), contended: false }]);
}

fn script_attribute_read_identity_common(
    mock: &MockDaliTransport,
    physical_minimum_reads: &[ScriptedReply],
) {
    script_attribute_read_identity_queries(mock, physical_minimum_reads);
    script_memory_bank_read(mock, 0, &BANK0_BYTES[..3]);
    script_memory_bank_read(mock, 0, &BANK0_BYTES);
    script_memory_bank_read(mock, 1, &BANK1_BYTES[..1]);
    script_memory_bank_read(mock, 1, &BANK1_BYTES);
}

const DT8_STATUS_CCT_ACTIVE: u8 = 0x20;

const DT8_STATUS_RGBWAF_ACTIVE: u8 = 0x80;

pub(super) const SCRIPTED_RUNTIME_STATUS: u8 = 0x84;

const SCRIPTED_RUNTIME_LEVEL: u8 = 0x7F;

const GOLDEN_TC_MIREK: u16 = 0x5678;

pub(super) const PD165_TC_MIREK: u16 = 250;

pub(super) const GOLDEN_GEAR_FEATURES: u8 = 0x41;

const GOLDEN_RGBWAF_CONTROL: u8 = 0x80;

fn script_attribute_read_probe_and_runtime(mock: &MockDaliTransport) {
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    script_detect_dt8_cct(mock, TEST_SHORT_ADDRESS);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryStatus),
        Some(SCRIPTED_RUNTIME_STATUS),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryActualLevel),
        Some(SCRIPTED_RUNTIME_LEVEL),
    );
}

fn script_attribute_read_random_address(mock: &MockDaliTransport) {
    for (command, reply) in [
        (StandardCommand::QueryRandomAddressH, 0x5C),
        (StandardCommand::QueryRandomAddressM, 0x1D),
        (StandardCommand::QueryRandomAddressL, 0xC2),
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(TEST_SHORT_ADDRESS, command), Some(reply));
    }
}

pub(super) fn script_dt8_colour_value(mock: &MockDaliTransport, value_id: u8, msb: u8, lsb: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(value_id)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(value_id),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourValue)),
        Some(msb),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(lsb),
    );
}

fn script_dt8_colour_section(
    mock: &MockDaliTransport,
    status: u8,
    tc_mirek: u16,
    gear_features: Option<u8>,
) {
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourStatus)),
        Some(status),
    );
    script_dt8_colour_value(mock, 0, 0x00, 0x80);
    script_dt8_colour_value(mock, 1, 0x12, 0x34);
    script_dt8_colour_value(mock, 2, (tc_mirek >> 8) as u8, tc_mirek as u8);
    script_dt8_colour_value(mock, 128, 0x00, 153);
    script_dt8_colour_value(mock, 130, 0x01, 114);
    script_dt8_gear_features(mock, gear_features);
}

fn script_dt8_gear_features(mock: &MockDaliTransport, answer: Option<u8>) {
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(
            TEST_SHORT_ADDRESS,
            ExtendedCommand::Dt8(Dt8Command::QueryGearFeaturesStatus),
        ),
        answer,
    );
}

pub(super) fn script_attribute_read_runtime_and_colour(
    mock: &MockDaliTransport,
    tc_mirek: u16,
    gear_features: Option<u8>,
) {
    mock.clear();
    script_attribute_read_probe_and_runtime(mock);
    script_dt8_colour_section(mock, DT8_STATUS_CCT_ACTIVE, tc_mirek, gear_features);
    script_attribute_read_random_address(mock);
}

fn script_dt8_narrow_colour_value(mock: &MockDaliTransport, value_id: u8, answer: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(value_id)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(value_id),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourValue)),
        Some(answer),
    );
}

fn script_dt8_colour_value_unanswered(mock: &MockDaliTransport, value_id: u8) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(value_id)));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(value_id),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourValue)),
        None,
    );
}

pub(super) fn script_attribute_read_no_probe(mock: &MockDaliTransport) {
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryStatus),
        Some(SCRIPTED_RUNTIME_STATUS),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryActualLevel),
        Some(SCRIPTED_RUNTIME_LEVEL),
    );
    script_attribute_read_random_address(mock);
}

pub(super) fn script_attribute_read_faulted_gear(
    mock: &MockDaliTransport,
    status: u8,
    level: u8,
    failure_byte: u8,
) {
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryStatus),
        Some(status),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryActualLevel),
        Some(level),
    );
    script_attribute_read_random_address(mock);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(6)));
    mock.expect_forward_frame_with_backward(
        extended_frame(
            TEST_SHORT_ADDRESS,
            ExtendedCommand::Dt6(Dt6Command::QueryFailureStatus),
        ),
        Some(failure_byte),
    );
}

pub(super) fn script_attribute_read_rgb_active(mock: &MockDaliTransport, rgb: (u8, u8, u8)) {
    script_attribute_read_rgb_active_with_status(mock, rgb, SCRIPTED_RUNTIME_STATUS);
}

pub(super) fn script_attribute_read_rgb_active_with_status(
    mock: &MockDaliTransport,
    rgb: (u8, u8, u8),
    status: u8,
) {
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    script_detect_dt8_with_features_and_status(
        mock,
        TEST_SHORT_ADDRESS,
        DT8_FEATURES_RGB_CAPABLE,
        DT8_STATUS_RGBWAF_ACTIVE,
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryStatus),
        Some(status),
    );
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryActualLevel),
        Some(SCRIPTED_RUNTIME_LEVEL),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourStatus)),
        Some(DT8_STATUS_RGBWAF_ACTIVE),
    );
    script_dt8_colour_value(mock, 0, 0x00, 0x80);
    script_dt8_colour_value(mock, 1, 0x12, 0x34);
    script_dt8_colour_value(mock, 2, 0x00, 0xFA);
    script_dt8_narrow_colour_value(mock, 9, rgb.0);
    script_dt8_narrow_colour_value(mock, 10, rgb.1);
    script_dt8_narrow_colour_value(mock, 11, rgb.2);
    script_dt8_narrow_colour_value(mock, 12, 0);
    script_dt8_narrow_colour_value(mock, 13, 0);
    script_dt8_narrow_colour_value(mock, 14, 0);
    script_dt8_colour_value(mock, 128, 0x00, 153);
    script_dt8_colour_value(mock, 130, 0x01, 114);
    script_dt8_gear_features(mock, Some(GOLDEN_GEAR_FEATURES));
    script_dt8_rgbwaf_control(mock, Some(GOLDEN_RGBWAF_CONTROL));
    script_attribute_read_random_address(mock);
}

fn script_dt8_rgbwaf_control(mock: &MockDaliTransport, answer: Option<u8>) {
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(
            TEST_SHORT_ADDRESS,
            ExtendedCommand::Dt8(Dt8Command::QueryRgbwafControl),
        ),
        answer,
    );
}

pub(super) fn script_attribute_read_colour_unanswered(mock: &MockDaliTransport) {
    mock.clear();
    script_attribute_read_probe_and_runtime(mock);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourStatus)),
        Some(DT8_STATUS_CCT_ACTIVE),
    );
    script_dt8_colour_value(mock, 0, 0x00, 0x80);
    script_dt8_colour_value(mock, 1, 0x12, 0x34);
    script_dt8_colour_value_unanswered(mock, 2);
    script_dt8_colour_value(mock, 128, 0x00, 153);
    script_dt8_colour_value(mock, 130, 0x01, 114);
    script_dt8_gear_features(mock, Some(GOLDEN_GEAR_FEATURES));
    script_attribute_read_random_address(mock);
}

pub(super) fn script_attribute_read_identity_queries(
    mock: &MockDaliTransport,
    physical_minimum_reads: &[ScriptedReply],
) {
    mock.clear();
    script_attribute_read_probe_and_runtime(mock);
    for (command, reply) in [
        (StandardCommand::QueryVersionNumber, 0x08),
        (StandardCommand::QueryDeviceType, 0x08),
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(TEST_SHORT_ADDRESS, command), Some(reply));
    }
    for reply in physical_minimum_reads {
        expect_scripted_reply(
            mock,
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryPhysicalMinimum),
            *reply,
        );
    }
    for (command, reply) in [
        (StandardCommand::QueryMinLevel, 0x01),
        (StandardCommand::QueryMaxLevel, 0xFE),
        (StandardCommand::QueryPowerOnLevel, 0x7F),
        (StandardCommand::QuerySystemFailureLevel, 0xFE),
        (StandardCommand::QueryFadeTimeFadeRate, 0x27),
        (StandardCommand::QueryLightSourceType, 0x06),
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(TEST_SHORT_ADDRESS, command), Some(reply));
    }
    script_dt8_colour_section(mock, 0x00, GOLDEN_TC_MIREK, Some(GOLDEN_GEAR_FEATURES));
    script_attribute_read_random_address(mock);
}

pub(super) fn script_attribute_read_identity(mock: &MockDaliTransport) {
    script_attribute_read_identity_common(
        mock,
        &[ScriptedReply {
            value: Some(0x01),
            contended: false,
        }],
    );
}

pub(super) fn script_attribute_read_identity_short_bank0(mock: &MockDaliTransport) {
    script_attribute_read_identity_prelude(mock);
    script_memory_bank_read(mock, 0, &BANK0_BYTES[..3]);
    script_memory_bank_short_read(mock, 0, &BANK0_BYTES[..SHORT_BANK0_BYTES]);
    script_memory_bank_read(mock, 1, &BANK1_BYTES[..1]);
    script_memory_bank_read(mock, 1, &BANK1_BYTES);
}

pub(super) fn script_attribute_read_identity_with_content_confirm(mock: &MockDaliTransport) {
    script_attribute_read_identity_common(
        mock,
        &[
            ScriptedReply {
                value: Some(0xFF),
                contended: true,
            },
            ScriptedReply {
                value: Some(0x01),
                contended: false,
            },
            ScriptedReply {
                value: Some(0x01),
                contended: false,
            },
        ],
    );
}

pub(super) fn script_extended_section(
    mock: &MockDaliTransport,
    fade_answers: &[u8],
    device_type_walk: &[u8],
    versions: &[(u8, Option<u8>)],
) {
    mock.clear();
    script_attribute_read_prelude(mock, TEST_SHORT_ADDRESS);
    for answer in fade_answers {
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryExtendedFadeTime),
            Some(*answer),
        );
    }
    let (first, rest) = device_type_walk.split_first().expect("a device-type answer");
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
        Some(*first),
    );
    for next in rest {
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryNextDeviceType),
            Some(*next),
        );
    }
    for (device_type, answer) in versions {
        mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(*device_type)));
        mock.expect_forward_frame_with_backward(
            extended_version_query_frame(TEST_SHORT_ADDRESS),
            *answer,
        );
    }
}
