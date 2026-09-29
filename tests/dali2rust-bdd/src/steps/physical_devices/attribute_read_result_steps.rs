use cucumber::then;
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::dt8_color::Dt8Command;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_test_support::wait_until;
use serde_json::{json, Value};

use crate::steps::frames::{dt8_raw_query_frame, extended_frame, special_frame, standard_frame};
use crate::steps::last_json;
use crate::steps::wire::assert_frame_before;
use crate::DaliWorld;

use super::attribute_read_scripts::SCRIPTED_RUNTIME_STATUS;
use super::memory_bank_scripts::{
    BANK0_BUS_UNIT_CONFIGURATION, BANK0_BYTES, BANK0_GTIN, BANK0_IMPLEMENTED_PARTS,
    BANK0_IMPLEMENTED_PARTS_RAW, BANK1_BYTES, BANK1_OEM_GTIN, BANK1_OEM_ID,
    READ_MEMORY_LOCATION_OPCODE, SHORT_BANK0_BYTES,
};
use super::read_model::{
    fetch_physical_device, fetch_physical_device_full, wait_for_physical_device,
    READ_MODEL_TIMEOUT,
};
use super::{TEST_ADAPTER_ID, TEST_RANDOM_ADDRESS, TEST_SHORT_ADDRESS};

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

// PD-269
#[then("adapter 0 physical device 0 eventually holds no colour state from before the power cycle")]
async fn then_pd_forgets_ram_colour(world: &mut DaliWorld) {
    const RAM_ATTRIBUTES: [&str; 5] =
        ["color_value_0", "color_value_1", "color_value_2", "gear_features", "rgbwaf_control"];
    let forgotten = |body: &Value| {
        body.pointer("/state/rgb").is_none_or(Value::is_null)
            && RAM_ATTRIBUTES
                .iter()
                .all(|name| body.pointer(&format!("/attributes/dt8_color/{name}")).is_none())
    };
    let json = wait_for_physical_device(world, forgotten);
    assert!(
        forgotten(&json),
        "IEC 62386-102 §9.16.9 and 209 Table 8: a gear that reports a power cycle has lost \
         its RAM colour state, so what the registry held from before must be gone: {json}"
    );
}

// PD-167 PD-251 PD-269
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

// PD-270
#[then(regex = r"^physical device 0 eventually exposes extended versions (\d+):(\d+) and (\d+):(\d+)$")]
async fn then_pd_extended_versions(world: &mut DaliWorld, t1: u64, v1: u64, t2: u64, v2: u64) {
    let wanted = [(t1, v1), (t2, v2)];
    let holds = |body: &Value| {
        let versions = body.pointer("/extended_versions").and_then(Value::as_array);
        versions.is_some_and(|list| {
            wanted.iter().all(|(t, v)| {
                list.iter().any(|entry| {
                    entry.get("device_type").and_then(Value::as_u64) == Some(*t)
                        && entry.get("version_number").and_then(Value::as_u64) == Some(*v)
                })
            })
        })
    };
    let json = wait_for_physical_device(world, holds);
    assert!(
        holds(&json),
        "IEC 62386-102 §11.6.2: every declared device type answers its own extended \
         version behind its own ENABLE DEVICE TYPE: {json}"
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
        Some("207 LED, 3 logical units"),
        "DiiA 098bp Table 5 names row 2: three logical units of 207"
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
