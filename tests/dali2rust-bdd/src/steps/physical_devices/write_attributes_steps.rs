use cucumber::{given, then};
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::{remains_false_for, wait_until, WORKER_SETTLE};
use serde_json::Value;

use crate::steps::frames::{special_frame, standard_frame};
use crate::DaliWorld;

use super::attribute_read_scripts::script_dt8_colour_value;
use super::read_model::{fetch_physical_device_full, wait_for_physical_device, READ_MODEL_TIMEOUT};
use super::write_scripts::{
    script_dimming_curve_write, script_dimming_curve_write_no_clear, script_fade_time_write,
    script_fade_time_write_answered, script_fade_time_write_violated, script_max_triple,
    script_min_triple,
    script_tc_limit_physical_pair, script_tc_limit_store_frames,
};
use super::{TEST_ADAPTER_ID, TEST_SHORT_ADDRESS};

// PD-252 PD-253
#[given(regex = r"^a dimming-curve (\d+) write-attributes script for short address 0$")]
async fn given_dimming_curve_write_script(world: &mut DaliWorld, curve: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_dimming_curve_write(&mock, curve, Some(curve));
}

// PD-254
#[given(
    regex = r"^a dimming-curve (\d+) write script for short address 0 that the gear ignores$"
)]
async fn given_dimming_curve_write_ignored_script(world: &mut DaliWorld, curve: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_dimming_curve_write(&mock, curve, Some(0));
    script_dimming_curve_write_no_clear(&mock, curve, Some(0));
    script_dimming_curve_write_no_clear(&mock, curve, Some(0));
}

// PD-268
#[given(
    regex = r"^a dimming-curve (\d+) write script for short address 0 whose read-back goes unanswered$"
)]
async fn given_dimming_curve_write_unanswered_script(world: &mut DaliWorld, curve: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    script_dimming_curve_write(&mock, curve, None);
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

// PD-267
#[given("a fade-time 500ms write script for short address 0 whose read-back goes unanswered")]
async fn given_fade_time_write_unanswered_script(world: &mut DaliWorld) {
    script_fade_time_write_answered(&world.dali_mock().lock().expect("mock lock"), 1, None);
}

// PD-274
#[given("a fade-time 500ms write script for short address 0 whose read-back holds a violation")]
async fn given_fade_time_write_violated_script(world: &mut DaliWorld) {
    script_fade_time_write_violated(&world.dali_mock().lock().expect("mock lock"), 1);
}

// PD-267 PD-268 PD-274
#[then(
    regex = r"^physical device (\d+) (common_102|dt6_led) (fade_time_ms|dimming_curve) carries no write provenance$"
)]
async fn then_pd_field_without_write_provenance(
    world: &mut DaliWorld,
    short: u8,
    section: String,
    field: String,
) {
    let port = world.server_port();
    let source_ptr = format!("/attributes/{section}/{field}/source");
    let write_confirmed = || {
        fetch_physical_device_full(port, TEST_ADAPTER_ID, short)
            .and_then(|json| json.pointer(&source_ptr).and_then(Value::as_str).map(str::to_owned))
            .as_deref()
            == Some("write_confirmed")
    };
    assert!(
        remains_false_for(write_confirmed, WORKER_SETTLE),
        "{field}: a read-back nobody answered proves nothing"
    );
}

// PD-034
#[given("a fade-time 2000ms write-attributes script for short address 0")]
async fn given_fade_time_2000_write_script(world: &mut DaliWorld) {
    script_fade_time_write(&world.dali_mock().lock().expect("mock lock"), 4);
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
