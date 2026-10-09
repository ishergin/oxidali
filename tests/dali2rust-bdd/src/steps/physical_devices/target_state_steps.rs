use cucumber::{given, then};
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::frames::{dt8_raw_query_frame, special_frame, standard_frame};
use crate::DaliWorld;

use super::read_model::{wait_pd_state_field, READ_MODEL_TIMEOUT};
use super::write_scripts::script_staged_dtrs;
use super::TEST_SHORT_ADDRESS;

// OP-100 OP-130 OP-132 PD-040 PD-041 PD-157 VL-020 WS-046 MQTT-002 MQTT-003 MQTT-005 MQTT-007 MQTT-008 MQTT-013 MQTT-015 MQTT-018 MQTT-019 COMM-092 MQTT-001 WS-003 WS-004 WS-013 WS-030 WS-032 WS-040 WS-041 WS-045 MQTT-024 HCL-077 HCL-078 MQTT-021 MQTT-023 PD-266 RED-023
#[given(regex = r"^a successful target-state script for level (\d+) on short address 0$")]
async fn given_successful_target_state_script(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(standard_frame(
        TEST_SHORT_ADDRESS,
        StandardCommand::DirectArcPower { level },
    ));
}

// PD-040 PD-271
#[given("a power-off target-state script for short address 0")]
async fn given_power_off_target_state_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    mock.expect_forward_frame(standard_frame(
        TEST_SHORT_ADDRESS,
        StandardCommand::DirectArcPower { level: 0 },
    ));
}

// PD-040 PD-157 MQTT-012 MQTT-013 MQTT-019 PD-166 PD-168 WS-059 HCL-077
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

// PD-040 PD-157 PD-271 SYS-217 PD-258
#[then(regex = r"^the physical device (\d+) state level should eventually be (\d+)$")]
async fn then_pd_state_level_eventually(world: &mut DaliWorld, short: u8, level: u8) {
    wait_pd_state_field(world, short, "/state/level", &Value::from(level));
}

// PD-271
#[then(regex = r#"^the physical device (\d+) state power should eventually be "(on|off)"$"#)]
async fn then_pd_state_power_eventually(world: &mut DaliWorld, short: u8, power: String) {
    wait_pd_state_field(world, short, "/state/power", &Value::from(power));
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
