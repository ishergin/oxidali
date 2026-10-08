use cucumber::{given, when};
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::Value;

use crate::steps::frames::standard_frame;
use crate::steps::polling::wait_for_operation_status;
use crate::DaliWorld;

use super::attribute_read_scripts::{
    script_attribute_read_colour_unanswered, script_attribute_read_faulted_gear,
    script_attribute_read_identity, script_attribute_read_identity_queries,
    script_attribute_read_identity_short_bank0,
    script_attribute_read_identity_with_content_confirm, script_attribute_read_no_probe,
    script_attribute_read_prelude, script_attribute_read_rgb_active,
    script_attribute_read_rgb_active_with_status, script_attribute_read_runtime_and_colour,
    script_extended_section, script_groups_attribute_read, ScriptedReply, GOLDEN_GEAR_FEATURES,
    PD165_TC_MIREK, SCRIPTED_RUNTIME_STATUS,
};
use super::memory_bank_scripts::{bank1_part251_bytes, script_memory_bank_read, BANK0_BYTES};
use super::read_model::{fetch_physical_device_full, physical_device_path, READ_MODEL_TIMEOUT};
use super::{TEST_ADAPTER_ID, TEST_SHORT_ADDRESS};

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

// PD-168 PD-257 PD-258 PD-269
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

// PD-269
#[given(regex = r"^an attribute-read script where short address 0 is RGB-active at (\d+) (\d+) (\d+) and reports no power cycle$")]
async fn given_attribute_read_rgb_no_power_cycle_script(world: &mut DaliWorld, r: u8, g: u8, b: u8) {
    const STATUS_LAMP_ON_WITHOUT_POWER_CYCLE: u8 = SCRIPTED_RUNTIME_STATUS & !0x80;
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_rgb_active_with_status(&mock, (r, g, b), STATUS_LAMP_ON_WITHOUT_POWER_CYCLE);
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

const EXTENDED_FADE_HALF_SECOND: u8 = 0x14;
const EXTENDED_FADE_SEVENTY_SECONDS: u8 = 0x36;
const DT8: u8 = 8;

// PD-158
#[given("an extended attribute-read script with fade byte 0x14 for short address 0")]
async fn given_extended_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_extended_section(&mock, &[EXTENDED_FADE_HALF_SECOND], &[DT8], &[(DT8, Some(2))]);
}

// PD-273
#[given(
    "an extended attribute-read script where the fade byte first reads 0x36 and then 0x14 for short address 0"
)]
async fn given_extended_fade_corrected_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    let fade = [EXTENDED_FADE_SEVENTY_SECONDS, EXTENDED_FADE_HALF_SECOND];
    script_extended_section(&mock, &fade, &[DT8], &[(DT8, Some(2))]);
}

// PD-270
#[given("an extended attribute-read script for a gear declaring device types 6 and 8")]
async fn given_extended_multi_type_script(world: &mut DaliWorld) {
    const MASK: u8 = 0xFF;
    const WALK_END: u8 = 0xFE;
    let mock = world.dali_mock().lock().expect("mock lock");
    let walk = [MASK, 6, DT8, WALK_END];
    script_extended_section(&mock, &[EXTENDED_FADE_HALF_SECOND], &walk, &[(6, Some(1)), (DT8, Some(0x08))]);
}

// PD-155
#[given("an attribute-read identity script where a contended physical minimum reply is corrected by content-confirm")]
async fn given_content_confirm_attribute_read_script(world: &mut DaliWorld) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_attribute_read_identity_with_content_confirm(&mock);
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

// SCN-085 SCN-096
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

// PD-158 PD-270 PD-273
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

// PD-034 PD-156 PD-165 PD-166 PD-167 PD-170 PD-171 PD-178 PD-251 PD-269
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

// PD-166 PD-168 PD-269
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
