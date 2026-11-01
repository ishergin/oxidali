use cucumber::{given, then};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use serde_json::Value;

use crate::steps::frames::{raw_short_command_frame, standard_frame};
use crate::DaliWorld;

use super::attribute_read_scripts::script_attribute_read_prelude;
use super::read_model::{fetch_physical_device_full, wait_for_physical_device};
use super::{TEST_ADAPTER_ID, TEST_SHORT_ADDRESS};

const ENABLE_DEVICE_TYPE_7: u16 = 0xC107;
// IEC 62386-208 §11.3.4.2
const PART_208_REGISTERS: [(&str, u8, u8); 8] = [
    ("features", 0xF0, 0x19),
    ("switch_status", 0xF1, 0x02),
    ("up_switch_on_threshold", 0xF2, 0x40),
    ("up_switch_off_threshold", 0xF3, 0xFF),
    ("down_switch_on_threshold", 0xF4, 0x30),
    ("down_switch_off_threshold", 0xF5, 0x10),
    ("error_hold_off", 0xF6, 0x06),
    ("gear_type", 0xF7, 0x02),
];

fn script_switching_read(mock: &MockDaliTransport, declared_type: u8) {
    const SWITCHING: u8 = 7;
    mock.clear();
    script_attribute_read_prelude(mock, TEST_SHORT_ADDRESS);
    mock.expect_forward_frame_with_backward(
        standard_frame(TEST_SHORT_ADDRESS, StandardCommand::QueryDeviceType),
        Some(declared_type),
    );
    if declared_type != SWITCHING {
        return;
    }
    for (_, opcode, answer) in PART_208_REGISTERS {
        mock.expect_forward_frame(ENABLE_DEVICE_TYPE_7);
        mock.expect_forward_frame_with_backward(raw_short_command_frame(TEST_SHORT_ADDRESS, opcode), Some(answer));
    }
}

// PD-310 PD-311
#[given(regex = r"^a switching attribute-read script for a gear declaring only device type (\d+)$")]
async fn given_switching_read_script(world: &mut DaliWorld, declared_type: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_switching_read(&mock, declared_type);
}

// PD-310
#[then("physical device 0 eventually exposes the scripted Part 208 registers")]
async fn then_pd_exposes_part_208_registers(world: &mut DaliWorld) {
    let register = |body: &Value, name: &str| {
        body.pointer(&format!("/attributes/dt7_switching/{name}/value")).and_then(Value::as_u64)
    };
    let json = wait_for_physical_device(world, |body| register(body, "gear_type").is_some());
    for (name, _, answer) in PART_208_REGISTERS {
        assert_eq!(
            register(&json, name),
            Some(u64::from(answer)),
            "IEC 62386-208 §11.3.4.2: {name} is the byte the gear answered: {json}"
        );
    }
}

// PD-311
#[then("no Part 208 frame reached the wire")]
async fn then_no_part_208_frame_was_sent(world: &mut DaliWorld) {
    let frames = world.dali_mock().lock().expect("mock lock").sent_frames();
    let part_208 = |frame: &u16| {
        *frame == ENABLE_DEVICE_TYPE_7
            || PART_208_REGISTERS
                .iter()
                .any(|(_, opcode, _)| *frame == raw_short_command_frame(TEST_SHORT_ADDRESS, *opcode))
    };
    assert!(
        !frames.iter().any(part_208),
        "a gear that does not declare device type 7 gets no Part 208 frame: {frames:04X?}"
    );
}

// PD-311
#[then("physical device 0 exposes no dt7_switching section")]
async fn then_pd_exposes_no_switching_section(world: &mut DaliWorld) {
    let json = fetch_physical_device_full(world.server_port(), TEST_ADAPTER_ID, TEST_SHORT_ADDRESS)
        .expect("the device is registered");
    assert!(
        json.pointer("/attributes/dt7_switching").is_none(),
        "a gear that is not switching gear has no Part 208 section: {json}"
    );
}
