use super::attributes::{write_attribute_section, AttributeSectionDto};
use super::runtime_state::runtime_error_dto;
use super::*;
use dali2rust_contracts::msg::{ErrorCode, FixedText32, FixedText64};
use dali2rust_domain::registry::{
    AttributeSectionKind, AttributeSectionView, AttributeSource, BankReading,
    GearDiagnosticsView, LuminaireMaintenanceView, LuminaireValue, MemoryBusUnitAttributesView,
    MemoryDiagnosticsAttributesView, MemoryEnergyAttributesView, MemoryLuminaireAttributesView,
    ObservedValue, PhysicalDeviceAttributesView, RuntimeErrorView, ScenesAttributesView,
    SourceDiagnosticsView,
};
use std::mem::size_of;

fn observed(value: u8) -> Option<ObservedValue<u8>> {
    Some(ObservedValue {
        value,
        source: AttributeSource::Readback,
        last_read_ms: Some(1_000),
        last_write_confirmed_ms: None,
    })
}

#[test]
fn http_built_dtos_stay_within_the_httpd_stack_budget() {
    let rows: &[(&str, usize, usize)] = &[
        (
            "PhysicalDeviceSummaryDto",
            size_of::<PhysicalDeviceSummaryDto>(),
            512,
        ),
        ("PhysicalDeviceCoreDto", size_of::<PhysicalDeviceCoreDto>(), 512),
        ("GearDiagnosticsDto", size_of::<GearDiagnosticsDto>(), 2_048),
        (
            "SourceDiagnosticsDto",
            size_of::<SourceDiagnosticsDto>(),
            2_048,
        ),
        (
            "MemoryLuminaireAttributesDto",
            size_of::<MemoryLuminaireAttributesDto>(),
            2_048,
        ),
        (
            "MemoryIdentityAttributesDto",
            size_of::<MemoryIdentityAttributesDto>(),
            2_048,
        ),
        ("Dt6LedAttributesDto", size_of::<Dt6LedAttributesDto>(), 2_048),
        ("ScenesAttributesDto", size_of::<ScenesAttributesDto>(), 2_048),
        (
            "MemoryDiagnosticsAttributesDto",
            size_of::<MemoryDiagnosticsAttributesDto>(),
            16,
        ),
        (
            "MemoryEnergyAttributesDto",
            size_of::<MemoryEnergyAttributesDto>(),
            16,
        ),
        ("AttributeSectionDto", size_of::<AttributeSectionDto>(), 16),
    ];
    for (name, actual, ceiling) in rows {
        assert!(
            actual <= ceiling,
            "{name} is {actual} B, ceiling {ceiling} B. \
             Split the response or borrow the view — see the test comment."
        );
    }
}

#[test]
fn a_runtime_error_serializes_as_its_product_code_not_its_discriminant() {
    let dto = runtime_error_dto(Some(RuntimeErrorView {
        code: ErrorCode::VlUnbound,
    }));
    let json = serde_json::to_string(&dto).unwrap();
    assert_eq!(json, r#"{"code":"vl_unbound"}"#);
}

fn every_section(
    attrs: &PhysicalDeviceAttributesView,
    energy: &MemoryEnergyAttributesView,
    diagnostics: &MemoryDiagnosticsAttributesView,
    bus_unit: &MemoryBusUnitAttributesView,
    luminaire: Option<&MemoryLuminaireAttributesView>,
) -> Vec<AttributeSectionView> {
    AttributeSectionKind::ALL
        .iter()
        .map(|kind| match kind {
            AttributeSectionKind::Common102 => {
                AttributeSectionView::Common102(Box::new(attrs.common102))
            }
            AttributeSectionKind::Dt6Led => {
                AttributeSectionView::Dt6Led(Box::new(attrs.dt6_led))
            }
            AttributeSectionKind::Dt8Color => {
                AttributeSectionView::Dt8Color(Box::new(attrs.dt8_color))
            }
            AttributeSectionKind::Extended => {
                AttributeSectionView::Extended(Box::new(attrs.extended))
            }
            AttributeSectionKind::Groups => {
                AttributeSectionView::Groups(Box::new(attrs.groups))
            }
            AttributeSectionKind::MemoryDiagnostics => {
                AttributeSectionView::MemoryDiagnostics(Box::new(diagnostics.clone()))
            }
            AttributeSectionKind::MemoryEnergy => {
                AttributeSectionView::MemoryEnergy(Box::new(*energy))
            }
            AttributeSectionKind::MemoryBusUnit => {
                AttributeSectionView::MemoryBusUnit(Box::new(*bus_unit))
            }
            AttributeSectionKind::MemoryIdentity => {
                AttributeSectionView::MemoryIdentity(Box::new(attrs.memory_identity))
            }
            AttributeSectionKind::MemoryLuminaire => AttributeSectionView::MemoryLuminaire(
                Box::new(luminaire.cloned().unwrap_or_default()),
            ),
            AttributeSectionKind::MemoryProfile => {
                AttributeSectionView::MemoryProfile(Box::new(attrs.memory_profile))
            }
            AttributeSectionKind::Scenes => {
                AttributeSectionView::Scenes(Box::new(attrs.scenes))
            }
        })
        .collect()
}

fn attributes_json(
    attrs: &PhysicalDeviceAttributesView,
    energy: &MemoryEnergyAttributesView,
    diagnostics: &MemoryDiagnosticsAttributesView,
    bus_unit: &MemoryBusUnitAttributesView,
    luminaire: Option<&MemoryLuminaireAttributesView>,
) -> String {
    let sections = every_section(attrs, energy, diagnostics, bus_unit, luminaire);
    let mut out: Vec<u8> = b"{".to_vec();
    let mut first = true;
    for view in &sections {
        write_attribute_section(&mut out, view, &mut first).expect("write section");
    }
    out.push(b'}');
    String::from_utf8(out).expect("utf-8")
}

#[test]
fn empty_attributes_serialize_to_empty_object() {
    let attrs = PhysicalDeviceAttributesView::default();
    let json = attributes_json(
        &attrs,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
    );
    assert_eq!(json, "{}");
}

fn implemented_parts_json(raw: u8) -> serde_json::Value {
    let bus_unit = MemoryBusUnitAttributesView {
        configuration: None,
        implemented_parts: observed(raw),
    };
    let json = attributes_json(
        &PhysicalDeviceAttributesView::default(),
        &Default::default(),
        &Default::default(),
        &bus_unit,
        None,
    );
    let body: serde_json::Value = serde_json::from_str(&json).expect("attributes json");
    body["memory_bus_unit"]["implemented_parts"]["value"].clone()
}

#[test]
fn implemented_parts_travel_as_part_numbers_and_an_out_of_range_byte_claims_none() {
    assert_eq!(
        implemented_parts_json(0x05),
        serde_json::json!({"raw": 5, "parts": [150, 152]})
    );
    assert_eq!(implemented_parts_json(0x00), serde_json::json!({"raw": 0, "parts": []}));
    assert_eq!(
        implemented_parts_json(0xFF),
        serde_json::json!({"raw": 255, "parts": null})
    );
}

fn ov<T>(value: T, seq: u64) -> Option<ObservedValue<T>> {
    Some(if seq % 2 == 1 {
        ObservedValue {
            value,
            source: AttributeSource::Readback,
            last_read_ms: Some(1_000 + seq),
            last_write_confirmed_ms: None,
        }
    } else {
        ObservedValue {
            value,
            source: AttributeSource::WriteConfirmed,
            last_read_ms: None,
            last_write_confirmed_ms: Some(2_000 + seq),
        }
    })
}

fn full_attributes_view() -> PhysicalDeviceAttributesView {
    let mut a = PhysicalDeviceAttributesView::default();
    let c = &mut a.common102;
    c.version = ov(1, 1);
    c.device_type = ov(2, 2);
    c.physical_minimum = ov(3, 3);
    c.min_level = ov(4, 4);
    c.max_level = ov(5, 5);
    c.power_on_level = ov(6, 6);
    c.system_failure_level = ov(7, 7);
    c.fade_time_ms = ov(1_500_u32, 8);
    c.fade_rate = ov(9, 9);
    c.light_source_type = ov(0xFF, 73);
    c.light_source_types = ov(0x0006_02FE_u32, 75);
    a.groups.membership = ov(0xA5A5_u16, 10);
    for (idx, slot) in a.scenes.levels.iter_mut().enumerate() {
        *slot = ov(100 + idx as u8, 11 + idx as u64);
    }
    let d = &mut a.dt6_led;
    d.gear_type = ov(20, 27);
    d.dimming_curve = ov(21, 28);
    d.possible_operating_mode = ov(22, 29);
    d.features = ov(23, 30);
    d.failure_status = ov(24, 31);
    d.short_circuit = ov(25, 32);
    d.open_circuit = ov(26, 33);
    d.load_decrease = ov(27, 34);
    d.load_increase = ov(28, 35);
    d.current_protector_active = ov(29, 36);
    d.thermal_shutdown = ov(30, 37);
    d.thermal_overload = ov(31, 38);
    d.reference_running = ov(32, 39);
    d.reference_measurement_failed = ov(33, 40);
    d.current_protector_enabled = ov(34, 41);
    d.operating_mode = ov(35, 42);
    d.fast_fade_time = ov(36, 43);
    d.min_fast_fade_time = ov(37, 44);
    d.extended_version_number = ov(38, 45);
    let t = &mut a.dt8_color;
    t.color_type = ov(44, 46);
    t.color_value_0 = ov(401_u16, 47);
    t.color_value_1 = ov(402_u16, 48);
    t.color_value_2 = ov(403_u16, 49);
    t.gear_features = ov(0x41, 69);
    t.rgbwaf_control = ov(0x80, 71);
    a.extended.fade_time_ms = ov(700_u32, 50);
    a.extended.version_number = ov(45, 51);
    let i = &mut a.memory_identity;
    i.last_memory_bank = ov(50, 52);
    i.gtin = ov(8_710_000_000_123_u64, 53);
    i.firmware_version_major = ov(51, 54);
    i.firmware_version_minor = ov(52, 55);
    i.identification_number = ov(9_876_543_210_u64, 56);
    i.hardware_version_major = ov(53, 57);
    i.hardware_version_minor = ov(54, 58);
    i.dali_101_version = ov(55, 59);
    i.dali_102_version = ov(56, 60);
    i.dali_103_version = ov(57, 61);
    i.logical_control_device_units = ov(58, 62);
    i.logical_control_gear_units = ov(59, 63);
    i.logical_control_gear_index = ov(60, 64);
    let p = &mut a.memory_profile;
    p.bank1_lock_byte = ov(85, 65);
    p.oem_gtin = ov(777_000_000_001_u64, 66);
    p.oem_identification_number = ov(888_000_000_002_u64, 67);
    a
}

const GOLDEN_FULL_ATTRIBUTES: &str = r#"{"common_102":{"device_type":{"last_read_ms":null,"last_write_confirmed_ms":2002,"source":"write_confirmed","value":2},"fade_rate":{"last_read_ms":1009,"last_write_confirmed_ms":null,"source":"readback","value":9},"fade_time_ms":{"last_read_ms":null,"last_write_confirmed_ms":2008,"source":"write_confirmed","value":1500},"light_source_type":{"last_read_ms":1073,"last_write_confirmed_ms":null,"source":"readback","value":255},"light_source_types":{"last_read_ms":1075,"last_write_confirmed_ms":null,"source":"readback","value":393982},"max_level":{"last_read_ms":1005,"last_write_confirmed_ms":null,"source":"readback","value":5},"min_level":{"last_read_ms":null,"last_write_confirmed_ms":2004,"source":"write_confirmed","value":4},"physical_minimum":{"last_read_ms":1003,"last_write_confirmed_ms":null,"source":"readback","value":3},"power_on_level":{"last_read_ms":null,"last_write_confirmed_ms":2006,"source":"write_confirmed","value":6},"system_failure_level":{"last_read_ms":1007,"last_write_confirmed_ms":null,"source":"readback","value":7},"version":{"last_read_ms":1001,"last_write_confirmed_ms":null,"source":"readback","value":1}},"dt6_led":{"current_protector_active":{"last_read_ms":null,"last_write_confirmed_ms":2036,"source":"write_confirmed","value":29},"current_protector_enabled":{"last_read_ms":1041,"last_write_confirmed_ms":null,"source":"readback","value":34},"dimming_curve":{"last_read_ms":null,"last_write_confirmed_ms":2028,"source":"write_confirmed","value":21},"extended_version_number":{"last_read_ms":1045,"last_write_confirmed_ms":null,"source":"readback","value":38},"failure_status":{"last_read_ms":1031,"last_write_confirmed_ms":null,"source":"readback","value":24},"fast_fade_time":{"last_read_ms":1043,"last_write_confirmed_ms":null,"source":"readback","value":36},"features":{"last_read_ms":null,"last_write_confirmed_ms":2030,"source":"write_confirmed","value":23},"gear_type":{"last_read_ms":1027,"last_write_confirmed_ms":null,"source":"readback","value":20},"load_decrease":{"last_read_ms":null,"last_write_confirmed_ms":2034,"source":"write_confirmed","value":27},"load_increase":{"last_read_ms":1035,"last_write_confirmed_ms":null,"source":"readback","value":28},"min_fast_fade_time":{"last_read_ms":null,"last_write_confirmed_ms":2044,"source":"write_confirmed","value":37},"open_circuit":{"last_read_ms":1033,"last_write_confirmed_ms":null,"source":"readback","value":26},"operating_mode":{"last_read_ms":null,"last_write_confirmed_ms":2042,"source":"write_confirmed","value":35},"possible_operating_mode":{"last_read_ms":1029,"last_write_confirmed_ms":null,"source":"readback","value":22},"reference_measurement_failed":{"last_read_ms":null,"last_write_confirmed_ms":2040,"source":"write_confirmed","value":33},"reference_running":{"last_read_ms":1039,"last_write_confirmed_ms":null,"source":"readback","value":32},"short_circuit":{"last_read_ms":null,"last_write_confirmed_ms":2032,"source":"write_confirmed","value":25},"thermal_overload":{"last_read_ms":null,"last_write_confirmed_ms":2038,"source":"write_confirmed","value":31},"thermal_shutdown":{"last_read_ms":1037,"last_write_confirmed_ms":null,"source":"readback","value":30}},"dt8_color":{"color_type":{"last_read_ms":null,"last_write_confirmed_ms":2046,"source":"write_confirmed","value":44},"color_value_0":{"last_read_ms":1047,"last_write_confirmed_ms":null,"source":"readback","value":401},"color_value_1":{"last_read_ms":null,"last_write_confirmed_ms":2048,"source":"write_confirmed","value":402},"color_value_2":{"last_read_ms":1049,"last_write_confirmed_ms":null,"source":"readback","value":403},"gear_features":{"last_read_ms":1069,"last_write_confirmed_ms":null,"source":"readback","value":65},"rgbwaf_control":{"last_read_ms":1071,"last_write_confirmed_ms":null,"source":"readback","value":128}},"extended":{"fade_time_ms":{"last_read_ms":null,"last_write_confirmed_ms":2050,"source":"write_confirmed","value":700},"version_number":{"last_read_ms":1051,"last_write_confirmed_ms":null,"source":"readback","value":45}},"groups":{"membership":{"last_read_ms":null,"last_write_confirmed_ms":2010,"source":"write_confirmed","value":42405}},"memory_identity":{"dali_101_version":{"last_read_ms":1059,"last_write_confirmed_ms":null,"source":"readback","value":55},"dali_102_version":{"last_read_ms":null,"last_write_confirmed_ms":2060,"source":"write_confirmed","value":56},"dali_103_version":{"last_read_ms":1061,"last_write_confirmed_ms":null,"source":"readback","value":57},"firmware_version_major":{"last_read_ms":null,"last_write_confirmed_ms":2054,"source":"write_confirmed","value":51},"firmware_version_minor":{"last_read_ms":1055,"last_write_confirmed_ms":null,"source":"readback","value":52},"gtin":{"last_read_ms":1053,"last_write_confirmed_ms":null,"source":"readback","value":8710000000123},"hardware_version_major":{"last_read_ms":1057,"last_write_confirmed_ms":null,"source":"readback","value":53},"hardware_version_minor":{"last_read_ms":null,"last_write_confirmed_ms":2058,"source":"write_confirmed","value":54},"identification_number":{"last_read_ms":null,"last_write_confirmed_ms":2056,"source":"write_confirmed","value":9876543210},"last_memory_bank":{"last_read_ms":null,"last_write_confirmed_ms":2052,"source":"write_confirmed","value":50},"logical_control_device_units":{"last_read_ms":null,"last_write_confirmed_ms":2062,"source":"write_confirmed","value":58},"logical_control_gear_index":{"last_read_ms":null,"last_write_confirmed_ms":2064,"source":"write_confirmed","value":60},"logical_control_gear_units":{"last_read_ms":1063,"last_write_confirmed_ms":null,"source":"readback","value":59}},"memory_profile":{"bank1_lock_byte":{"last_read_ms":1065,"last_write_confirmed_ms":null,"source":"readback","value":85},"oem_gtin":{"last_read_ms":null,"last_write_confirmed_ms":2066,"source":"write_confirmed","value":777000000001},"oem_identification_number":{"last_read_ms":1067,"last_write_confirmed_ms":null,"source":"readback","value":888000000002}},"scenes":{"scene_0":{"last_read_ms":1011,"last_write_confirmed_ms":null,"source":"readback","value":100},"scene_1":{"last_read_ms":null,"last_write_confirmed_ms":2012,"source":"write_confirmed","value":101},"scene_10":{"last_read_ms":1021,"last_write_confirmed_ms":null,"source":"readback","value":110},"scene_11":{"last_read_ms":null,"last_write_confirmed_ms":2022,"source":"write_confirmed","value":111},"scene_12":{"last_read_ms":1023,"last_write_confirmed_ms":null,"source":"readback","value":112},"scene_13":{"last_read_ms":null,"last_write_confirmed_ms":2024,"source":"write_confirmed","value":113},"scene_14":{"last_read_ms":1025,"last_write_confirmed_ms":null,"source":"readback","value":114},"scene_15":{"last_read_ms":null,"last_write_confirmed_ms":2026,"source":"write_confirmed","value":115},"scene_2":{"last_read_ms":1013,"last_write_confirmed_ms":null,"source":"readback","value":102},"scene_3":{"last_read_ms":null,"last_write_confirmed_ms":2014,"source":"write_confirmed","value":103},"scene_4":{"last_read_ms":1015,"last_write_confirmed_ms":null,"source":"readback","value":104},"scene_5":{"last_read_ms":null,"last_write_confirmed_ms":2016,"source":"write_confirmed","value":105},"scene_6":{"last_read_ms":1017,"last_write_confirmed_ms":null,"source":"readback","value":106},"scene_7":{"last_read_ms":null,"last_write_confirmed_ms":2018,"source":"write_confirmed","value":107},"scene_8":{"last_read_ms":1019,"last_write_confirmed_ms":null,"source":"readback","value":108},"scene_9":{"last_read_ms":null,"last_write_confirmed_ms":2020,"source":"write_confirmed","value":109}}}"#;

fn bank_ov(v: BankReading, at: u64) -> Option<ObservedValue<BankReading>> {
    Some(ObservedValue {
        value: v,
        source: AttributeSource::Readback,
        last_read_ms: Some(at),
        last_write_confirmed_ms: None,
    })
}

fn reading(value: u64) -> BankReading {
    BankReading { value: Some(value), ..Default::default() }
}

#[test]
fn the_three_bank_states_reach_the_wire_as_three() {
    let mut energy = MemoryEnergyAttributesView::default();
    energy.active.energy = bank_ov(reading(0), 1_000);
    energy.active.energy_scale = Some(ObservedValue {
        value: -3,
        source: AttributeSource::Readback,
        last_read_ms: Some(1_000),
        last_write_confirmed_ms: None,
    });
    energy.active.power = bank_ov(
        BankReading {
            temporarily_unavailable: true,
            tmask_since_ms: Some(500),
            ..Default::default()
        },
        1_000,
    );
    energy.apparent.energy = bank_ov(
        BankReading { not_implemented: true, ..Default::default() },
        1_000,
    );

    let json: serde_json::Value = serde_json::from_str(&attributes_json(
        &PhysicalDeviceAttributesView::default(),
        &energy,
        &Default::default(),
        &Default::default(),
        None,
    ))
    .expect("parse");
    let active = &json["memory_energy"]["active"];
    assert_eq!(active["energy"]["value"]["value"], 0, "a real zero is a value");
    assert_eq!(active["energy"]["value"]["not_implemented"], false);
    assert_eq!(active["energy_scale"]["value"], -3, "signed, raw, beside its value");
    assert_eq!(active["power"]["value"]["temporarily_unavailable"], true);
    assert_eq!(active["power"]["value"]["tmask_since_ms"], 500);
    assert_eq!(active["power"]["value"]["value"], serde_json::Value::Null);
    assert_eq!(json["memory_energy"]["apparent"]["energy"]["value"]["not_implemented"], true);
    assert!(
        json["memory_energy"].get("loadside").is_none(),
        "204 is optional in Part 252; an absent bank is omitted, not empty"
    );
    assert!(
        json.get("memory_diagnostics").is_none(),
        "and a section nothing was read for stays off the wire entirely"
    );
}

#[test]
fn a_condition_carries_its_counter_and_saturation_is_its_own_fact() {
    let mut diag = MemoryDiagnosticsAttributesView::default();
    diag.control_gear.bank_version = Some(ObservedValue {
        value: 1,
        source: AttributeSource::Readback,
        last_read_ms: Some(1),
        last_write_confirmed_ms: None,
    });
    diag.control_gear.thermal_derating.flag = bank_ov(reading(1), 1);
    diag.control_gear.thermal_derating.counter = bank_ov(
        BankReading { value: Some(0xFD), saturated: true, ..Default::default() },
        1,
    );

    let json: serde_json::Value = serde_json::from_str(&attributes_json(
        &PhysicalDeviceAttributesView::default(),
        &Default::default(),
        &diag,
        &Default::default(),
        None,
    ))
    .expect("parse");
    let cond = &json["memory_diagnostics"]["control_gear"]["thermal_derating"];
    assert_eq!(cond["flag"]["value"]["value"], 1);
    assert_eq!(cond["counter"]["value"]["value"], 0xFD);
    assert_eq!(cond["counter"]["value"]["saturated"], true);
}

#[test]
fn full_attributes_wire_shape_is_frozen() {
    let json = attributes_json(
        &full_attributes_view(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
    );
    assert_eq!(json, GOLDEN_FULL_ATTRIBUTES);
}

fn populated_energy() -> MemoryEnergyAttributesView {
    let mut e = MemoryEnergyAttributesView::default();
    e.active.bank_version = ov(1, 101);
    e.active.energy_scale = ov(-3_i8, 102);
    e.active.energy = ov(reading(123_456), 103);
    e.active.power_scale = ov(-1_i8, 104);
    e.active.power = ov(
        BankReading { temporarily_unavailable: true, tmask_since_ms: Some(900), ..Default::default() },
        105,
    );
    e.apparent.bank_version = ov(1, 106);
    e.apparent.energy = ov(BankReading { not_implemented: true, ..Default::default() }, 107);
    e
}

fn populate_gear(g: &mut GearDiagnosticsView) {
    g.bank_version = ov(2, 110);
    g.operating_time_s = ov(reading(3_600), 111);
    g.start_counter = ov(reading(42), 112);
    g.supply_voltage_decivolt = ov(reading(2_301), 113);
    g.supply_frequency_hz = ov(reading(50), 114);
    g.power_factor_centi = ov(reading(97), 115);
    g.overall_failure.flag = ov(reading(0), 116);
    g.undervoltage.flag = ov(reading(1), 117);
    g.undervoltage.counter =
        ov(BankReading { value: Some(0xFE), saturated: true, ..Default::default() }, 118);
    g.thermal_shutdown.counter = ov(reading(3), 119);
    g.temperature_offset60 = ov(reading(85), 120);
    g.output_current_percent = ov(reading(64), 121);
}

fn populate_source(s: &mut SourceDiagnosticsView) {
    s.bank_version = ov(1, 122);
    s.start_counter_resettable = ov(reading(7), 123);
    s.on_time_s = ov(reading(86_400), 124);
    s.voltage_decivolt = ov(reading(480), 125);
    s.current_milliamp = ov(BankReading { tmask_since_ms: Some(1), ..Default::default() }, 126);
    s.short_circuit.flag = ov(reading(0), 127);
    s.open_circuit.counter = ov(reading(2), 128);
    s.temperature_offset60 = ov(reading(70), 129);
}

fn populate_maintenance(m: &mut LuminaireMaintenanceView) {
    m.bank_version = ov(1, 130);
    m.rated_life_kilohours = ov(reading(50), 131);
    m.rated_starts_hundreds = ov(reading(200), 132);
}

fn populated_diagnostics() -> MemoryDiagnosticsAttributesView {
    let mut d = MemoryDiagnosticsAttributesView::default();
    populate_gear(&mut d.control_gear);
    populate_source(&mut d.light_source);
    populate_maintenance(&mut d.luminaire);
    d
}

fn lv(raw: u32, value: Option<u32>, part209_implemented: bool) -> LuminaireValue {
    LuminaireValue { raw, value, part209_implemented }
}

fn populated_luminaire() -> MemoryLuminaireAttributesView {
    let text32 = |s: &str| FixedText32::try_from(s).expect("fits 32 bytes");
    MemoryLuminaireAttributesView {
        content_format_id: ov(3_u16, 140),
        year: ov(lv(24, Some(2_024), true), 141),
        week: ov(lv(0xFF, None, true), 142),
        nominal_input_power_w: ov(lv(36, Some(36), false), 143),
        cri: ov(lv(92, Some(92), true), 144),
        cct_kelvin: ov(lv(3_000, Some(3_000), true), 145),
        luminaire_colour: ov(text32("warm \"white\" \\ 3000K"), 146),
        luminaire_identification: ov(
            FixedText64::try_from("L\u{fc}mi-\u{c4}\tline\nend").expect("fits 64 bytes"),
            147,
        ),
        oem_name: ov(text32("ACME"), 148),
        free_use: ov(text32(""), 149),
        lamp_current_ma: ov(lv(700, Some(700), true), 150),
        ..Default::default()
    }
}

fn populated_bus_unit() -> MemoryBusUnitAttributesView {
    MemoryBusUnitAttributesView { configuration: ov(10, 151), implemented_parts: ov(0x05, 152) }
}

const GOLDEN_MEMORY_SECTIONS: &str = r#"{"memory_diagnostics":{"control_gear":{"bank_version":{"last_read_ms":null,"last_write_confirmed_ms":2110,"source":"write_confirmed","value":2},"operating_time_s":{"last_read_ms":1111,"last_write_confirmed_ms":null,"source":"readback","value":{"value":3600,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"output_current_percent":{"last_read_ms":1121,"last_write_confirmed_ms":null,"source":"readback","value":{"value":64,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"overall_failure":{"flag":{"last_read_ms":null,"last_write_confirmed_ms":2116,"source":"write_confirmed","value":{"value":0,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}},"power_factor_centi":{"last_read_ms":1115,"last_write_confirmed_ms":null,"source":"readback","value":{"value":97,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"start_counter":{"last_read_ms":null,"last_write_confirmed_ms":2112,"source":"write_confirmed","value":{"value":42,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"supply_frequency_hz":{"last_read_ms":null,"last_write_confirmed_ms":2114,"source":"write_confirmed","value":{"value":50,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"supply_voltage_decivolt":{"last_read_ms":1113,"last_write_confirmed_ms":null,"source":"readback","value":{"value":2301,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"temperature_offset60":{"last_read_ms":null,"last_write_confirmed_ms":2120,"source":"write_confirmed","value":{"value":85,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"thermal_shutdown":{"counter":{"last_read_ms":1119,"last_write_confirmed_ms":null,"source":"readback","value":{"value":3,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}},"undervoltage":{"counter":{"last_read_ms":null,"last_write_confirmed_ms":2118,"source":"write_confirmed","value":{"value":254,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":true}},"flag":{"last_read_ms":1117,"last_write_confirmed_ms":null,"source":"readback","value":{"value":1,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}}},"light_source":{"bank_version":{"last_read_ms":null,"last_write_confirmed_ms":2122,"source":"write_confirmed","value":1},"current_milliamp":{"last_read_ms":null,"last_write_confirmed_ms":2126,"source":"write_confirmed","value":{"value":null,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":1,"saturated":false}},"on_time_s":{"last_read_ms":null,"last_write_confirmed_ms":2124,"source":"write_confirmed","value":{"value":86400,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"open_circuit":{"counter":{"last_read_ms":null,"last_write_confirmed_ms":2128,"source":"write_confirmed","value":{"value":2,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}},"short_circuit":{"flag":{"last_read_ms":1127,"last_write_confirmed_ms":null,"source":"readback","value":{"value":0,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}},"start_counter_resettable":{"last_read_ms":1123,"last_write_confirmed_ms":null,"source":"readback","value":{"value":7,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"temperature_offset60":{"last_read_ms":1129,"last_write_confirmed_ms":null,"source":"readback","value":{"value":70,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"voltage_decivolt":{"last_read_ms":1125,"last_write_confirmed_ms":null,"source":"readback","value":{"value":480,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}},"luminaire":{"bank_version":{"last_read_ms":null,"last_write_confirmed_ms":2130,"source":"write_confirmed","value":1},"rated_life_kilohours":{"last_read_ms":1131,"last_write_confirmed_ms":null,"source":"readback","value":{"value":50,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"rated_starts_hundreds":{"last_read_ms":null,"last_write_confirmed_ms":2132,"source":"write_confirmed","value":{"value":200,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}}},"memory_energy":{"active":{"bank_version":{"last_read_ms":1101,"last_write_confirmed_ms":null,"source":"readback","value":1},"energy":{"last_read_ms":1103,"last_write_confirmed_ms":null,"source":"readback","value":{"value":123456,"not_implemented":false,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}},"energy_scale":{"last_read_ms":null,"last_write_confirmed_ms":2102,"source":"write_confirmed","value":-3},"power":{"last_read_ms":1105,"last_write_confirmed_ms":null,"source":"readback","value":{"value":null,"not_implemented":false,"temporarily_unavailable":true,"tmask_since_ms":900,"saturated":false}},"power_scale":{"last_read_ms":null,"last_write_confirmed_ms":2104,"source":"write_confirmed","value":-1}},"apparent":{"bank_version":{"last_read_ms":null,"last_write_confirmed_ms":2106,"source":"write_confirmed","value":1},"energy":{"last_read_ms":1107,"last_write_confirmed_ms":null,"source":"readback","value":{"value":null,"not_implemented":true,"temporarily_unavailable":false,"tmask_since_ms":null,"saturated":false}}}},"memory_bus_unit":{"configuration":{"last_read_ms":1151,"last_write_confirmed_ms":null,"source":"readback","value":{"raw":10,"class":"Part 202 emergency control gear","emergency_type":"B"}},"implemented_parts":{"last_read_ms":null,"last_write_confirmed_ms":2152,"source":"write_confirmed","value":{"raw":5,"parts":[150,152]}}},"memory_luminaire":{"cct_kelvin":{"last_read_ms":1145,"last_write_confirmed_ms":null,"source":"readback","value":{"raw":3000,"value":3000,"part209_implemented":true}},"content_format_id":{"last_read_ms":null,"last_write_confirmed_ms":2140,"source":"write_confirmed","value":3},"cri":{"last_read_ms":null,"last_write_confirmed_ms":2144,"source":"write_confirmed","value":{"raw":92,"value":92,"part209_implemented":true}},"free_use":{"last_read_ms":1149,"last_write_confirmed_ms":null,"source":"readback","value":""},"lamp_current_ma":{"last_read_ms":null,"last_write_confirmed_ms":2150,"source":"write_confirmed","value":{"raw":700,"value":700,"part209_implemented":true}},"luminaire_colour":{"last_read_ms":null,"last_write_confirmed_ms":2146,"source":"write_confirmed","value":"warm \"white\" \\ 3000K"},"luminaire_identification":{"last_read_ms":1147,"last_write_confirmed_ms":null,"source":"readback","value":"Lümi-Ä\tline\nend"},"nominal_input_power_w":{"last_read_ms":1143,"last_write_confirmed_ms":null,"source":"readback","value":{"raw":36,"value":36,"part209_implemented":false}},"oem_name":{"last_read_ms":null,"last_write_confirmed_ms":2148,"source":"write_confirmed","value":"ACME"},"week":{"last_read_ms":null,"last_write_confirmed_ms":2142,"source":"write_confirmed","value":{"raw":255,"value":null,"part209_implemented":true}},"year":{"last_read_ms":1141,"last_write_confirmed_ms":null,"source":"readback","value":{"raw":24,"value":2024,"part209_implemented":true}}}}"#;

#[test]
fn populated_memory_sections_wire_shape_is_frozen() {
    let json = attributes_json(
        &PhysicalDeviceAttributesView::default(),
        &populated_energy(),
        &populated_diagnostics(),
        &populated_bus_unit(),
        Some(&populated_luminaire()),
    );
    assert_eq!(json, GOLDEN_MEMORY_SECTIONS);
}

fn serializes_a_non_empty_object(view: &AttributeSectionView) -> bool {
    AttributeSectionDto::from_view(view).is_some_and(|dto| {
        let json = serde_json::to_value(&dto).expect("a present section serializes");
        json.as_object().is_some_and(|fields| !fields.is_empty())
    })
}

#[test]
fn every_section_is_present_exactly_when_it_has_something_to_write() {
    let empty = every_section(
        &PhysicalDeviceAttributesView::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
    );
    let populated = every_section(
        &full_attributes_view(),
        &populated_energy(),
        &populated_diagnostics(),
        &populated_bus_unit(),
        Some(&populated_luminaire()),
    );
    for view in &empty {
        let kind = view.kind().wire_name();
        assert!(AttributeSectionDto::from_view(view).is_none(), "empty {kind} is absent");
    }
    for view in &populated {
        let kind = view.kind().wire_name();
        assert!(serializes_a_non_empty_object(view), "populated {kind} writes its fields");
    }
}

#[test]
fn one_field_makes_a_hand_written_section_present() {
    let bus_unit = MemoryBusUnitAttributesView { implemented_parts: observed(1), ..Default::default() };
    let mut energy = MemoryEnergyAttributesView::default();
    energy.loadside.power = ov(reading(1), 1);
    let mut source = MemoryDiagnosticsAttributesView::default();
    source.light_source.bank_version = observed(1);
    let mut maintenance = MemoryDiagnosticsAttributesView::default();
    maintenance.luminaire.rated_starts_hundreds = ov(reading(1), 1);
    let mut scenes = ScenesAttributesView::default();
    scenes.levels[15] = observed(1);
    for view in [
        AttributeSectionView::MemoryBusUnit(Box::new(bus_unit)),
        AttributeSectionView::MemoryEnergy(Box::new(energy)),
        AttributeSectionView::MemoryDiagnostics(Box::new(source)),
        AttributeSectionView::MemoryDiagnostics(Box::new(maintenance)),
        AttributeSectionView::Scenes(Box::new(scenes)),
    ] {
        assert!(serializes_a_non_empty_object(&view), "{}", view.kind().wire_name());
    }
}

#[test]
fn a_diagnostics_bank_without_its_version_stays_off_the_wire() {
    let mut diag = MemoryDiagnosticsAttributesView::default();
    diag.control_gear.operating_time_s = ov(reading(3_600), 1);
    let view = AttributeSectionView::MemoryDiagnostics(Box::new(diag));
    assert!(AttributeSectionDto::from_view(&view).is_none());
}

#[test]
fn populated_attributes_emit_every_section() {
    let mut attrs = PhysicalDeviceAttributesView::default();
    attrs.common102.min_level = observed(10);
    attrs.groups.membership = Some(ObservedValue {
        value: 0x0003_u16,
        source: AttributeSource::WriteConfirmed,
        last_read_ms: None,
        last_write_confirmed_ms: Some(2_000),
    });
    attrs.scenes.levels[5] = observed(128);
    attrs.dt6_led.gear_type = observed(4);
    attrs.dt8_color.color_type = observed(32);
    attrs.extended.version_number = observed(2);
    attrs.memory_identity.firmware_version_major = observed(1);
    attrs.memory_profile.bank1_lock_byte = observed(0x55);

    let json: serde_json::Value = serde_json::from_str(&attributes_json(
        &attrs,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
    )).expect("parse");
    let obj = json.as_object().expect("attributes object");
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "common_102",
            "dt6_led",
            "dt8_color",
            "extended",
            "groups",
            "memory_identity",
            "memory_profile",
            "scenes",
        ],
        "sections present, BTreeMap key order"
    );
    assert_eq!(json["common_102"]["min_level"]["value"], 10);
    assert_eq!(json["common_102"]["min_level"]["source"], "readback");
    assert_eq!(json["groups"]["membership"]["source"], "write_confirmed");
    assert_eq!(json["scenes"]["scene_5"]["value"], 128);
}
