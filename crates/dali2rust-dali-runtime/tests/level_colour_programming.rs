use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::write_short_attributes;
use dali2rust_domain::dali::devices::dt8_color::{
    COLOUR_TYPE_BYTE_RGBWAF, COLOUR_TYPE_BYTE_TC, COLOUR_TYPE_BYTE_XY,
};
use dali2rust_gear_model::gear::ColourSetting;
use dali2rust_gear_model::{ColorMode, Gear, GearSpec};
use dali2rust_test_support::StoppedClock;

const SHORT: u8 = 0;
const LEVEL: u8 = 140;
const LIT_LEVEL: u8 = 90;
const LIT_MIREK: u16 = 250;
const STRAY_MIREK: u16 = 222;
const STORED_MIREK: u16 = 300;
const TC_RANGE: (u16, u16) = (153, 400);
const STORED_RGBWAF: ([u8; 6], u8) = ([10, 20, 30, 0, 0, 0], 0x80);

type Bus = Arc<Mutex<SimDaliTransport>>;

#[derive(Clone, Copy, Debug)]
enum Level {
    PowerOn,
    SystemFailure,
}

const LEVELS: [Level; 2] = [Level::PowerOn, Level::SystemFailure];

fn lit_gear_bus(caps: (bool, bool, bool)) -> (DaliController<SimDaliTransport>, Bus) {
    let spec = GearSpec::dt8(Some(SHORT), 0x00_5678, caps, TC_RANGE);
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

fn stored_colour(gear: &mut Gear, level: Level) -> &mut ColourSetting {
    match level {
        Level::PowerOn => &mut gear.power_on.colour,
        Level::SystemFailure => &mut gear.system_failure.colour,
    }
}

fn stored_level(gear: &Gear, level: Level) -> u8 {
    match level {
        Level::PowerOn => gear.power_on.level,
        Level::SystemFailure => gear.system_failure.level,
    }
}

fn stage_stray_tc(bus: &Bus) {
    with_gear(bus, |gear| {
        gear.pending_mode = ColorMode::Cct;
        gear.pending_ct = STRAY_MIREK;
    });
}

fn write(controller: &mut DaliController<SimDaliTransport>, level: Level) {
    let (power_on, failure) = match level {
        Level::PowerOn => (Some(LEVEL), None),
        Level::SystemFailure => (None, Some(LEVEL)),
    };
    let execution = write_short_attributes(
        controller, SHORT, None, None, power_on, failure, None, (None, None), (None, None), None, None, Default::default(),
    );
    assert_eq!(execution.error, None, "{level:?}");
}

fn assert_written_and_light_untouched(bus: &Bus, level: Level) {
    with_gear(bus, |gear| {
        assert_eq!(stored_level(gear, level), LEVEL);
        assert_eq!(gear.pending_mode, ColorMode::None, "209 §11.2.2: the store consumes the temporaries");
        assert_eq!((gear.level, gear.color_ct), (LIT_LEVEL, LIT_MIREK), "Table 6: no visible effect");
    });
}

// IEC 62386-209 §9.2.2, Table 1, §9.3.2, Table 2, §11.2.2
#[test]
fn writing_a_level_keeps_the_stored_colour_over_a_charged_temporary() {
    for level in LEVELS {
        let (mut controller, bus) = lit_gear_bus((true, false, false));
        let stored = ColourSetting { colour_type: COLOUR_TYPE_BYTE_TC, tc: STORED_MIREK, ..ColourSetting::mask() };
        with_gear(&bus, |gear| *stored_colour(gear, level) = stored);
        stage_stray_tc(&bus);

        write(&mut controller, level);

        assert_eq!(with_gear(&bus, |gear| *stored_colour(gear, level)), stored, "{level:?} colour moved");
        assert_written_and_light_untouched(&bus, level);
    }
}

// IEC 62386-209 §9.12.7
#[test]
fn a_stored_colour_of_another_type_is_kept_whole() {
    for level in LEVELS {
        let (mut controller, bus) = lit_gear_bus((true, false, true));
        let stored = ColourSetting {
            colour_type: COLOUR_TYPE_BYTE_RGBWAF,
            levels: STORED_RGBWAF.0,
            control: STORED_RGBWAF.1,
            ..ColourSetting::mask()
        };
        with_gear(&bus, |gear| *stored_colour(gear, level) = stored);
        stage_stray_tc(&bus);

        write(&mut controller, level);

        assert_eq!(with_gear(&bus, |gear| *stored_colour(gear, level)), stored, "{level:?} colour moved");
        assert_written_and_light_untouched(&bus, level);
    }
}

fn mask_of(colour_type: u8) -> ColourSetting {
    ColourSetting { colour_type, ..ColourSetting::mask() }
}

// IEC 62386-209 §9.2.1, §9.3.1, Table 8
#[test]
fn a_stored_mask_colour_comes_back_as_mask_values_of_the_temporary_type() {
    let strays: [(ColorMode, (bool, bool, bool), u8); 3] = [
        (ColorMode::Cct, (true, false, false), COLOUR_TYPE_BYTE_TC),
        (ColorMode::Xy, (false, true, false), COLOUR_TYPE_BYTE_XY),
        (ColorMode::Rgb, (false, false, true), COLOUR_TYPE_BYTE_RGBWAF),
    ];
    for (mode, caps, colour_type) in strays {
        let (mut controller, bus) = lit_gear_bus(caps);
        with_gear(&bus, |gear| {
            gear.power_on.colour = ColourSetting::mask();
            gear.pending_mode = mode;
            (gear.pending_x, gear.pending_y, gear.pending_ct) = (1000, 2000, STRAY_MIREK);
            gear.rgbwaf.pending_levels = [7; 6];
            gear.rgbwaf.pending_control = 0x80;
        });

        write(&mut controller, Level::PowerOn);

        let kept = with_gear(&bus, |gear| gear.power_on.colour);
        assert_eq!(kept, mask_of(colour_type), "{mode:?}: the gear still changes no colour at power on");
        assert_written_and_light_untouched(&bus, Level::PowerOn);
    }
}
