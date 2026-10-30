use super::*;
use crate::runtime::executor::test_helpers::shared::{
    assert_script_consumed, setup_controller, short_address, short_raw_query_frame,
    wire_counters,
};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::controller::WireHold;
use dali2rust_domain::dali::frame::HoldCause;
use dali2rust_domain::dali::commands::DaliCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_contracts::msg::ErrorCode;
use dali2rust_platform::dali::DaliWireCounters;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn declared(types: &[u8]) -> Option<DeviceTypeSet> {
    let mut set = DeviceTypeSet::default();
    for device_type in types {
        assert!(set.insert(*device_type), "type {device_type} is not declarable");
    }
    Some(set)
}

#[test]
fn an_unfinished_enumeration_declares_nothing_rather_than_an_empty_set() {
    let answered_254 = SupportedDeviceTypesQuery {
        answered: true,
        supported: BTreeSet::new(),
        enum_degraded: false,
    };
    assert_eq!(
        declared_device_types(&answered_254),
        Some(DeviceTypeSet::default())
    );

    let never_answered = SupportedDeviceTypesQuery::default();
    assert_eq!(declared_device_types(&never_answered), None);

    let degraded = SupportedDeviceTypesQuery {
        answered: true,
        supported: BTreeSet::new(),
        enum_degraded: true,
    };
    assert_eq!(declared_device_types(&degraded), None);
}

#[test]
fn one_query_device_type_frame_settles_every_case_but_mask() {
    assert_eq!(
        declared_device_types_from_single_answer(Some(DEVICE_TYPE_NONE)),
        Some(DeviceTypeSet::default())
    );
    assert_eq!(
        declared_device_types_from_single_answer(Some(6)),
        declared(&[6])
    );
    assert_eq!(
        declared_device_types_from_single_answer(Some(DEVICE_TYPE_MASK)),
        None
    );
    assert_eq!(declared_device_types_from_single_answer(None), None);
}

fn script_present_short_addresses(mock: &MockDaliTransport, present: &[u8]) {
    for short in 0..=63u8 {
        let response = present.contains(&short).then_some(0xFF);
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryControlGearPresent,
            }
            .to_forward_frame()
            .raw(),
            response,
        );
    }
}

fn script_random_address_once(mock: &MockDaliTransport, short: u8, random_address: u32) {
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
}

fn script_random_address_unanswered_h(mock: &MockDaliTransport, short: u8) {
    script_random_address_query(mock, short, StandardCommand::QueryRandomAddressH, None);
}

fn script_random_address_unanswered_m(mock: &MockDaliTransport, short: u8, high: u8) {
    script_random_address_query(
        mock,
        short,
        StandardCommand::QueryRandomAddressH,
        Some(high),
    );
    script_random_address_query(mock, short, StandardCommand::QueryRandomAddressM, None);
}

fn script_random_address_query(
    mock: &MockDaliTransport,
    short: u8,
    command: StandardCommand,
    reply: Option<u8>,
) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command,
        }
        .to_forward_frame()
        .raw(),
        reply,
    );
}

fn script_random_address_reads(
    mock: &MockDaliTransport,
    short: u8,
    random_addresses: &[u32],
) {
    for &random_address in random_addresses {
        script_random_address_once(mock, short, random_address);
    }
}

fn script_verify_session_begin(mock: &MockDaliTransport) {
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Terminate).to_forward_frame().raw());
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Initialise(0x00))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::Initialise(0x00))
            .to_forward_frame()
            .raw(),
    );
}

fn script_verify_attempt(
    mock: &MockDaliTransport,
    random_address: u32,
    reply: Option<u8>,
) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrH(((random_address >> 16) & 0xFF) as u8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrM(((random_address >> 8) & 0xFF) as u8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrL((random_address & 0xFF) as u8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Special(SpecialCommand::QueryShortAddress)
            .to_forward_frame()
            .raw(),
        reply,
    );
}

fn script_verify_session_end(mock: &MockDaliTransport) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrH(0xFF))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrM(0xFF))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::SearchAddrL(0xFF))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        DaliCommand::Special(SpecialCommand::Compare)
            .to_forward_frame()
            .raw(),
        None,
    );
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Terminate).to_forward_frame().raw());
}

fn script_describe_dt6(mock: &MockDaliTransport, short: u8) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        Some(6),
    );
}

#[test]
fn read_random_address_stable_reconstructs_per_byte_majority() {
    let mock = MockDaliTransport::new();
    let short = 17;
    script_random_address_reads(&mock, short, &[0x5C_1D_C2, 0x5C_1D_1D, 0x5C_5C_C2]);

    let (transport, mut controller) = setup_controller(mock);
    let random_address =
        read_random_address_stable(&mut controller, short).expect("random address majority");

    assert_eq!(random_address, 0x5C_1D_C2);
    assert_script_consumed(&transport);
}

#[test]
fn read_random_address_stable_returns_unstable_without_majority() {
    let mock = MockDaliTransport::new();
    let short = 17;
    script_random_address_reads(
        &mock,
        short,
        &[0x11_22_33, 0x44_55_66, 0x77_88_99, 0xAA_BB_CC, 0xDD_EE_FF],
    );

    let (transport, mut controller) = setup_controller(mock);
    let error = read_random_address_stable(&mut controller, short).expect_err("unstable");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("query_random_address_unstable")
    );
    assert_script_consumed(&transport);
}

#[test]
fn read_random_address_stable_names_the_byte_when_no_sample_ever_answers() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..RANDOM_ADDRESS_MAX_SAMPLES {
        script_random_address_unanswered_h(&mock, short);
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = read_random_address_stable(&mut controller, short).expect_err("no answer");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("query_random_address_h_no_answer")
    );
    assert_script_consumed(&transport);
}

#[test]
fn read_random_address_stable_survives_one_lost_byte_on_the_first_sample() {
    let mock = MockDaliTransport::new();
    let short = 17;
    script_random_address_unanswered_m(&mock, short, 0x5C);
    script_random_address_reads(&mock, short, &[0x5C_1D_C2, 0x5C_1D_C2]);

    let (transport, mut controller) = setup_controller(mock);
    let random_address =
        read_random_address_stable(&mut controller, short).expect("later samples answer");

    assert_eq!(random_address, 0x5C_1D_C2);
    assert_script_consumed(&transport);
}

#[test]
fn one_random_address_sample_goes_out_as_a_single_transaction() {
    let mock = MockDaliTransport::new();
    let short = 17;
    script_random_address_once(&mock, short, 0x5C_1D_C2);

    let (transport, mut controller) = setup_controller(mock);
    let counters = Arc::new(DaliWireCounters::default());
    controller.set_wire_counters(Arc::clone(&counters));

    let random_address = read_random_address_once(&mut controller, short).expect("sample");

    assert_eq!(random_address, 0x5C_1D_C2);
    assert_eq!(
        counters.transactions_started.load(Ordering::Relaxed),
        1,
        "three frames are one unit, not three"
    );
    assert_eq!(
        counters.frames_sent_by_priority[0].load(Ordering::Relaxed),
        2,
        "H opens the transaction at its own class; M and L follow at priority 1"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_contended_random_address_byte_is_terminal_and_not_silence() {
    let mock = MockDaliTransport::new();
    let short = 17;
    script_random_address_query(
        &mock,
        short,
        StandardCommand::QueryRandomAddressH,
        Some(0x5C),
    );
    mock.expect_forward_frame_corrupted_in_window(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryRandomAddressM,
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let error = read_random_address_stable(&mut controller, short).expect_err("contended");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed(READ_CONTENDED_MESSAGE)
    );
    assert_eq!(error.code(), ErrorCode::ReadContended);
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_uses_device_type_queries_for_presence() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        Some(6),
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt6Led,
            color_mode: ColorMode::Brightness,
            dt8_xy_capable: false,
            dt8_tc_capable: false,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[6]),
        })
    );
    assert_script_consumed(&transport);
}

fn script_colour_type_features(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), DT8_QUERY_COLOUR_TYPE_FEATURES),
        answer,
    );
}

#[test]
fn a_zero_colour_features_answer_the_re_read_corrects_is_counted() {
    const TC_ONLY: u8 = 0x02;
    let short = 17;
    let mock = MockDaliTransport::new();
    for answer in [Some(0), Some(TC_ONLY), Some(0), Some(0), None, Some(TC_ONLY), Some(TC_ONLY)] {
        script_colour_type_features(&mock, short, answer);
    }

    let (transport, mut controller) = setup_controller(mock);
    let counters = wire_counters(&mut controller);
    let mut probe = || {
        query_dt8_capabilities(&mut controller, short_address(short), DISCOVERY_CONTENT_CONFIRM)
            .expect("features")
            .map(|caps| caps.tc_capable)
    };
    assert_eq!(probe(), Some(true), "the second answer replaces the zero");
    assert_eq!(counters.readback_colour_features_corrected.load(Ordering::Relaxed), 1);
    assert_eq!(probe(), Some(false), "a confirmed zero is the gear's answer");
    assert_eq!(probe(), Some(true), "silence is re-asked too");
    assert_eq!(probe(), Some(true), "a non-zero first answer is taken as it is");
    assert_eq!(
        counters.readback_colour_features_corrected.load(Ordering::Relaxed),
        1,
        "a confirmed zero, silence and a non-zero answer corrected nothing"
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_recognizes_dt8_behind_concrete_first_via_fallback_probe() {
    let mock = MockDaliTransport::new();
    let short = 17;
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
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Cct,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[0, 8]),
        })
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_recognizes_masked_multi_dt_advertisement() {
    let mock = MockDaliTransport::new();
    let short = 17;
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        Some(255),
    );
    for response in [Some(6), Some(8), Some(254)] {
        mock.expect_forward_frame_with_backward(
            DaliCommand::Standard {
                address: short_address(short),
                command: StandardCommand::QueryNextDeviceType,
            }
            .to_forward_frame()
            .raw(),
            response,
        );
    }
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Cct,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[6, 8]),
        })
    );
    assert_script_consumed(&transport);
}

fn expect_device_type_query(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryDeviceType,
        }
        .to_forward_frame()
        .raw(),
        answer,
    );
}

fn expect_next_device_type(mock: &MockDaliTransport, short: u8, answer: Option<u8>) {
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryNextDeviceType,
        }
        .to_forward_frame()
        .raw(),
        answer,
    );
}

#[test]
fn a_broken_mask_walk_is_rerun_once_and_the_full_set_survives() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_device_type_query(&mock, short, Some(255));
    expect_next_device_type(&mock, short, Some(6));
    expect_next_device_type(&mock, short, None);
    expect_device_type_query(&mock, short, Some(255));
    for answer in [Some(6), Some(8), Some(254)] {
        expect_next_device_type(&mock, short, answer);
    }
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected.expect("device").device_type,
        DeviceType::Dt8Color,
        "the set from the SECOND, complete walk classifies the device"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_collision_mid_walk_restarts_the_sequence_instead_of_reasking() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_device_type_query(&mock, short, Some(255));
    expect_next_device_type(&mock, short, Some(6));
    mock.expect_forward_frame_collision(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryNextDeviceType,
        }
        .to_forward_frame()
        .raw(),
    );
    expect_device_type_query(&mock, short, Some(255));
    for answer in [Some(6), Some(8), Some(254)] {
        expect_next_device_type(&mock, short, answer);
    }
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let identity = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect("the restarted walk completes");

    assert_eq!(identity.device_type, DeviceType::Dt8Color);
    assert_script_consumed(&transport);
}

#[test]
fn a_twice_broken_walk_degrades_to_unknown_in_a_scan() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..2 {
        expect_device_type_query(&mock, short, Some(255));
        expect_next_device_type(&mock, short, None);
    }
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        None,
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("scan must not fail");

    let listed = detected.expect("listed");
    assert_eq!(
        listed.device_type,
        DeviceType::Unknown,
        "a gear whose enumeration cannot complete is listed, not lost"
    );
    assert!(
        listed.type_enum_degraded,
        "the degradation must be marked, or the counter stays silent"
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_twice_broken_walk_fails_an_explicit_read_with_the_named_error() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..2 {
        expect_device_type_query(&mock, short, Some(255));
        expect_next_device_type(&mock, short, None);
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect_err("strict path refuses the partial set");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed(DEVICE_TYPE_ENUM_INCOMPLETE)
    );
    assert_eq!(
        crate::runtime::executor::classify_read_abort(error),
        dali2rust_contracts::msg::AttributeGroupReadOutcome::SequenceIncomplete,
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_sixteen_type_gear_reaches_the_terminator() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_device_type_query(&mock, short, Some(255));
    for value in 0..16 {
        expect_next_device_type(&mock, short, Some(value));
    }
    expect_next_device_type(&mock, short, Some(254));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF9),
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let identity = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect("the full-ceiling walk completes");

    assert_eq!(identity.device_type, DeviceType::Dt8Color);
    assert_script_consumed(&transport);
}

#[test]
fn a_runaway_type_list_fails_honestly_after_one_rerun() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..2 {
        expect_device_type_query(&mock, short, Some(255));
        for value in 0..17 {
            expect_next_device_type(&mock, short, Some(value));
        }
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect_err("a runaway list must not commit");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed(DEVICE_TYPE_ENUM_INCOMPLETE)
    );
    assert_script_consumed(&transport);
}

#[test]
fn a_twice_rearmed_walk_fails_as_bus_contended() {
    let mock = MockDaliTransport::new();
    let short = 17;
    for _ in 0..2 {
        expect_device_type_query(&mock, short, Some(255));
        expect_next_device_type(&mock, short, Some(6));
        expect_next_device_type(&mock, short, Some(6));
    }

    let (transport, mut controller) = setup_controller(mock);
    let error = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect_err("a re-armed walk must not commit");

    assert_eq!(error, SemanticDaliError::OperationFailed("bus_contended"));
    assert_script_consumed(&transport);
}

#[test]
fn a_transport_error_mid_walk_propagates_instead_of_laundering() {
    let mock = MockDaliTransport::new();
    let short = 17;
    expect_device_type_query(&mock, short, Some(255));
    mock.expect_forward_frame_send_error(
        DaliCommand::Standard {
            address: short_address(short),
            command: StandardCommand::QueryNextDeviceType,
        }
        .to_forward_frame()
        .raw(),
    );

    let (transport, mut controller) = setup_controller(mock);
    let error = probe_device_identity(
        &mut controller,
        short_address(short),
        Dt8ProbePolicy::ExplicitRead,
    )
    .expect_err("a dead wire is not a walk break");

    assert_eq!(
        error,
        SemanticDaliError::OperationFailed("dali_transport_error")
    );
    assert_eq!(
        crate::runtime::executor::classify_read_abort(error),
        dali2rust_contracts::msg::AttributeGroupReadOutcome::TransportAbort,
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_falls_back_to_dt8_queries_when_advertisement_is_missing() {
    let mock = MockDaliTransport::new();
    let short = 17;
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
        Some(0x02),
    );
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(
        short_raw_query_frame(short_address(short), 0xF8),
        Some(0x20),
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Cct,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[0, 8]),
        })
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_keeps_dt8_probe_when_control_gear_present_would_timeout() {
    let mock = MockDaliTransport::new();
    let short = 17;
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

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Rgb,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: true,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[0, 8]),
        })
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_preserves_dt8_capabilities_when_color_status_times_out() {
    let mock = MockDaliTransport::new();
    let short = 17;
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
        None,
    );

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    assert_eq!(
        detected,
        Some(DiscoveredDevice {
            short_address: short,
            random_address: None,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Unknown,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: true,
            dt8_rgbwaf_capable: false,
            type_enum_degraded: false,
            supported_device_types: declared(&[0, 8]),
        })
    );
    assert_script_consumed(&transport);
}

#[test]
fn discover_known_control_gear_verifies_random_address_before_publish() {
    let mock = MockDaliTransport::new();
    script_present_short_addresses(&mock, &[0]);
    script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
    script_verify_session_begin(&mock);
    script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
    script_verify_session_end(&mock);
    mock.expect_forward_frame_with_backward(
        DaliCommand::Standard {
            address: short_address(0),
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
    mock.expect_forward_frame_with_backward(short_raw_query_frame(short_address(0), 0xF9), Some(0x02));
    mock.expect_forward_frame(
        DaliCommand::Special(SpecialCommand::EnableDeviceType(8))
            .to_forward_frame()
            .raw(),
    );
    mock.expect_forward_frame_with_backward(short_raw_query_frame(short_address(0), 0xF8), Some(0x20));

    let (transport, mut controller) = setup_controller(mock);
    let discovered = discover_known_control_gear(&mut controller).expect("discover");

    assert_eq!(
        discovered,
        DiscoveryScanReport {
            devices: vec![DiscoveredDevice {
                short_address: 0,
                random_address: Some(0x5C_1D_C2),
                device_type: DeviceType::Dt8Color,
                color_mode: ColorMode::Cct,
                dt8_xy_capable: false,
                dt8_tc_capable: true,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[0, 8]),
            }],
            error: None,
        }
    );
    assert_script_consumed(&transport);
}

#[test]
fn discover_known_control_gear_uses_third_random_read_to_resolve_mismatch() {
    let mock = MockDaliTransport::new();
    script_present_short_addresses(&mock, &[0]);
    script_random_address_reads(&mock, 0, &[0x5C_1D_C1, 0x5C_1D_C2, 0x5C_1D_C2]);
    script_verify_session_begin(&mock);
    script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
    script_verify_session_end(&mock);
    script_describe_dt6(&mock, 0);

    let (transport, mut controller) = setup_controller(mock);
    let discovered = discover_known_control_gear(&mut controller).expect("discover");

    assert_eq!(
        discovered,
        DiscoveryScanReport {
            devices: vec![DiscoveredDevice {
                short_address: 0,
                random_address: Some(0x5C_1D_C2),
                device_type: DeviceType::Dt6Led,
                color_mode: ColorMode::Brightness,
                dt8_xy_capable: false,
                dt8_tc_capable: false,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[6]),
            }],
            error: None,
        }
    );
    assert_script_consumed(&transport);
}

#[test]
fn discover_known_control_gear_retries_verify_after_short_address_timeout() {
    let mock = MockDaliTransport::new();
    script_present_short_addresses(&mock, &[0]);
    script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
    script_verify_session_begin(&mock);
    script_verify_attempt(&mock, 0x5C_1D_C2, None);
    script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
    script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
    script_verify_session_end(&mock);
    script_describe_dt6(&mock, 0);

    let (transport, mut controller) = setup_controller(mock);
    let discovered = discover_known_control_gear(&mut controller).expect("discover");

    assert_eq!(
        discovered,
        DiscoveryScanReport {
            devices: vec![DiscoveredDevice {
                short_address: 0,
                random_address: Some(0x5C_1D_C2),
                device_type: DeviceType::Dt6Led,
                color_mode: ColorMode::Brightness,
                dt8_xy_capable: false,
                dt8_tc_capable: false,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[6]),
            }],
            error: None,
        }
    );
    assert_script_consumed(&transport);
}

#[test]
fn discover_known_control_gear_keeps_verified_devices_when_later_verify_fails() {
    let mock = MockDaliTransport::new();
    script_present_short_addresses(&mock, &[0, 1]);
    script_random_address_reads(&mock, 0, &[0x5C_1D_C2, 0x5C_1D_C2]);
    script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
    script_verify_session_begin(&mock);
    script_verify_attempt(&mock, 0x5C_1D_C2, Some(0x01));
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
    script_verify_attempt(&mock, 0xC8_E7_31, None);
    script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
    script_verify_attempt(&mock, 0xC8_E7_31, None);
    script_random_address_reads(&mock, 1, &[0xC8_E7_31, 0xC8_E7_31]);
    script_verify_attempt(&mock, 0xC8_E7_31, None);
    script_verify_session_end(&mock);
    script_describe_dt6(&mock, 0);

    let (transport, mut controller) = setup_controller(mock);
    let discovered = discover_known_control_gear(&mut controller).expect("discover");

    assert_eq!(
        discovered,
        DiscoveryScanReport {
            devices: vec![DiscoveredDevice {
                short_address: 0,
                random_address: Some(0x5C_1D_C2),
                device_type: DeviceType::Dt6Led,
                color_mode: ColorMode::Brightness,
                dt8_xy_capable: false,
                dt8_tc_capable: false,
                dt8_rgb_capable: false,
                dt8_rgbwaf_capable: false,
                type_enum_degraded: false,
                supported_device_types: declared(&[6]),
            }],
            error: Some(SemanticDaliError::OperationFailed(
                "query_short_address_no_answer",
            )),
        }
    );
    assert_script_consumed(&transport);
}

#[test]
fn detect_device_rereads_dt8_features_when_first_read_is_empty() {
    let mock = MockDaliTransport::new();
    let short = 17;
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
        Some(0x00),
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

    let (transport, mut controller) = setup_controller(mock);
    let detected = detect_device(&mut controller, short).expect("detect");

    let d = detected.expect("device detected");
    assert!(d.dt8_tc_capable, "cct capability recovered by the re-read");
    assert!(d.dt8_rgb_capable);
    assert!(!d.dt8_xy_capable);
    assert_script_consumed(&transport);
}

#[test]
fn the_colour_mode_follows_the_one_active_type_bit_and_is_unknown_for_several() {
    for (status, mode) in [
        (0x10, ColorMode::Xy),
        (0x21, ColorMode::Cct),
        (0x80, ColorMode::Rgb),
        (0x40, ColorMode::Unknown),
        (0x00, ColorMode::Unknown),
        (0x30, ColorMode::Unknown),
        (0x90, ColorMode::Unknown),
        (0xFF, ColorMode::Unknown),
    ] {
        assert_eq!(color_mode_from_status(status), mode, "status {status:#04x}");
    }
}

struct HeldWhenScriptLeft {
    transport: Arc<std::sync::Mutex<MockDaliTransport>>,
    left: usize,
}

impl WireHold for HeldWhenScriptLeft {
    fn held(&self) -> Option<HoldCause> {
        let remaining = self.transport.lock().expect("mock lock").scripted_exchanges_remaining();
        (remaining <= self.left).then_some(HoldCause::Quiescent)
    }
}

const DESCRIBE_EXCHANGES: usize = 1;

#[test]
fn a_hold_while_describing_names_itself_over_an_earlier_skip_and_describes_nothing() {
    let mock = MockDaliTransport::new();
    let random_address = 0x5C_1D_C2;
    script_present_short_addresses(&mock, &[0, 1]);
    for _ in 0..RANDOM_ADDRESS_MAX_SAMPLES {
        script_random_address_unanswered_h(&mock, 0);
    }
    script_random_address_reads(&mock, 1, &[random_address, random_address]);
    script_verify_session_begin(&mock);
    script_verify_attempt(&mock, random_address, Some(0x03));
    mock.expect_forward_frame(DaliCommand::Special(SpecialCommand::Withdraw).to_forward_frame().raw());
    script_verify_session_end(&mock);
    script_describe_dt6(&mock, 1);

    let (transport, mut controller) = setup_controller(mock);
    let hold = Arc::new(HeldWhenScriptLeft { transport: Arc::clone(&transport), left: DESCRIBE_EXCHANGES });
    let discovered = controller.with_wire_hold(hold, discover_known_control_gear).expect("discover");

    assert_eq!(
        discovered,
        DiscoveryScanReport { devices: vec![], error: Some(SemanticDaliError::Conflict("controller_quiescent")) },
        "the skipped short 0 does not hide the hold, and short 1 is not reported as unknown"
    );
    let remaining = transport.lock().expect("mock lock").scripted_exchanges_remaining();
    assert_eq!(remaining, DESCRIBE_EXCHANGES, "nothing was sent to describe short 1");
}
