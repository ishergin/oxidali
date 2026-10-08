use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_contracts::msg::{
    ColorMode as SetpointColorMode, ColorValue, DaliSceneTargetState, PowerState,
    SceneProgramAction,
};
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::scene::program_scene_row;
use dali2rust_domain::dali::devices::dt8_color::{COLOUR_TYPE_BYTE_MASK, COLOUR_TYPE_BYTE_TC};
use dali2rust_gear_model::gear::ColourSetting;
use dali2rust_gear_model::{ColorMode, Gear, GearSpec};
use dali2rust_test_support::StoppedClock;

const SHORT: u8 = 0;
const SCENE: u8 = 12;
const ROW_LEVEL: u8 = 200;
const LIT_LEVEL: u8 = 90;
const STRAY_MIREK: u16 = 222;
const OLD_SCENE_MIREK: u16 = 300;
const LIT_MIREK: u16 = 250;
const TC_RANGE: (u16, u16) = (153, 400);
const ROW_KELVIN: u16 = 3000;
const ROW_MIREK: u16 = 333;

type Bus = Arc<Mutex<SimDaliTransport>>;

fn tc_gear_bus() -> (DaliController<SimDaliTransport>, Bus) {
    let spec = GearSpec::dt8(Some(SHORT), 0x00_1234, (true, false, false), TC_RANGE);
    let bus = Arc::new(Mutex::new(SimDaliTransport::new(vec![spec])));
    let controller = DaliController::new(Arc::clone(&bus), Box::new(StoppedClock::at(0)));
    with_gear(&bus, |gear| {
        gear.level = LIT_LEVEL;
        gear.color_mode = ColorMode::Cct;
        gear.color_ct = LIT_MIREK;
    });
    (controller, bus)
}

fn with_gear<T>(bus: &Bus, f: impl FnOnce(&mut Gear) -> T) -> T {
    f(&mut bus.lock().expect("sim lock").fleet_mut().gears_mut()[0])
}

fn level_only_row(level: u8) -> DaliSceneTargetState {
    DaliSceneTargetState {
        power: Some(PowerState::On),
        level: Some(level),
        color: None,
    }
}

fn program(controller: &mut DaliController<SimDaliTransport>, row: &DaliSceneTargetState) {
    let readback = program_scene_row(controller, SHORT, SCENE, SceneProgramAction::Update, Some(row))
        .expect("scene row programs");
    assert_eq!(readback, Some(row.level.expect("row level")));
}

fn assert_level_only_scene_and_light_untouched(bus: &Bus) {
    with_gear(bus, |gear| {
        assert_eq!(gear.scenes[usize::from(SCENE)], ROW_LEVEL);
        assert_eq!(
            gear.scene_colours[usize::from(SCENE)].colour_type,
            COLOUR_TYPE_BYTE_MASK,
            "209 Table 4: a row without colour must leave the scene colour MASK, got {:?}",
            gear.scene_colours[usize::from(SCENE)]
        );
        assert_eq!(gear.pending_mode, ColorMode::None, "209 §11.2.2: STORE consumes the temporaries");
        assert_eq!((gear.level, gear.color_ct), (LIT_LEVEL, LIT_MIREK), "Table 6: no visible effect");
    });
}

#[test]
fn a_row_without_colour_does_not_store_a_stray_temporary_colour() {
    let (mut controller, bus) = tc_gear_bus();
    with_gear(&bus, |gear| {
        gear.pending_mode = ColorMode::Cct;
        gear.pending_ct = STRAY_MIREK;
    });

    program(&mut controller, &level_only_row(ROW_LEVEL));

    assert_level_only_scene_and_light_untouched(&bus);
}

#[test]
fn a_row_without_colour_removes_the_colour_the_scene_held_before() {
    let (mut controller, bus) = tc_gear_bus();
    with_gear(&bus, |gear| {
        gear.scene_colours[usize::from(SCENE)] = ColourSetting {
            colour_type: COLOUR_TYPE_BYTE_TC,
            tc: OLD_SCENE_MIREK,
            ..ColourSetting::mask()
        };
    });

    program(&mut controller, &level_only_row(ROW_LEVEL));

    assert_level_only_scene_and_light_untouched(&bus);
}

#[test]
fn a_row_with_colour_still_stores_its_own_colour_over_a_stray_temporary() {
    let (mut controller, bus) = tc_gear_bus();
    with_gear(&bus, |gear| {
        gear.pending_mode = ColorMode::Cct;
        gear.pending_ct = STRAY_MIREK;
    });
    let row = DaliSceneTargetState {
        color: Some(ColorValue {
            mode: SetpointColorMode::Cct,
            color_temperature_kelvin: ROW_KELVIN,
            ..ColorValue::default()
        }),
        ..level_only_row(ROW_LEVEL)
    };

    program(&mut controller, &row);

    with_gear(&bus, |gear| {
        let stored = gear.scene_colours[usize::from(SCENE)];
        assert_eq!((stored.colour_type, stored.tc), (COLOUR_TYPE_BYTE_TC, ROW_MIREK));
        assert_eq!(gear.scenes[usize::from(SCENE)], ROW_LEVEL);
    });
}
