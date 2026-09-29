use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::steps::frames::{dt8_raw_query_frame, special_frame, standard_frame};

use super::{TEST_RANDOM_ADDRESS, TEST_SHORT_ADDRESS};

pub(crate) fn script_detect_dt8_cct(mock: &MockDaliTransport, short: u8) {
    script_detect_dt8_with_features(mock, short, 0x02);
}

pub(crate) fn script_detect_dt8_with_features(mock: &MockDaliTransport, short: u8, features: u8) {
    script_detect_dt8_with_features_and_status(mock, short, features, 0x20);
}

pub(crate) fn script_detect_dt8_with_features_and_status(
    mock: &MockDaliTransport,
    short: u8,
    features: u8,
    status: u8,
) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryDeviceType),
        Some(0),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(dt8_raw_query_frame(short, 0xF9), Some(features));
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(dt8_raw_query_frame(short, 0xF8), Some(status));
}

fn script_detect_multi_dt_mask_cct(mock: &MockDaliTransport, short: u8) {
    mock.expect_forward_frame_with_backward(
        standard_frame(short, StandardCommand::QueryDeviceType),
        Some(0xFF),
    );
    for answer in [Some(6), Some(8), Some(254)] {
        mock.expect_forward_frame_with_backward(
            standard_frame(short, StandardCommand::QueryNextDeviceType),
            answer,
        );
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(dt8_raw_query_frame(short, 0xF9), Some(0x02));
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(dt8_raw_query_frame(short, 0xF8), Some(0x20));
}

fn script_discovery_random_address_once(mock: &MockDaliTransport, short: u8, random_address: u32) {
    for (command, reply) in [
        (
            StandardCommand::QueryRandomAddressH,
            ((random_address >> 16) & 0xFF) as u8,
        ),
        (
            StandardCommand::QueryRandomAddressM,
            ((random_address >> 8) & 0xFF) as u8,
        ),
        (
            StandardCommand::QueryRandomAddressL,
            (random_address & 0xFF) as u8,
        ),
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(short, command), Some(reply));
    }
}

fn script_discovery_random_address_reads(
    mock: &MockDaliTransport,
    short: u8,
    random_addresses: &[u32],
) {
    for &random_address in random_addresses {
        script_discovery_random_address_once(mock, short, random_address);
    }
}

fn script_discovery_search_address(mock: &MockDaliTransport, random_address: u32) {
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(
        ((random_address >> 16) & 0xFF) as u8,
    )));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(
        ((random_address >> 8) & 0xFF) as u8,
    )));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(
        (random_address & 0xFF) as u8,
    )));
}

fn script_discovery_verify_attempt(
    mock: &MockDaliTransport,
    random_address: u32,
    reply: Option<u8>,
) {
    script_discovery_search_address(mock, random_address);
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::QueryShortAddress), reply);
}

fn script_discovery_verify_attempt_violating(mock: &MockDaliTransport, random_address: u32) {
    script_discovery_search_address(mock, random_address);
    mock.expect_forward_frame_corrupted_in_window(special_frame(SpecialCommand::QueryShortAddress));
}

pub(crate) fn script_discovery(mock: &MockDaliTransport) {
    script_discovery_with_features(mock, 0x02);
}

pub(crate) fn script_six_channel_discovery(mock: &MockDaliTransport) {
    script_discovery_with_features(mock, DT8_FEATURES_RGB_CAPABLE);
}

pub(crate) fn script_discovery_with_features(mock: &MockDaliTransport, features: u8) {
    script_scan_discovery(mock, &[(TEST_SHORT_ADDRESS, TEST_RANDOM_ADDRESS)], features);
}

fn script_presence_sweep(mock: &MockDaliTransport, present: &[(u8, u32)]) {
    for short in 0..=63u8 {
        let response = present.iter().any(|(s, _)| *s == short).then_some(0xFF);
        mock.expect_forward_frame_with_backward(
            standard_frame(short, StandardCommand::QueryControlGearPresent),
            response,
        );
    }
}

fn script_verify_session(mock: &MockDaliTransport, present: &[(u8, u32)]) {
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    for &(short, random_address) in present {
        script_discovery_verify_attempt(mock, random_address, Some((short << 1) | 0x01));
        mock.expect_forward_frame(special_frame(SpecialCommand::Withdraw));
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
}

pub(crate) fn script_scan_discovery(
    mock: &MockDaliTransport,
    present: &[(u8, u32)],
    features: u8,
) {
    let featured: Vec<(u8, u32, u8)> = present
        .iter()
        .map(|&(short, random_address)| (short, random_address, features))
        .collect();
    script_scan_discovery_per_device(mock, &featured);
}

pub(super) fn script_scan_discovery_per_device(
    mock: &MockDaliTransport,
    present: &[(u8, u32, u8)],
) {
    let pairs: Vec<(u8, u32)> = present
        .iter()
        .map(|&(short, random_address, _)| (short, random_address))
        .collect();
    mock.clear();
    script_presence_sweep(mock, &pairs);
    for &(short, random_address) in &pairs {
        script_discovery_random_address_reads(mock, short, &[random_address, random_address]);
    }
    script_verify_session(mock, &pairs);
    for &(short, _, features) in present {
        script_detect_dt8_with_features(mock, short, features);
    }
}

pub(super) fn script_discovery_multi_dt_mask(mock: &MockDaliTransport) {
    script_discovery_one_verified_device(mock);
    script_detect_multi_dt_mask_cct(mock, TEST_SHORT_ADDRESS);
}

fn script_discovery_one_verified_device_prelude(mock: &MockDaliTransport) {
    mock.clear();
    for short in 0..=63u8 {
        let response = (short == TEST_SHORT_ADDRESS).then_some(0xFF);
        mock.expect_forward_frame_with_backward(
            standard_frame(short, StandardCommand::QueryControlGearPresent),
            response,
        );
    }
    script_discovery_random_address_reads(
        mock,
        TEST_SHORT_ADDRESS,
        &[TEST_RANDOM_ADDRESS, TEST_RANDOM_ADDRESS],
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
}

fn script_discovery_one_verified_device(mock: &MockDaliTransport) {
    script_discovery_one_verified_device_prelude(mock);
    script_discovery_verify_attempt(mock, TEST_RANDOM_ADDRESS, Some(0x01));
    mock.expect_forward_frame(special_frame(SpecialCommand::Withdraw));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
}

pub(super) fn script_discovery_declares_only_dt6(mock: &MockDaliTransport) {
    script_discovery_one_verified_device(mock);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
        Some(6),
    );
}

pub(super) fn script_discovery_unterminated_type_walk(mock: &MockDaliTransport) {
    script_discovery_one_verified_device(mock);
    for _ in 0..2 {
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
            Some(0xFF),
        );
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryNextDeviceType),
            None,
        );
    }
    for _ in 0..2 {
        mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
        mock.expect_forward_frame_with_backward(dt8_raw_query_frame(TEST_SHORT_ADDRESS, 0xF9), None);
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(dt8_raw_query_frame(TEST_SHORT_ADDRESS, 0xF8), None);
}

pub(super) fn script_discovery_no_answers(mock: &MockDaliTransport) {
    mock.clear();
    for short in 0..=63u8 {
        mock.expect_forward_frame_with_backward(
            standard_frame(short, StandardCommand::QueryControlGearPresent),
            None,
        );
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
}

pub(super) fn script_discovery_partial_failure(mock: &MockDaliTransport) {
    mock.clear();
    for short in 0..=63u8 {
        let response = (short <= 1).then_some(0xFF);
        mock.expect_forward_frame_with_backward(
            standard_frame(short, StandardCommand::QueryControlGearPresent),
            response,
        );
    }
    script_discovery_random_address_reads(mock, 0, &[TEST_RANDOM_ADDRESS, TEST_RANDOM_ADDRESS]);
    script_discovery_random_address_reads(mock, 1, &[0x00C8_E731, 0x00C8_E731]);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    mock.expect_forward_frame(special_frame(SpecialCommand::Initialise(0x00)));
    script_discovery_verify_attempt(mock, TEST_RANDOM_ADDRESS, Some(0x01));
    mock.expect_forward_frame(special_frame(SpecialCommand::Withdraw));
    script_discovery_verify_attempt(mock, 0x00C8_E731, None);
    script_discovery_random_address_reads(mock, 1, &[0x00C8_E731, 0x00C8_E731]);
    script_discovery_verify_attempt(mock, 0x00C8_E731, None);
    script_discovery_random_address_reads(mock, 1, &[0x00C8_E731, 0x00C8_E731]);
    script_discovery_verify_attempt(mock, 0x00C8_E731, None);
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    script_detect_dt8_cct(mock, TEST_SHORT_ADDRESS);
}

const DISCOVERY_VERIFY_ATTEMPTS: usize = 3;

pub(super) fn script_discovery_permanent_multiple_responders(mock: &MockDaliTransport) {
    script_discovery_one_verified_device_prelude(mock);
    for attempt in 0..DISCOVERY_VERIFY_ATTEMPTS {
        if attempt > 0 {
            script_discovery_random_address_reads(
                mock,
                TEST_SHORT_ADDRESS,
                &[TEST_RANDOM_ADDRESS, TEST_RANDOM_ADDRESS],
            );
        }
        script_discovery_verify_attempt_violating(mock, TEST_RANDOM_ADDRESS);
    }
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
}

pub(super) fn script_discovery_corrupted_window_retry(mock: &MockDaliTransport) {
    script_discovery_one_verified_device_prelude(mock);
    script_discovery_verify_attempt_violating(mock, TEST_RANDOM_ADDRESS);
    script_discovery_random_address_reads(mock, TEST_SHORT_ADDRESS, &[TEST_RANDOM_ADDRESS, TEST_RANDOM_ADDRESS]);
    script_discovery_verify_attempt(mock, TEST_RANDOM_ADDRESS, Some(0x01));
    mock.expect_forward_frame(special_frame(SpecialCommand::Withdraw));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrH(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrM(0xFF)));
    mock.expect_forward_frame(special_frame(SpecialCommand::SearchAddrL(0xFF)));
    mock.expect_forward_frame_with_backward(special_frame(SpecialCommand::Compare), None);
    mock.expect_forward_frame(special_frame(SpecialCommand::Terminate));
    script_detect_dt8_cct(mock, TEST_SHORT_ADDRESS);
}

pub(super) const DT8_FEATURES_RGB_CAPABLE: u8 = 0xC2;
