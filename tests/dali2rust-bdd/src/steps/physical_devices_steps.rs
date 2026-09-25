use std::time::Duration;
use std::time::Instant;

use cucumber::{given, then, when};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::dt8_color::Dt8Command;
use dali2rust_domain::dali::frame::ForwardFrame;
use dali2rust_domain::dali::net::address::DaliAddress;
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::{json, Value};

use crate::steps::wire::assert_frame_before;
use crate::steps::{last_json, response_json};
use crate::{DaliWorld};

const TEST_ADAPTER_ID: u8 = 0;
const TEST_SHORT_ADDRESS: u8 = 0;
const TEST_RANDOM_ADDRESS: u32 = 0x5C1D_C2;
const READ_MEMORY_LOCATION_OPCODE: u8 = 0xC5;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
const FULL_SEGMENT_OPERATION_TIMEOUT: Duration = Duration::from_secs(90);
const GROUP_APPLY_PACING_TIMEOUT: Duration = Duration::from_secs(14);
const READ_MODEL_TIMEOUT: Duration = Duration::from_secs(4);
const BANK0_BYTES: [u8; 29] = [
    0x1C, 0x00, 0x01, 0x00, 0x9D, 0xAD, 0xA2, 0x1B, 0x43, 0x01, 0x00, 0x26, 0x01, 0x06, 0xFA,
    0x9E, 0x78, 0x7D, 0x9B, 0x01, 0x00, 0x08, 0x08, 0xFF, 0x00, 0x01, 0x00,
    0x02,
    0b0000_0101,
];
const BANK0_BUS_UNIT_CONFIGURATION: u64 = 0x02;
const BANK0_IMPLEMENTED_PARTS_RAW: u64 = 0b0000_0101;
const BANK0_IMPLEMENTED_PARTS: [u16; 2] = [150, 152];
const SHORT_BANK0_BYTES: usize = 20;
const BANK1_BYTES: [u8; 17] = [
    0x10, 0x00, 0x06, 0x58, 0x23, 0x32, 0xA8, 0xFC, 0xFF, 0xF9, 0x00, 0x00, 0x00, 0xE6, 0xFF,
    0x9B, 0x01,
];
fn bank1_part251_bytes() -> Vec<u8> {
    use dali2rust_gear_model::luminaire::LuminaireBank;
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

const BANK0_GTIN: u64 = 0x009D_ADA2_1B43;
const BANK1_OEM_GTIN: u64 = 0x5823_32A8_FCFF;
const BANK1_OEM_ID: u64 = 0xF900_0000_E6FF_9B01;

fn short_address(short: u8) -> DaliAddress {
    DaliAddress::short(short).expect("valid short address")
}

pub(crate) fn standard_frame(short: u8, command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

fn extended_frame(short: u8, command: ExtendedCommand) -> u16 {
    DaliCommand::Extended {
        address: short_address(short),
        command,
    }
    .to_forward_frame()
    .raw()
}

pub(crate) fn special_frame(command: SpecialCommand) -> u16 {
    DaliCommand::Special(command).to_forward_frame().raw()
}

pub(crate) fn dt8_raw_query_frame(short: u8, opcode: u8) -> u16 {
    ForwardFrame::new(short_address(short).encode_address_byte() | 0x01, opcode).raw()
}

fn operation_path_from_last_response(world: &DaliWorld) -> String {
    let response = last_json(world);
    let op_id = response
        .get("operation_id")
        .and_then(Value::as_str)
        .expect("operation_id in last response");
    format!("/api/v1/operations/{op_id}")
}

fn physical_device_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}")
}

fn physical_devices_path(adapter_id: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices")
}

fn physical_device_attributes_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}/attributes")
}

fn physical_device_memory_banks_path(adapter_id: u8, short_address: u8) -> String {
    format!("/api/v1/adapters/{adapter_id}/physical-devices/{short_address}/memory-banks")
}

pub(crate) fn fetch_json(port: u16, path: &str) -> Option<Value> {
    let response = DaliWorld::send_http_request_raw(port, "GET", path, None, "");
    (response.status == 200).then(|| response_json(&response))
}

fn fetch_physical_device_full(port: u16, adapter_id: u8, short_address: u8) -> Option<Value> {
    let mut core = fetch_json(port, &physical_device_path(adapter_id, short_address))?;
    let attributes = fetch_json(port, &physical_device_attributes_path(adapter_id, short_address))
        .and_then(|v| v.get("attributes").cloned())
        .unwrap_or_else(|| Value::Object(Default::default()));
    let banks = fetch_json(port, &physical_device_memory_banks_path(adapter_id, short_address))
        .and_then(|v| v.get("memory_banks").cloned())
        .unwrap_or_else(|| Value::Array(Vec::new()));
    core["attributes"] = attributes;
    core["memory_banks"] = banks;
    Some(core)
}

fn refresh_get(world: &mut DaliWorld, path: &str) -> Value {
    world.send_http_request("GET", path, None, "");
    last_json(world)
}

pub(crate) fn wait_for_operation_status(world: &mut DaliWorld, expected: &str) {
    wait_for_operation_status_within(world, expected, OPERATION_TIMEOUT);
}

pub(crate) fn wait_for_operation_status_within(
    world: &mut DaliWorld,
    expected: &str,
    budget: Duration,
) {
    use std::cell::RefCell;

    let port = world.server_port();
    let path = operation_path_from_last_response(world);
    let start = Instant::now();
    let last_json = RefCell::new(None);
    wait_until(
        || {
            let fetched = fetch_json(port, &path);
            *last_json.borrow_mut() = fetched.clone();
            let status = fetched
                .as_ref()
                .and_then(|json| json.get("status"))
                .and_then(Value::as_str);
            matches!(status, Some("failed" | "timed_out" | "superseded"))
                || status == Some(expected)
                || start.elapsed() >= budget
        },
        budget + Duration::from_millis(50),
    );
    let last_json = last_json.into_inner();
    let status = last_json
        .as_ref()
        .and_then(|json| json.get("status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != expected {
        let mock = world.dali_mock().lock().expect("mock lock");
        panic!(
            "operation did not reach expected status {expected}: last_status={status}; last_json={last_json:?}; script_error={:?}; frames={:?}",
            mock.script_error(),
            mock.sent_frames()
        );
    }
    world.send_http_request("GET", &path, None, "");
}

fn wait_for_physical_device(world: &mut DaliWorld, predicate: impl Fn(&Value) -> bool) -> Value {
    let path = physical_device_path(TEST_ADAPTER_ID, TEST_SHORT_ADDRESS);
    let port = world.server_port();
    wait_until(
        || {
            fetch_physical_device_full(port, TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
                .as_ref()
                .is_some_and(&predicate)
        },
        READ_MODEL_TIMEOUT,
    );
    let core = refresh_get(world, &path);
    fetch_physical_device_full(world.server_port(), TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
        .unwrap_or(core)
}

fn fetch_physical_device(world: &mut DaliWorld) -> Value {
    let path = physical_device_path(TEST_ADAPTER_ID, TEST_SHORT_ADDRESS);
    let core = refresh_get(world, &path);
    fetch_physical_device_full(world.server_port(), TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
        .unwrap_or(core)
}

fn wait_for_physical_devices(world: &mut DaliWorld, predicate: impl Fn(&Value) -> bool) -> Value {
    let path = physical_devices_path(TEST_ADAPTER_ID);
    let port = world.server_port();
    wait_until(
        || fetch_json(port, &path).as_ref().is_some_and(&predicate),
        READ_MODEL_TIMEOUT,
    );
    refresh_get(world, &path)
}

#[derive(Clone, Copy)]
struct ScriptedReply {
    value: Option<u8>,
    contended: bool,
}

fn expect_scripted_reply(mock: &MockDaliTransport, frame: u16, reply: ScriptedReply) {
    if reply.contended {
        mock.expect_forward_frame_with_backward_contended(frame, reply.value);
    } else {
        mock.expect_forward_frame_with_backward(frame, reply.value);
    }
}

fn script_detect_dt8_cct(mock: &MockDaliTransport, short: u8) {
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

fn script_memory_bank_read(mock: &MockDaliTransport, bank: u8, bytes: &[u8]) {
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

fn script_memory_bank_short_read(mock: &MockDaliTransport, bank: u8, bytes: &[u8]) {
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
    mock.clear();
    script_presence_sweep(mock, present);
    for &(short, random_address) in present {
        script_discovery_random_address_reads(mock, short, &[random_address, random_address]);
    }
    script_verify_session(mock, present);
    for &(short, _) in present {
        script_detect_dt8_with_features(mock, short, features);
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

pub(crate) fn read_groups_membership_from_gear(world: &mut DaliWorld, short: u8, mask: u16) {
    {
        let mock = world.dali_mock().lock().expect("mock lock");
        script_groups_attribute_read(&mock, short, mask);
    }
    let path = format!("{}/attribute-reads", physical_device_path(TEST_ADAPTER_ID, short));
    world.send_http_request(
        "POST",
        &path,
        Some(br#"{"attribute_groups":["groups"],"memory_banks":"none"}"#),
        "application/json",
    );
    wait_for_operation_status(world, "succeeded");
    let port = world.server_port();
    wait_until(
        || {
            fetch_physical_device_full(port, TEST_ADAPTER_ID, short)
                .as_ref()
                .and_then(|json| json.pointer("/attributes/groups/membership/value"))
                .and_then(Value::as_u64)
                == Some(u64::from(mask))
        },
        READ_MODEL_TIMEOUT,
    );
}

fn script_discovery_multi_dt_mask(mock: &MockDaliTransport) {
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

fn script_discovery_declares_only_dt6(mock: &MockDaliTransport) {
    script_discovery_one_verified_device(mock);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
        Some(6),
    );
}

fn script_discovery_unterminated_type_walk(mock: &MockDaliTransport) {
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

fn script_discovery_no_answers(mock: &MockDaliTransport) {
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

fn script_discovery_partial_failure(mock: &MockDaliTransport) {
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

fn script_discovery_permanent_multiple_responders(mock: &MockDaliTransport) {
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

fn script_discovery_corrupted_window_retry(mock: &MockDaliTransport) {
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
const DT8_FEATURES_RGB_CAPABLE: u8 = 0xC2;
const SCRIPTED_RUNTIME_STATUS: u8 = 0x84;
const SCRIPTED_RUNTIME_LEVEL: u8 = 0x7F;
const GOLDEN_TC_MIREK: u16 = 0x5678;
const PD165_TC_MIREK: u16 = 250;
const GOLDEN_GEAR_FEATURES: u8 = 0x41;
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

fn script_dt8_colour_value(mock: &MockDaliTransport, value_id: u8, msb: u8, lsb: u8) {
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

fn script_attribute_read_runtime_and_colour(
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

fn script_attribute_read_no_probe(mock: &MockDaliTransport) {
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

fn script_attribute_read_faulted_gear(
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

fn script_attribute_read_rgb_active(mock: &MockDaliTransport, rgb: (u8, u8, u8)) {
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
        Some(SCRIPTED_RUNTIME_STATUS),
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

fn script_attribute_read_colour_unanswered(mock: &MockDaliTransport) {
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

fn script_attribute_read_identity_queries(
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

fn script_attribute_read_identity(mock: &MockDaliTransport) {
    script_attribute_read_identity_common(
        mock,
        &[ScriptedReply {
            value: Some(0x01),
            contended: false,
        }],
    );
}

fn script_attribute_read_identity_short_bank0(mock: &MockDaliTransport) {
    script_attribute_read_identity_prelude(mock);
    script_memory_bank_read(mock, 0, &BANK0_BYTES[..3]);
    script_memory_bank_short_read(mock, 0, &BANK0_BYTES[..SHORT_BANK0_BYTES]);
    script_memory_bank_read(mock, 1, &BANK1_BYTES[..1]);
    script_memory_bank_read(mock, 1, &BANK1_BYTES);
}

fn script_attribute_read_identity_with_content_confirm(mock: &MockDaliTransport) {
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

fn assert_no_script_errors(world: &DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    assert_eq!(
        mock.scripted_exchanges_remaining(),
        0,
        "scripted exchanges remain: {:?}",
        mock.sent_frames()
    );
    assert_eq!(mock.script_error(), None, "unexpected mock script error");
}

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-012 MQTT-013 MQTT-015 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-190 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-241 PD-242 PD-243 PD-250 PD-252 PD-253 PD-254 PD-255 PERS-005 STATS-005 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 WS-003 WS-004 WS-010 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024
#[given("a golden control-gear discovery script for short address 0")]
async fn given_golden_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery(&mock);
}

fn full_segment_random_addresses() -> Vec<(u8, u32)> {
    (0..=63u8)
        .map(|short| (short, 0x5C_0000 | (u32::from(short) << 8) | u32::from(short)))
        .collect()
}

// PD-240
#[given("a discovery script for all 64 short addresses")]
async fn given_full_segment_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_scan_discovery(&mock, &full_segment_random_addresses(), 0x02);
}

// PD-167 PD-178 PD-251
#[given("a six-channel DT8 discovery script for short address 0")]
async fn given_six_channel_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_six_channel_discovery(&mock);
}

// PD-106
#[given("a refresh detect-only script for short address 0")]
async fn given_refresh_detect_only_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_detect_dt8_cct(&mock, TEST_SHORT_ADDRESS);
}

// PD-107
#[given("a discovery script where no control gear answers the presence sweep")]
async fn given_empty_presence_sweep_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_no_answers(&mock);
}

// PD-103
#[given("a discovery script where short address 0 verifies before short address 1 times out")]
async fn given_partial_failure_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_partial_failure(&mock);
}

// PD-104
#[given("a discovery script where foreign-master activity corrupts the QueryShortAddress backward window")]
async fn given_corrupted_window_retry_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_corrupted_window_retry(&mock);
}

// PD-105 PD-189 PD-192
#[given("a discovery script where QueryDeviceType returns MASK before QueryNextDeviceType enumerates DT6 and DT8")]
async fn given_multi_dt_mask_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_multi_dt_mask(&mock);
}

// PD-193
#[given("a discovery script where short address 0 declares only DT6")]
async fn given_dt6_only_discovery_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_declares_only_dt6(&mock);
}

// PD-194
#[given("a discovery script where two gear answer one search address for the whole verify")]
async fn given_permanent_multiple_responders_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_permanent_multiple_responders(&mock);
}

// PD-191
#[given("a discovery script where the QueryNextDeviceType walk never reaches the 254 terminator")]
async fn given_unterminated_type_walk_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_discovery_unterminated_type_walk(&mock);
}

// PD-198
#[given("a golden attribute-read script with DiiA Part 251 luminaire data for short address 0")]
async fn given_attribute_read_part251_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity_queries(
        &mock,
        &[ScriptedReply { value: Some(0x01), contended: false }],
    );
    let bank1 = bank1_part251_bytes();
    script_memory_bank_read(&mock, 0, &BANK0_BYTES[..3]);
    script_memory_bank_read(&mock, 0, &BANK0_BYTES);
    script_memory_bank_read(&mock, 1, &bank1[..1]);
    script_memory_bank_read(&mock, 1, &bank1);
}

// PD-199
#[given("a golden attribute-read script with an unrecognised bank 1 content format for short address 0")]
async fn given_attribute_read_vendor_bank1_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity_queries(
        &mock,
        &[ScriptedReply { value: Some(0x01), contended: false }],
    );
    let mut bank1 = bank1_part251_bytes();
    bank1[usize::from(VENDOR_FORMAT_ID_OFFSET)] = 0x00;
    bank1[usize::from(VENDOR_FORMAT_ID_OFFSET) + 1] = 0x42;
    script_memory_bank_read(&mock, 0, &BANK0_BYTES[..3]);
    script_memory_bank_read(&mock, 0, &BANK0_BYTES);
    script_memory_bank_read(&mock, 1, &bank1[..1]);
    script_memory_bank_read(&mock, 1, &bank1);
}

const VENDOR_FORMAT_ID_OFFSET: u8 = 0x11;

// OP-100 PD-150 PD-197 SYS-217 PD-195 PD-196 PD-220 PD-221 PD-230 PERS-005
#[given("a golden attribute-read identity script for short address 0")]
async fn given_golden_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity(&mock);
}

// PD-165
#[given("an attribute-read script where short address 0 answers 4000K as its active colour")]
async fn given_attribute_read_colour_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_runtime_and_colour(&mock, PD165_TC_MIREK, Some(GOLDEN_GEAR_FEATURES));
}

// PD-170
#[given(
    regex = r"^an attribute-read script where short address 0 answers gear features (0x[0-9A-Fa-f]+)$"
)]
async fn given_attribute_read_gear_features(world: &mut DaliWorld, byte: String) {
    let features = u8::from_str_radix(byte.trim_start_matches("0x"), 16).expect("hex byte");
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_runtime_and_colour(&mock, PD165_TC_MIREK, Some(features));
}

// PD-171
#[given("an attribute-read script where short address 0 leaves the gear features query unanswered")]
async fn given_attribute_read_gear_features_silent(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_runtime_and_colour(&mock, PD165_TC_MIREK, None);
}

// PD-170 PD-171
#[then(regex = r"^adapter 0 physical device 0 eventually exposes gear features (\d+)$")]
async fn then_pd_gear_features(world: &mut DaliWorld, expected: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/dt8_color/gear_features/value")
            .and_then(Value::as_u64)
            == Some(expected)
    });
    assert_eq!(
        json.pointer("/attributes/dt8_color/gear_features/source")
            .and_then(Value::as_str),
        Some("readback")
    );
}

// PD-178
#[then(regex = r#"^adapter 0 physical device 0 exposes dt8 attribute "([a-z_]+)" (\d+)$"#)]
async fn then_pd_dt8_attribute(world: &mut DaliWorld, name: String, expected: u64) {
    let pointer = format!("/attributes/dt8_color/{name}/value");
    let json = wait_for_physical_device(world, |body| {
        body.pointer(&pointer).and_then(Value::as_u64) == Some(expected)
    });
    assert_eq!(
        json.pointer(&pointer).and_then(Value::as_u64),
        Some(expected),
        "{json}"
    );
}

// PD-178
#[then(regex = r#"^adapter 0 physical device 0 reports capability "([a-z]+)" (true|false)$"#)]
async fn then_pd_capability(world: &mut DaliWorld, name: String, expected: String) {
    let want = expected == "true";
    let pointer = format!("/capabilities/{name}");
    let json = wait_for_physical_device(world, |body| {
        body.pointer(&pointer).and_then(Value::as_bool) == Some(want)
    });
    assert_eq!(
        json.pointer(&pointer).and_then(Value::as_bool),
        Some(want),
        "{json}"
    );
}

// PD-171
#[then("adapter 0 physical device 0 exposes no gear features")]
async fn then_pd_no_gear_features(world: &mut DaliWorld) {
    let json = fetch_physical_device(world);
    assert!(
        json.pointer("/attributes/dt8_color/gear_features").is_none(),
        "an unanswered 247 must leave the attribute absent, not invent a byte: {json}"
    );
}

// PD-168 PD-256
#[given("an attribute-read script for a gear reporting lamp failure for short address 0")]
async fn given_attribute_read_lamp_failure_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_faulted_gear(&mock, 0x02, 100, 0x21);
}

// PD-258
#[given("an attribute-read script where the gear answers MASK for its actual level")]
async fn given_attribute_read_masked_level_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_faulted_gear(&mock, 0x00, 0xFF, 0x20);
}

// PD-256 PD-258
#[then(regex = r"^adapter 0 physical device 0 eventually reports Part 207 failure byte (\d+)$")]
async fn then_pd_failure_byte(world: &mut DaliWorld, expected: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/dt6_led/failure_status/value").and_then(Value::as_u64)
            == Some(expected)
    });
    for (bit, field) in [
        "short_circuit",
        "open_circuit",
        "load_decrease",
        "load_increase",
        "current_protector_active",
        "thermal_shutdown",
        "thermal_overload",
        "reference_measurement_failed",
    ]
    .into_iter()
    .enumerate()
    {
        let expected_value = if expected & (1 << bit) != 0 { 255 } else { 0 };
        assert_eq!(
            json.pointer(&format!("/attributes/dt6_led/{field}/value"))
                .and_then(Value::as_u64),
            Some(expected_value),
            "bit {bit} of 0x{expected:02X} is {field}: {json:?}"
        );
    }
    assert!(
        json.pointer("/attributes/dt6_led/reference_running").is_none(),
        "249 is a state the byte does not carry — the escalation must leave it \
         unread rather than invent a no: {json:?}"
    );
}

// PD-256 PD-257
#[then(regex = r"^the attribute-read transport trace should (include|exclude) the Part 207 failure query$")]
async fn then_trace_failure_query(world: &mut DaliWorld, expectation: String) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let query = extended_frame(
        TEST_SHORT_ADDRESS,
        ExtendedCommand::Dt6(Dt6Command::QueryFailureStatus),
    );
    let prelude = special_frame(SpecialCommand::EnableDeviceType(6));
    let present = frames.contains(&query) && frames.contains(&prelude);
    match expectation.as_str() {
        "include" => assert!(present, "expected the DT6 prelude and command 241: {frames:04X?}"),
        _ => assert!(!present, "a healthy read must spend no Part 207 frames: {frames:04X?}"),
    }
}

// PD-168 PD-257 PD-258
#[given("an attribute-read script with no device-type probe for short address 0")]
async fn given_attribute_read_no_probe_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_no_probe(&mock);
}

// PD-167 PD-251 PD-178
#[given(regex = r"^an attribute-read script where short address 0 is RGB-active at (\d+) (\d+) (\d+)$")]
async fn given_attribute_read_rgb_script(world: &mut DaliWorld, r: u8, g: u8, b: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_rgb_active(&mock, (r, g, b));
}

// PD-167 PD-251
#[then(regex = r"^adapter 0 physical device 0 eventually exposes runtime rgb (\d+) (\d+) (\d+)$")]
async fn then_pd_runtime_rgb(world: &mut DaliWorld, r: u64, g: u64, b: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/state/rgb/r").and_then(Value::as_u64) == Some(r)
    });
    assert_eq!(json.pointer("/state/rgb/g").and_then(Value::as_u64), Some(g));
    assert_eq!(json.pointer("/state/rgb/b").and_then(Value::as_u64), Some(b));
    assert_eq!(
        json.pointer("/state/color_mode").and_then(Value::as_str),
        Some("rgbwaf")
    );
}

// PD-166
#[given("an attribute-read script where short address 0 leaves the colour temperature query unanswered")]
async fn given_attribute_read_colour_unanswered_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_colour_unanswered(&mock);
}

// PD-163
#[given("an attribute-read script where bank 0 ends before its header says")]
async fn given_attribute_read_short_bank0_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity_short_bank0(&mock);
}

// PD-169
#[given("an attribute-read script where short address 0 answers presence and then falls silent")]
async fn given_attribute_read_vanishing_device_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for command in [
        StandardCommand::QueryRandomAddressH,
        StandardCommand::QueryRandomAddressM,
        StandardCommand::QueryRandomAddressL,
    ] {
        mock.expect_forward_frame_with_backward(standard_frame(TEST_SHORT_ADDRESS, command), None);
    }
}

// PD-164
#[given("an attribute-read script where short address 0 never answers the presence probe")]
async fn given_attribute_read_absent_device_script(world: &mut DaliWorld) {
    const PRESENCE_PROBE_ATTEMPTS: usize = 3;
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for _ in 0..PRESENCE_PROBE_ATTEMPTS {
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
            None,
        );
    }
}

// OP-100 OP-130 OP-132 PD-040 PD-041 PD-157 VL-020 WS-046 MQTT-002 MQTT-003 MQTT-005 MQTT-007 MQTT-008 MQTT-013 MQTT-015 MQTT-018 MQTT-019 COMM-092 MQTT-001 WS-003 WS-004 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 MQTT-024
#[given(regex = r"^a successful target-state script for level (\d+) on short address 0$")]
async fn given_successful_target_state_script(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(standard_frame(
        TEST_SHORT_ADDRESS,
        StandardCommand::DirectArcPower { level },
    ));
}

// COMM-004 COMM-008 COMM-052 COMM-056 COMM-057 INP-001 INP-002 INP-003 INP-004 INP-005 OP-100 PD-042 PD-043 PD-255 RULE-020 RULE-021 RULE-023 SYS-230 SYS-231 SYS-232 SYS-233 SYS-235 SYS-236 SYS-238 SYS-239 SYS-240
#[given("the DALI mock transport trace is cleared")]
async fn given_mock_trace_cleared(world: &mut DaliWorld) {
    world.dali_mock().lock().expect("mock lock").clear();
}

// PD-040
#[given("a power-off target-state script for short address 0")]
async fn given_power_off_target_state_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(standard_frame(
        TEST_SHORT_ADDRESS,
        StandardCommand::DirectArcPower { level: 0 },
    ));
}

// PD-040 PD-157 MQTT-012 MQTT-013 MQTT-019 PD-166 PD-168
#[given("a cct 3000K target-state script for short address 0")]
async fn given_cct_target_state_script(world: &mut DaliWorld) {
    const CCT_3000K_MIREK: u16 = 333;
    const DT8_SET_TEMPERATURE_TC_OPCODE: u8 = 231;
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_staged_dtrs(
        &mock,
        TEST_SHORT_ADDRESS,
        &[
            (CCT_3000K_MIREK & 0x00FF) as u8,
            (CCT_3000K_MIREK >> 8) as u8,
        ],
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_raw_query_frame(
        TEST_SHORT_ADDRESS,
        DT8_SET_TEMPERATURE_TC_OPCODE,
    ));
    const DT8_QUERY_COLOUR_STATUS_OPCODE: u8 = 248;
    const DT8_STATUS_TC_ACTIVE: u8 = 0x20;
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        dt8_raw_query_frame(TEST_SHORT_ADDRESS, DT8_QUERY_COLOUR_STATUS_OPCODE),
        Some(DT8_STATUS_TC_ACTIVE),
    );
}

// PD-037
#[given("an rgb target-state script for short address 0")]
async fn given_rgb_target_state_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_rgb_target_state(&mock, [254, 0, 0]);
}

// PD-250
#[given("a mid-tone rgb target-state script for short address 0")]
async fn given_mid_tone_rgb_target_state_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_rgb_target_state(&mock, [254, 116, 26]);
}

fn script_rgb_target_state(mock: &MockDaliTransport, rgb: [u8; 3]) {
    const DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE: u8 = 235;
    const DT8_SET_TEMPORARY_WAF_DIMLEVEL_OPCODE: u8 = 236;
    const DT8_SET_TEMPORARY_RGBWAF_CONTROL_OPCODE: u8 = 237;
    const DT8_QUERY_COLOUR_STATUS_OPCODE: u8 = 248;
    const DT8_QUERY_RGBWAF_CONTROL_OPCODE: u8 = 251;
    const DT8_STATUS_RGB_ACTIVE: u8 = 0x80;
    const RGBWAF_CONTROL_POWER_UP: u8 = 0x3F;
    const RGBWAF_CONTROL_NORMALISED: u8 = 0x80;
    mock.clear();

    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        dt8_raw_query_frame(TEST_SHORT_ADDRESS, DT8_QUERY_RGBWAF_CONTROL_OPCODE),
        Some(RGBWAF_CONTROL_POWER_UP),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(
        RGBWAF_CONTROL_NORMALISED,
    )));
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        Some(RGBWAF_CONTROL_NORMALISED),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_raw_query_frame(
        TEST_SHORT_ADDRESS,
        DT8_SET_TEMPORARY_RGBWAF_CONTROL_OPCODE,
    ));

    script_staged_dtrs(mock, TEST_SHORT_ADDRESS, &rgb);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_raw_query_frame(
        TEST_SHORT_ADDRESS,
        DT8_SET_TEMPORARY_RGB_DIMLEVEL_OPCODE,
    ));
    script_staged_dtrs(mock, TEST_SHORT_ADDRESS, &[0, 0, 0]);
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame(dt8_raw_query_frame(
        TEST_SHORT_ADDRESS,
        DT8_SET_TEMPORARY_WAF_DIMLEVEL_OPCODE,
    ));

    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(8)));
    mock.expect_forward_frame_with_backward(
        dt8_raw_query_frame(TEST_SHORT_ADDRESS, DT8_QUERY_COLOUR_STATUS_OPCODE),
        Some(DT8_STATUS_RGB_ACTIVE),
    );
}

fn script_fade_time_write(mock: &MockDaliTransport, dtr0: u8) {
    mock.clear();
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(dtr0)));
    let set_fade = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::SetFadeTime);
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame(set_fade);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryFadeTimeFadeRate),
        Some(dtr0 << 4),
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
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(dtr0)));
    let set_frame = standard_frame(short, set);
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame(set_frame);
    mock.expect_forward_frame_with_backward(standard_frame(short, query), Some(answer));
}

fn script_min_triple(mock: &MockDaliTransport, dtr0: u8, answer: u8) {
    script_level_bound_triple(
        mock,
        StandardCommand::SetMinLevel,
        StandardCommand::QueryMinLevel,
        dtr0,
        answer,
    );
}

fn script_max_triple(mock: &MockDaliTransport, dtr0: u8, answer: u8) {
    script_level_bound_triple(
        mock,
        StandardCommand::SetMaxLevel,
        StandardCommand::QueryMaxLevel,
        dtr0,
        answer,
    );
}

// PD-252 PD-253
#[given(regex = r"^a dimming-curve (\d+) write-attributes script for short address 0$")]
async fn given_dimming_curve_write_script(world: &mut DaliWorld, curve: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_dimming_curve_write(&mock, curve, curve);
}

// PD-254
#[given(
    regex = r"^a dimming-curve (\d+) write script for short address 0 that the gear ignores$"
)]
async fn given_dimming_curve_write_ignored_script(world: &mut DaliWorld, curve: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_dimming_curve_write(&mock, curve, 0);
    script_dimming_curve_write_no_clear(&mock, curve, 0);
    script_dimming_curve_write_no_clear(&mock, curve, 0);
}

fn script_dimming_curve_write(mock: &MockDaliTransport, curve: u8, answer: u8) {
    mock.clear();
    script_dimming_curve_write_no_clear(mock, curve, answer);
}

fn script_dimming_curve_write_no_clear(mock: &MockDaliTransport, curve: u8, answer: u8) {
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
        Some(answer),
    );
}

// PD-184
#[given(regex = r"^a min-level (\d+) write-attributes script for short address 0$")]
async fn given_min_level_write_script(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_min_triple(&mock, level, level);
}

// PD-185
#[given(regex = r"^a min-level write script for short address 0 where (\d+) is clamped to (\d+)$")]
async fn given_min_level_clamped_script(world: &mut DaliWorld, requested: u8, accepted: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_min_triple(&mock, requested, accepted);
    script_min_triple(&mock, requested, accepted);
}

// PD-186
#[given(regex = r"^a min-max write script for short address 0 raising (\d+) and (\d+) over old max (\d+)$")]
async fn given_min_max_raising_script(
    world: &mut DaliWorld,
    min: u8,
    max: u8,
    old_max: u8,
) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_min_triple(&mock, min, old_max);
    script_min_triple(&mock, min, old_max);
    script_max_triple(&mock, max, max);
    script_min_triple(&mock, min, min);
}

// PD-188
#[given(regex = r"^a min-max write script for short address 0 lowering to (\d+) and (\d+)$")]
async fn given_min_max_lowering_script(world: &mut DaliWorld, min: u8, max: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_min_triple(&mock, min, min);
    script_max_triple(&mock, max, max);
}

// PD-184 PD-185 PD-186 PD-188 PD-252 POLICY-007 PD-253
#[then(
    regex = r"^physical device (\d+) eventually exposes write-confirmed (common_102|dt6_led) (min_level|max_level|dimming_curve|power_on_level|system_failure_level) (\d+)$"
)]
async fn then_pd_write_confirmed_bound(
    world: &mut DaliWorld,
    short: u8,
    section: String,
    field: String,
    expected: u64,
) {
    let port = world.server_port();
    let value_ptr = format!("/attributes/{section}/{field}/value");
    let source_ptr = format!("/attributes/{section}/{field}/source");
    let read = |json: &Value| {
        let value = json.pointer(&value_ptr).and_then(Value::as_u64)?;
        let source = json.pointer(&source_ptr).and_then(Value::as_str)?;
        Some((value, source.to_owned()))
    };
    wait_until(
        || {
            fetch_physical_device_full(port, TEST_ADAPTER_ID, short).as_ref().and_then(read)
                == Some((expected, "write_confirmed".to_owned()))
        },
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_physical_device_full(port, TEST_ADAPTER_ID, short);
    assert_eq!(
        json.as_ref().and_then(read),
        Some((expected, "write_confirmed".to_owned())),
        "{field} write-confirmed readback: {json:?}"
    );
}

// PD-030 PD-241
#[given("a fade-time 500ms write-attributes script for short address 0")]
async fn given_fade_time_write_script(world: &mut DaliWorld) {
    script_fade_time_write(&world.dali_mock().lock().expect("mock lock"), 1);
}

// PD-242
#[given("a fade-time 100ms write-attributes script for short address 0")]
async fn given_fade_time_100_write_script(world: &mut DaliWorld) {
    script_fade_time_write(&world.dali_mock().lock().expect("mock lock"), 1);
}

// PD-243
#[given("a fade-time 90500ms write-attributes script for short address 0")]
async fn given_fade_time_90500_write_script(world: &mut DaliWorld) {
    script_fade_time_write(&world.dali_mock().lock().expect("mock lock"), 15);
}

// PD-034
#[given("a fade-time 2000ms write-attributes script for short address 0")]
async fn given_fade_time_2000_write_script(world: &mut DaliWorld) {
    script_fade_time_write(&world.dali_mock().lock().expect("mock lock"), 4);
}

// PD-034
#[given("a common-102 attribute-read script with fade byte 0x47 for short address 0")]
async fn given_common102_fade_readback_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    for (command, reply) in [
        (StandardCommand::QueryVersionNumber, 0x08),
        (StandardCommand::QueryDeviceType, 0x08),
        (StandardCommand::QueryPhysicalMinimum, 0x01),
        (StandardCommand::QueryMinLevel, 0x01),
        (StandardCommand::QueryMaxLevel, 0xFE),
        (StandardCommand::QueryPowerOnLevel, 0xFE),
        (StandardCommand::QuerySystemFailureLevel, 0xFE),
        (StandardCommand::QueryFadeTimeFadeRate, 0x47),
        (StandardCommand::QueryLightSourceType, 0x06),
        (StandardCommand::QueryRandomAddressH, 0x12),
        (StandardCommand::QueryRandomAddressM, 0x34),
        (StandardCommand::QueryRandomAddressL, 0x56),
    ] {
        mock.expect_forward_frame_with_backward(
            standard_frame(TEST_SHORT_ADDRESS, command),
            Some(reply),
        );
    }
}

// PD-034 PD-241 PD-242 PD-243 SYS-233
#[then(regex = r"^physical device 0 eventually exposes fade_time_ms (\d+)$")]
async fn then_pd_fade_time_ms(world: &mut DaliWorld, expected: u64) {
    let port = world.server_port();
    let read = |json: &Value| {
        json.pointer("/attributes/common_102/fade_time_ms/value")
            .and_then(Value::as_u64)
    };
    wait_until(
        || fetch_physical_device_full(port, TEST_ADAPTER_ID, 0).as_ref().and_then(read) == Some(expected),
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_physical_device_full(port, TEST_ADAPTER_ID, 0);
    assert_eq!(
        json.as_ref().and_then(read),
        Some(expected),
        "fade_time_ms readback: {json:?}"
    );
}

// PD-040 PD-157 SYS-217
#[then(regex = r"^the physical device (\d+) state level should eventually be (\d+)$")]
async fn then_pd_state_level_eventually(world: &mut DaliWorld, short: u8, level: u8) {
    let port = world.server_port();
    let path = physical_device_path(TEST_ADAPTER_ID, short);
    wait_until(
        || {
            fetch_json(port, &path)
                .and_then(|json| json.pointer("/state/level").and_then(Value::as_u64))
                == Some(u64::from(level))
        },
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_json(port, &path);
    assert_eq!(
        json.as_ref()
            .and_then(|j| j.pointer("/state/level"))
            .and_then(Value::as_u64),
        Some(u64::from(level)),
        "physical device {short} runtime level: {json:?}"
    );
}

// OP-100 OP-130 OP-131 OP-132 PD-041 COMM-080
#[then(regex = r"^the operations list should contain exactly (\d+) operations?$")]
async fn then_operations_list_count(world: &mut DaliWorld, expected: usize) {
    world.send_http_request("GET", "/api/v1/operations", None, "");
    let json = last_json(world);
    let operations = json
        .get("operations")
        .and_then(Value::as_array)
        .expect("operations array");
    assert_eq!(
        operations.len(),
        expected,
        "unexpected operations list: {json:?}"
    );
}

// VL-054
#[given("a target-state script where the level command collides until a sequence retry succeeds")]
async fn given_contended_target_state_retry_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    let dapc = standard_frame(
        TEST_SHORT_ADDRESS,
        StandardCommand::DirectArcPower { level: 220 },
    );
    mock.expect_forward_frame_collision(dapc);
    mock.expect_forward_frame_collision(dapc);
    mock.expect_forward_frame_collision(dapc);
    mock.expect_forward_frame(dapc);
}

// PD-156
#[given("an attribute-read script where the common_102 group aborts after contention retries")]
async fn given_contended_abort_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        Some(0xFF),
    );
    let version = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryVersionNumber);
    mock.expect_forward_frame_collision(version);
    mock.expect_forward_frame_collision(version);
    mock.expect_forward_frame_collision(version);
}

// PD-159
#[given("a groups attribute-read script with a doubled-byte first pair for short address 0")]
async fn given_doubled_groups_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_attribute_read_prelude(&mock, TEST_SHORT_ADDRESS);
    let lo = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryGroups0To7);
    let hi = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryGroups8To15);
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x02));
    mock.expect_forward_frame_with_backward(lo, Some(0x02));
    mock.expect_forward_frame_with_backward(hi, Some(0x00));
}

// PD-159 PD-169 PD-164
#[when(r#"I start an attribute read for adapter 0 physical device 0 with attribute group "groups" only"#)]
async fn when_start_groups_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["groups"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// SCN-085
#[when(r#"I start an attribute read for adapter 0 physical device 0 with attribute group "scene_colours" only"#)]
async fn when_start_scene_colours_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["scene_colours"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// PD-159
#[then("physical device 0 eventually exposes groups membership 2")]
async fn then_pd_groups_membership_two(world: &mut DaliWorld) {
    let port = world.server_port();
    let read = |json: &Value| {
        json.pointer("/attributes/groups/membership/value")
            .and_then(Value::as_u64)
    };
    wait_until(
        || fetch_physical_device_full(port, TEST_ADAPTER_ID, 0).as_ref().and_then(read) == Some(2),
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_physical_device_full(port, TEST_ADAPTER_ID, 0);
    assert_eq!(
        json.as_ref().and_then(read),
        Some(2),
        "healed membership must be G1 only: {json:?}"
    );
}

fn script_tc_limit_physical_pair(mock: &MockDaliTransport) {
    script_dt8_colour_value(mock, 129, 0x00, 153);
    script_dt8_colour_value(mock, 131, 0x01, 0x72);
}

fn script_tc_limit_store_frames(mock: &MockDaliTransport, selector: u8, mirek: u16) {
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

// PD-181
#[given("a tc-limit write script for short address 0 storing coolest 200 and warmest 350")]
async fn given_tc_limit_write_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_tc_limit_physical_pair(&mock);
    script_tc_limit_store_frames(&mock, 0, 200);
    script_dt8_colour_value(&mock, 128, 0x00, 200);
    script_tc_limit_store_frames(&mock, 1, 350);
    script_dt8_colour_value(&mock, 130, 0x01, 0x5E);
    script_dt8_colour_value(&mock, 128, 0x00, 200);
    script_dt8_colour_value(&mock, 130, 0x01, 0x5E);
}

// PD-183
#[given("a tc-limit write script where the pair never lands on short address 0")]
async fn given_tc_limit_write_void_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_tc_limit_physical_pair(&mock);
    for _ in 0..3 {
        script_tc_limit_store_frames(&mock, 0, 200);
        script_dt8_colour_value(&mock, 128, 0x00, 153);
    }
}

// PD-181
#[then(
    regex = r"^adapter 0 physical device 0 eventually exposes colour temperature range (\d+) to (\d+) kelvin$"
)]
async fn then_pd_tc_range_kelvin(world: &mut DaliWorld, min_k: u64, max_k: u64) {
    wait_for_physical_device(world, |body| {
        body.pointer("/color_temperature_range/min_kelvin")
            .and_then(Value::as_u64)
            == Some(min_k)
            && body
                .pointer("/color_temperature_range/max_kelvin")
                .and_then(Value::as_u64)
                == Some(max_k)
    });
}

// PD-158
#[given("an extended-fade-time 500ms write-attributes script for short address 0")]
async fn given_extended_fade_write_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(special_frame(SpecialCommand::Dtr0(0x14)));
    let set = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::SetExtendedFadeTime);
    mock.expect_forward_frame(set);
    mock.expect_forward_frame(set);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryExtendedFadeTime),
        Some(0x14),
    );
}

// PD-158
#[given("an extended attribute-read script with fade byte 0x14 for short address 0")]
async fn given_extended_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_attribute_read_prelude(&mock, TEST_SHORT_ADDRESS);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryExtendedFadeTime),
        Some(0x14),
    );
    mock.expect_forward_frame(special_frame(SpecialCommand::EnableDeviceType(6)));
    mock.expect_forward_frame_with_backward(
        extended_frame(
            TEST_SHORT_ADDRESS,
            ExtendedCommand::Dt6(Dt6Command::QueryExtendedVersionNumber),
        ),
        Some(2),
    );
}

// PD-158
#[when(r#"I start an attribute read for adapter 0 physical device 0 with attribute group "extended" only"#)]
async fn when_start_extended_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["extended"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// PD-158
#[then("physical device 0 eventually exposes extended fade_time_ms 500 as read back")]
async fn then_pd_extended_fade_time_read_back(world: &mut DaliWorld) {
    let port = world.server_port();
    let read = |json: &Value| {
        let value = json
            .pointer("/attributes/extended/fade_time_ms/value")
            .and_then(Value::as_u64);
        let read_stamped = json
            .pointer("/attributes/extended/fade_time_ms/last_read_ms")
            .is_some_and(|v| !v.is_null());
        (value == Some(500) && read_stamped).then_some(500u64)
    };
    wait_until(
        || fetch_physical_device_full(port, TEST_ADAPTER_ID, 0).as_ref().and_then(read).is_some(),
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_physical_device_full(port, TEST_ADAPTER_ID, 0);
    assert_eq!(
        json.as_ref().and_then(read),
        Some(500),
        "extended.fade_time_ms must expose the canonical 500 ms with last_read_ms set: {json:?}"
    );
}

// PD-034 PD-156 PD-165 PD-166 PD-167 PD-170 PD-171 PD-178 PD-251
#[when("I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour")]
async fn when_start_runtime_and_colour_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["runtime_status","dt8_color"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// PD-166 PD-168
#[when(r#"I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only"#)]
async fn when_start_runtime_status_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["runtime_status"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// PD-165 PD-166 MQTT-012 PD-168 PD-171
#[then(regex = r"^adapter 0 physical device 0 eventually exposes runtime colour temperature (\d+) K$")]
async fn then_pd_runtime_colour_temperature(world: &mut DaliWorld, kelvin: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/state/color_temperature_kelvin").and_then(Value::as_u64) == Some(kelvin)
    });
    assert_eq!(
        json.pointer("/state/color_mode").and_then(Value::as_str),
        Some("cct"),
    );
}

// PD-166 PD-168
#[then(regex = r"^adapter 0 physical device 0 still exposes runtime colour temperature (\d+) K$")]
async fn then_pd_runtime_colour_temperature_survives(world: &mut DaliWorld, kelvin: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/state/status/raw").and_then(Value::as_u64) == Some(u64::from(SCRIPTED_RUNTIME_STATUS))
    });
    assert_eq!(
        json.pointer("/state/color_temperature_kelvin").and_then(Value::as_u64),
        Some(kelvin),
        "a read that never asked for the colour must not overwrite it",
    );
}

// PD-034 PD-156
#[when(r#"I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only"#)]
async fn when_start_common102_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["common_102"],"memory_banks":"none"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// PD-163 PD-169 PD-164 SYS-231 SYS-232
#[then(regex = r#"^the operation attribute-read outcomes should show "(\w+)" as "(\w+)"$"#)]
async fn then_attribute_read_outcome_for_section(
    world: &mut DaliWorld,
    section: String,
    outcome: String,
) {
    let json = last_json(world);
    let outcomes = json
        .pointer("/attribute_read_outcomes")
        .unwrap_or_else(|| panic!("attribute_read_outcomes in operation view: {json:?}"));
    assert_eq!(
        outcomes.pointer(&format!("/{section}")).and_then(Value::as_str),
        Some(outcome.as_str()),
        "outcomes: {outcomes:?}"
    );
}

// PD-156
#[then(r#"the operation attribute-read outcomes should show "common_102" as "contended_abort" and "identity" as "success""#)]
async fn then_attribute_read_outcomes_classified(world: &mut DaliWorld) {
    let json = last_json(world);
    let outcomes = json
        .pointer("/attribute_read_outcomes")
        .unwrap_or_else(|| panic!("attribute_read_outcomes in operation view: {json:?}"));
    assert_eq!(
        outcomes.pointer("/common_102").and_then(Value::as_str),
        Some("contended_abort"),
        "outcomes: {outcomes:?}"
    );
    assert_eq!(
        outcomes.pointer("/identity").and_then(Value::as_str),
        Some("success"),
        "outcomes: {outcomes:?}"
    );
    assert_eq!(
        outcomes.pointer("/scenes").and_then(Value::as_str),
        Some("not_requested"),
        "outcomes: {outcomes:?}"
    );
    assert_eq!(
        outcomes.pointer("/memory_banks").and_then(Value::as_str),
        Some("not_requested"),
        "outcomes: {outcomes:?}"
    );
}

// PD-155
#[given("an attribute-read identity script where a contended physical minimum reply is corrected by content-confirm")]
async fn given_content_confirm_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity_with_content_confirm(&mock);
}

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-012 MQTT-013 MQTT-015 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-167 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-178 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-189 PD-190 PD-191 PD-192 PD-193 PD-194 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-240 PD-241 PD-242 PD-243 PD-250 PD-251 PD-252 PD-253 PD-254 PD-255 PERS-005 STATS-005 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 WS-003 WS-004 WS-010 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024
#[when("I start a discovery run for adapter 0")]
async fn when_start_discovery_run(world: &mut DaliWorld) {
    let body = br#"{"mode":"scan_known_short_addresses"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(body),
        "application/json",
    );
}

// PD-106
#[when("I start a refresh discovery run for adapter 0")]
async fn when_start_refresh_discovery_run(world: &mut DaliWorld) {
    let body = br#"{"mode":"refresh_known"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/discovery-runs",
        Some(body),
        "application/json",
    );
}

// PD-198 PD-199
#[when(r#"I start an attribute read for adapter 0 physical device 0 with memory_banks "profile""#)]
async fn when_start_attribute_read_profile(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["runtime_status","common_102","dt8_color"],"memory_banks":"profile"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// OP-100 PD-150 PD-155 PD-197 SYS-217 PD-163 PD-195 PD-196 PD-220 PD-221 PD-230 PERS-005
#[when(r#"I start an attribute read for adapter 0 physical device 0 with memory_banks "identity""#)]
async fn when_start_attribute_read(world: &mut DaliWorld) {
    let body = br#"{"attribute_groups":["runtime_status","common_102","dt8_color"],"memory_banks":"identity"}"#;
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(body),
        "application/json",
    );
}

// ADP-022 ADP-023 COMM-001 COMM-004 COMM-008 COMM-010 COMM-030 COMM-032 COMM-034 COMM-036 COMM-038 COMM-052 COMM-056 COMM-057 COMM-092 GRP-030 GRP-063 HCL-020 HCL-022 HCL-027 HCL-028 HCL-030 HCL-051 HCL-054 HCL-056 HCL-057 HCL-060 HCL-061 HCL-062 HCL-063 HCL-064 MQTT-001 MQTT-003 MQTT-005 MQTT-007 MQTT-009 MQTT-012 MQTT-013 MQTT-014 MQTT-015 MQTT-017 MQTT-018 MQTT-019 OP-100 OP-132 PD-027 PD-028 PD-029 PD-030 PD-034 PD-035 PD-036 PD-037 PD-040 PD-041 PD-042 PD-043 PD-060 PD-061 PD-062 PD-063 PD-102 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 PD-164 PD-165 PD-166 PD-167 PD-168 PD-169 PD-170 PD-171 PD-176 PD-177 PD-178 PD-179 PD-180 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 PD-189 PD-190 PD-191 PD-192 PD-193 PD-195 PD-196 PD-197 PD-198 PD-199 PD-200 PD-201 PD-220 PD-221 PD-222 PD-230 PD-241 PD-242 PD-243 PD-250 PD-251 PD-252 PD-253 PD-254 PD-255 PERS-005 REG-030 REG-031 RULE-003 RULE-004 RULE-005 RULE-006 RULE-020 RULE-021 RULE-022 RULE-023 RULE-024 SCN-040 SCN-041 SCN-046 SCN-050 SCN-060 SCN-062 SCN-063 SCN-083 SCN-084 SCN-085 SCN-086 SCN-092 SCN-093 SET-HA-020 STATS-005 SYS-210 SYS-211 SYS-213 SYS-217 SYS-230 SYS-231 SYS-232 SYS-233 SYS-234 SYS-235 SYS-236 SYS-239 SYS-240 SYS-241 WS-003 WS-004 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 WS-046 MQTT-024
#[then("the last operation eventually succeeds")]
async fn then_last_operation_eventually_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status(world, "succeeded");
}

// PD-240
#[then("the last operation eventually succeeds within the full-segment budget")]
async fn then_full_segment_operation_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status_within(world, "succeeded", FULL_SEGMENT_OPERATION_TIMEOUT);
}

// GRP-066
#[then("the last operation eventually succeeds within the group-apply pacing budget")]
async fn then_group_apply_operation_succeeds(world: &mut DaliWorld) {
    wait_for_operation_status_within(world, "succeeded", GROUP_APPLY_PACING_TIMEOUT);
}

// ADP-021 ADP-022 ADP-023 INP-077 INP-080 INP-081 PD-103 PD-156 PD-164 PD-169 PD-183 PD-194 RULE-022 SCN-062 SYS-231 SYS-233 ADP-025 ADP-026
#[then("the last operation eventually fails")]
async fn then_last_operation_eventually_fails(world: &mut DaliWorld) {
    wait_for_operation_status(world, "failed");
}

// ADP-021 ADP-022 ADP-023 PD-164 PD-169 PD-183 SYS-231 SYS-233 ADP-025 ADP-026
#[then(regex = r#"^the operation error code should be "(\w+)"$"#)]
async fn then_operation_error_code(world: &mut DaliWorld, code: String) {
    let json = last_json(world);
    assert_eq!(
        json.pointer("/error/code").and_then(Value::as_str),
        Some(code.as_str()),
        "operation view: {json:?}"
    );
}

// ADP-021 ADP-022 ADP-023 INP-077 INP-080 INP-081 PD-183 RULE-022 ADP-025 ADP-026
#[then(regex = r#"^the operation error message should be "(\w+)"$"#)]
async fn then_operation_error_message(world: &mut DaliWorld, message: String) {
    let json = last_json(world);
    assert_eq!(
        json.pointer("/error/message").and_then(Value::as_str),
        Some(message.as_str()),
        "operation view: {json:?}"
    );
}

// PD-194
#[then("the operation error message should mention several responders")]
async fn then_operation_error_mentions_several_responders(world: &mut DaliWorld) {
    let json = last_json(world);
    let message = json
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("query_short_address_multiple"),
        "expected a several-responders diagnosis, got {message:?} in {json:?}"
    );
}

// PD-102
#[then("the discovery transport trace should match the golden control-gear identity flow")]
async fn then_discovery_trace_matches(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_frame_before(
        &frames,
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryControlGearPresent),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryRandomAddressH),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Initialise(0x00)),
        special_frame(SpecialCommand::SearchAddrH(0x5C)),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::SearchAddrL(0xC2)),
        special_frame(SpecialCommand::QueryShortAddress),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::QueryShortAddress),
        special_frame(SpecialCommand::Withdraw),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Terminate),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
    );
}

// PD-104
#[then("the discovery transport trace should re-arm before re-asking QueryShortAddress")]
async fn then_discovery_trace_rearms_before_requery(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let query_short = special_frame(SpecialCommand::QueryShortAddress);
    let query_positions: Vec<_> = frames
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| (*frame == query_short).then_some(index))
        .collect();
    assert_eq!(
        query_positions.len(),
        2,
        "expected exactly one re-ask after the violating answer; trace: {frames:?}"
    );
    let search_h = special_frame(SpecialCommand::SearchAddrH(0x5C));
    let rearm = frames[query_positions[0]..query_positions[1]]
        .iter()
        .any(|frame| *frame == search_h);
    assert!(
        rearm,
        "the second ask must be preceded by a fresh search address, not sent blind: {frames:?}"
    );
    assert_frame_before(&frames, query_short, special_frame(SpecialCommand::Withdraw));
}

// PD-105
#[then("the discovery transport trace should enumerate DT6 and DT8 after a MASK device-type advertisement")]
async fn then_discovery_trace_enumerates_mask(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let query_device_type = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType);
    let query_next = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryNextDeviceType);
    let next_positions: Vec<_> = frames
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| (*frame == query_next).then_some(index))
        .collect();
    assert_eq!(
        next_positions.len(),
        3,
        "expected three QueryNextDeviceType frames after MASK advertisement; trace: {frames:?}"
    );
    assert_frame_before(&frames, query_device_type, query_next);
    assert_frame_before(
        &frames,
        query_next,
        special_frame(SpecialCommand::EnableDeviceType(8)),
    );
}

// PD-102 PD-104 PD-105 PD-106 PD-189 PD-190 PD-192 PD-193
#[then(regex = r"^adapter 0 physical device 0 eventually declares device types ([0-9, ]+)$")]
async fn then_declares_device_types(world: &mut DaliWorld, list: String) {
    let expected: Vec<u64> = list
        .split(',')
        .map(|t| t.trim().parse::<u64>().expect("device type number"))
        .collect();
    let want = json!(expected);
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/supported_device_types") == Some(&want)
    });
    assert_eq!(
        json.pointer("/supported_device_types"),
        Some(&want),
        "declared device types: {json:?}"
    );
}

// PD-191
#[then("adapter 0 physical device 0 eventually declares no known device types")]
async fn then_declares_no_known_device_types(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/device_type_discovered").and_then(Value::as_str) == Some("unknown")
    });
    assert_eq!(
        json.pointer("/supported_device_types"),
        None,
        "an unfinished enumeration must report no set at all: {json:?}"
    );
}

// PD-102 PD-104 PD-105 PD-106
#[then("adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity")]
async fn then_discovery_state_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/random_address").and_then(Value::as_u64) == Some(u64::from(TEST_RANDOM_ADDRESS))
            && body.pointer("/device_type_discovered").and_then(Value::as_str) == Some("dt8_color")
            && body.pointer("/color_mode_discovered").and_then(Value::as_str) == Some("cct")
    });
    assert_eq!(json.pointer("/random_address").and_then(Value::as_u64), Some(u64::from(TEST_RANDOM_ADDRESS)));
    assert_eq!(json.pointer("/capabilities/cct"), Some(&json!(true)));
    assert_eq!(json.pointer("/capabilities/xy"), Some(&json!(false)));
    assert_eq!(json.pointer("/capabilities/rgb"), Some(&json!(false)));
}

// PD-240
#[then("adapter 0 physical devices eventually include all 64 short addresses")]
async fn then_full_segment_devices_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_devices(world, |body| {
        body.pointer("/physical_devices")
            .and_then(Value::as_array)
            .is_some_and(|devices| devices.len() == 64)
    });
    let devices = json
        .pointer("/physical_devices")
        .and_then(Value::as_array)
        .expect("physical devices array");
    let mut shorts: Vec<u64> = devices
        .iter()
        .filter_map(|d| d.pointer("/short_address").and_then(Value::as_u64))
        .collect();
    shorts.sort_unstable();
    assert_eq!(
        shorts,
        (0..64u64).collect::<Vec<_>>(),
        "every short address the scan announced should be in the registry"
    );
}

// PD-103 PD-107
#[then("adapter 0 physical devices eventually include only the verified device 0")]
async fn then_partial_discovery_state_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_devices(world, |body| {
        let Some(devices) = body.pointer("/physical_devices").and_then(Value::as_array) else {
            return false;
        };
        devices.len() == 1
            && devices[0].pointer("/short_address").and_then(Value::as_u64) == Some(0)
            && devices[0].pointer("/random_address").and_then(Value::as_u64)
                == Some(u64::from(TEST_RANDOM_ADDRESS))
    });
    let devices = json
        .pointer("/physical_devices")
        .and_then(Value::as_array)
        .expect("physical devices array");
    assert_eq!(devices.len(), 1, "expected one verified device: {json:?}");
    assert_eq!(
        devices[0].pointer("/short_address").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        devices[0].pointer("/random_address").and_then(Value::as_u64),
        Some(u64::from(TEST_RANDOM_ADDRESS))
    );
}

// PD-150
#[then("the attribute-read transport trace should include DT8 content-DTR0 and bank 0/1 reads")]
async fn then_attribute_trace_matches(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert_frame_before(
        &frames,
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryStatus),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryActualLevel),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Dtr0(0)),
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
    );
    assert_frame_before(
        &frames,
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryContentDtr0),
        extended_frame(TEST_SHORT_ADDRESS, ExtendedCommand::Dt8(Dt8Command::QueryColourValue)),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Dtr1(0)),
        dt8_raw_query_frame(TEST_SHORT_ADDRESS, READ_MEMORY_LOCATION_OPCODE),
    );
    assert_frame_before(
        &frames,
        special_frame(SpecialCommand::Dtr1(0)),
        special_frame(SpecialCommand::Dtr1(1)),
    );
}

// PD-155
#[then("the attribute-read transport trace should retry the contended physical minimum query until content confirms")]
async fn then_attribute_trace_retries_contended_common102(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let physical_minimum = standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryPhysicalMinimum);
    let count = frames.iter().filter(|frame| **frame == physical_minimum).count();
    assert_eq!(
        count,
        3,
        "expected QueryPhysicalMinimum to be read three times for content-confirm; trace: {frames:?}"
    );
}

// PD-150 PD-155 SYS-217 PD-195
#[then("adapter 0 physical device 0 eventually reports status flags lamp_on and power_cycle_seen")]
async fn then_status_named_bits(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/state/status/lamp_on").and_then(Value::as_bool) == Some(true)
    });
    assert_eq!(
        json.pointer("/state/status/raw").and_then(Value::as_u64),
        Some(0x84),
        "bits 7 and 2 — the fixture the named flags are decoded from"
    );
    assert_eq!(
        json.pointer("/state/status/lamp_on").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        json.pointer("/state/status/power_cycle_seen")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        json.pointer("/state/status/power_failure").is_none(),
        "the old key is gone, not carried as an alias"
    );
}

// PD-196
#[then(regex = r"^adapter 0 physical device 0 eventually reports light source type (\d+)$")]
async fn then_light_source_type(world: &mut DaliWorld, expected: u64) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/common_102/light_source_type/value")
            .and_then(Value::as_u64)
            == Some(expected)
    });
    assert_eq!(
        json.pointer("/attributes/common_102/light_source_type/value")
            .and_then(Value::as_u64),
        Some(expected)
    );
    assert!(
        json.pointer("/attributes/common_102/light_source_types")
            .is_none(),
        "a concrete answer carries no MASK triple"
    );
}

// PD-197
#[then("adapter 0 physical device 0 eventually exposes the bus unit configuration and implemented parts")]
async fn then_bus_unit_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/memory_bus_unit/configuration/value/raw")
            .and_then(Value::as_u64)
            == Some(BANK0_BUS_UNIT_CONFIGURATION)
    });
    assert_eq!(
        json.pointer("/attributes/memory_bus_unit/configuration/value/class")
            .and_then(Value::as_str),
        Some("unnamed row"),
        "Table 5 rows 0-8 are illegible in the repository's copy of 098bp, so \
         the number travels and the label says exactly that"
    );
    assert!(
        json.pointer("/attributes/memory_bus_unit/configuration/value/emergency_type")
            .is_none(),
        "only rows 9-12 carry a Part 202 emergency type letter"
    );
    assert_eq!(
        json.pointer("/attributes/memory_bus_unit/implemented_parts/value/raw")
            .and_then(Value::as_u64),
        Some(BANK0_IMPLEMENTED_PARTS_RAW),
    );
    assert_eq!(
        json.pointer("/attributes/memory_bus_unit/implemented_parts/value/parts"),
        Some(&json!(BANK0_IMPLEMENTED_PARTS)),
        "098bp Table 4: bit x of 0x1C is Part 15x, so bits 0 and 2 are Parts 150 and 152"
    );
}

// PD-198
#[then("adapter 0 physical device 0 eventually exposes the Part 251 luminaire data")]
async fn then_part251_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/memory_luminaire/content_format_id/value")
            .and_then(Value::as_u64)
            == Some(3)
    });
    let ident = json
        .pointer("/attributes/memory_luminaire/luminaire_identification/value")
        .and_then(Value::as_str)
        .expect("luminaire identification");
    assert!(
        ident.starts_with("DALI2RUST BENCH LUMINAIRE"),
        "the 60-character region is read whole across chunks, got {ident:?}"
    );
    assert_eq!(
        json.pointer("/attributes/memory_luminaire/luminaire_colour/value")
            .and_then(Value::as_str),
        Some("warm white"),
    );
    assert_eq!(
        json.pointer("/attributes/memory_luminaire/nominal_min_ac_voltage_v/value/value")
            .and_then(Value::as_u64),
        Some(198),
    );
    assert!(json
        .pointer("/attributes/memory_luminaire/cri/value/raw")
        .and_then(Value::as_u64)
        .is_some());
    assert!(json
        .pointer("/attributes/memory_luminaire/light_distribution/value")
        .is_none());
    assert!(json
        .pointer("/attributes/memory_luminaire/oem_name/value")
        .is_none());
}

// PD-199
#[then("adapter 0 physical device 0 eventually reports the bank 1 content format without luminaire fields")]
async fn then_vendor_bank1_yields_nothing(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/attributes/memory_luminaire/content_format_id/value")
            .and_then(Value::as_u64)
            == Some(0x0042)
    });
    for field in [
        "year",
        "week",
        "cri",
        "cct_kelvin",
        "nominal_input_power_w",
        "nominal_light_output_lm",
        "luminaire_colour",
        "luminaire_identification",
    ] {
        assert!(
            json.pointer(&format!("/attributes/memory_luminaire/{field}/value"))
                .is_none(),
            "{field} was produced from manufacturer-specific bytes"
        );
    }
    assert_eq!(
        json.pointer("/attributes/memory_profile/oem_gtin/value")
            .and_then(Value::as_u64),
        Some(BANK1_OEM_GTIN),
    );
}

// PD-150 PD-155 SYS-217
#[then("adapter 0 physical device 0 eventually exposes the golden runtime status and memory-bank identity")]
async fn then_attribute_state_exposed(world: &mut DaliWorld) {
    let json = wait_for_physical_device(world, |body| {
        body.pointer("/state/status/raw").and_then(Value::as_u64) == Some(0x84)
            && body.pointer("/random_address").and_then(Value::as_u64) == Some(u64::from(TEST_RANDOM_ADDRESS))
            && body.pointer("/attributes/memory_identity/gtin/value").and_then(Value::as_u64) == Some(BANK0_GTIN)
            && body.pointer("/attributes/memory_profile/oem_gtin/value").and_then(Value::as_u64)
                == Some(BANK1_OEM_GTIN)
            && body
                .pointer("/attributes/common_102/physical_minimum/value")
                .and_then(Value::as_u64)
                == Some(1)
    });
    assert_eq!(json.pointer("/state/status/raw").and_then(Value::as_u64), Some(0x84));
    assert_eq!(
        json.pointer("/attributes/common_102/physical_minimum/value")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        json.pointer("/attributes/dt8_color/color_value_1/value")
            .and_then(Value::as_u64),
        Some(0x1234),
    );
    assert_eq!(
        json.pointer("/attributes/memory_identity/gtin/value")
            .and_then(Value::as_u64),
        Some(BANK0_GTIN),
    );
    assert_eq!(
        json.pointer("/attributes/memory_profile/oem_gtin/value")
            .and_then(Value::as_u64),
        Some(BANK1_OEM_GTIN),
    );
    assert_eq!(
        json.pointer("/attributes/memory_profile/oem_identification_number/value")
            .and_then(Value::as_u64),
        Some(BANK1_OEM_ID),
    );
    assert_eq!(json.pointer("/memory_banks/0/bank").and_then(Value::as_u64), Some(0));
    assert_eq!(
        json.pointer("/memory_banks/0/total_bytes_read")
            .and_then(Value::as_u64),
        Some(BANK0_BYTES.len() as u64),
    );
    assert_eq!(json.pointer("/memory_banks/1/bank").and_then(Value::as_u64), Some(1));
    assert_eq!(
        json.pointer("/memory_banks/1/total_bytes_read")
            .and_then(Value::as_u64),
        Some(BANK1_BYTES.len() as u64),
    );
}

// PD-163
#[then("adapter 0 physical device 0 eventually exposes bank 0 at the length the gear proved")]
async fn then_physical_device_exposes_short_bank0(world: &mut DaliWorld) {
    let port = world.server_port();
    let bank_bytes = |json: &Value, bank: usize| {
        json.pointer(&format!("/memory_banks/{bank}/total_bytes_read"))
            .and_then(Value::as_u64)
    };
    wait_until(
        || {
            fetch_physical_device_full(port, TEST_ADAPTER_ID, 0).as_ref().and_then(|j| bank_bytes(j, 0))
                == Some(SHORT_BANK0_BYTES as u64)
        },
        READ_MODEL_TIMEOUT,
    );
    let json = fetch_physical_device_full(port, TEST_ADAPTER_ID, 0);
    assert_eq!(
        json.as_ref().and_then(|j| bank_bytes(j, 0)),
        Some(SHORT_BANK0_BYTES as u64),
        "bank 0 is committed at the length the gear proved, not the header's: {json:?}"
    );
    assert_eq!(
        json.as_ref().and_then(|j| bank_bytes(j, 1)),
        Some(BANK1_BYTES.len() as u64),
        "the sweep must reach bank 1, which an aborted bank 0 never did: {json:?}"
    );
}

// GRP-066 PD-030 PD-034 PD-040 PD-102 PD-103 PD-104 PD-105 PD-106 PD-107 PD-150 PD-155 PD-156 PD-157 PD-158 PD-159 PD-163 VL-020 VL-054 PD-169 MQTT-012 MQTT-017 PD-181 PD-183 PD-184 PD-185 PD-186 PD-188 SCN-083 SCN-084 SCN-085 COMM-010 COMM-034 COMM-038 COMM-080 COMM-081 COMM-092 PD-037 PD-164 PD-165 PD-166 PD-167 PD-168 PD-170 PD-171 PD-189 PD-190 PD-191 PD-194 PD-195 PD-196 PD-197 PD-198 PD-199 PD-240 PD-241 PD-242 PD-243 PD-250 PD-251 PD-252 PD-253 PD-254 SCN-060 SCN-092 SCN-093 SYS-234
#[then("all scripted DALI exchanges should be consumed without errors")]
async fn then_all_scripted_exchanges_consumed(world: &mut DaliWorld) {
    assert_no_script_errors(world);
}

const FULL_ATTRIBUTE_READ_BODY: &[u8] = br#"{"attribute_groups":["runtime_status","common_102","dt8_color","dt6_led","extended","groups","scenes"]}"#;

// SYS-230 SYS-231 SYS-232
#[when("I start a full attribute read for adapter 0 physical device 0")]
async fn when_start_full_attribute_read(world: &mut DaliWorld) {
    world.send_http_request(
        "POST",
        "/api/v1/adapters/0/physical-devices/0/attribute-reads",
        Some(FULL_ATTRIBUTE_READ_BODY),
        "application/json",
    );
}

// SYS-234
#[then(regex = r"^the operator setpoint for level (\d+) on short address (\d+) should eventually reach the wire$")]
async fn then_setpoint_eventually_on_wire(world: &mut DaliWorld, level: u8, short: u8) {
    let expected = standard_frame(short, StandardCommand::DirectArcPower { level });
    let mock = std::sync::Arc::clone(world.dali_mock());
    wait_until(
        || {
            mock.lock()
                .expect("mock lock")
                .sent_frames()
                .contains(&expected)
        },
        READ_MODEL_TIMEOUT,
    );
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    assert!(
        frames.contains(&expected),
        "the operator setpoint 0x{expected:04X} was dropped rather than deferred: {frames:?}"
    );
}

// SYS-230
#[then(regex = r"^the operator setpoint for level (\d+) on short address (\d+) should reach the wire within (\d+) frames$")]
async fn then_setpoint_within_frames(world: &mut DaliWorld, level: u8, short: u8, budget: usize) {
    let expected = standard_frame(short, StandardCommand::DirectArcPower { level });
    let frames = world
        .dali_mock()
        .lock()
        .expect("mock lock")
        .sent_frames();
    let Some(index) = frames.iter().position(|frame| *frame == expected) else {
        panic!("the operator setpoint 0x{expected:04X} never reached the wire: {frames:?}");
    };
    assert!(
        index >= 1,
        "the attended work must have been on the wire, or this proves nothing: {frames:?}"
    );
    assert!(
        index < budget,
        "the operator setpoint 0x{expected:04X} landed at frame {index}, behind \
         {index} frames of attended work; the budget is {budget}: {frames:?}"
    );
}
