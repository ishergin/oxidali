use core::sync::atomic::{AtomicU8, Ordering};

use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_TC;
use dali2rust_domain::dali::pres::describe::{describe_forward16, DescribeContext};
use dali2rust_gear_model::{Gear, GearFleet, SCENE_COUNT};
use esp_idf_svc::sys::esp_timer_get_time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off = 0,
    Change = 1,
    Frame = 2,
    Trace = 3,
}

impl Level {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "off" => Some(Level::Off),
            "change" => Some(Level::Change),
            "frame" => Some(Level::Frame),
            "trace" => Some(Level::Trace),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Change => "change",
            Level::Frame => "frame",
            Level::Trace => "trace",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Change as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Off,
        1 => Level::Change,
        2 => Level::Frame,
        _ => Level::Trace,
    }
}

pub fn enabled(at: Level) -> bool {
    level() >= at
}

pub fn now_us() -> i64 {
    // SAFETY: a plain read of the IDF timer; no state, callable from any task.
    unsafe { esp_timer_get_time() }
}

pub fn note(args: core::fmt::Arguments<'_>) {
    println!("# {args}");
}

#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => { $crate::logsink::note(format_args!($($arg)*)) };
}

pub fn error(args: core::fmt::Arguments<'_>) {
    println!("E {} {args}", now_us());
}

#[macro_export]
macro_rules! logerr {
    ($($arg:tt)*) => { $crate::logsink::error(format_args!($($arg)*)) };
}

pub fn forward(frame: u16, enabled_device_type: Option<u8>) {
    if !enabled(Level::Frame) {
        return;
    }
    let address = (frame >> 8) as u8;
    let command = frame as u8;
    let d = describe_forward16(address, command, DescribeContext { enabled_device_type });
    let target = d
        .target
        .map(|t| format!("{t:?}"))
        .unwrap_or_else(|| "-".into());
    match d.detail {
        Some(detail) => println!(
            "F {} {address:02x} {command:02x} {target} {} {detail}",
            now_us(),
            d.name
        ),
        None => println!(
            "F {} {address:02x} {command:02x} {target} {}",
            now_us(),
            d.name
        ),
    }
}

pub fn backward(byte: u8, answered: usize, idle_ticks_at_submit: u32) {
    if !enabled(Level::Frame) {
        return;
    }
    println!(
        "B {} {byte:02x} n={answered} t={idle_ticks_at_submit}",
        now_us()
    );
}

pub fn collision(answered: usize) {
    if !enabled(Level::Frame) {
        return;
    }
    println!("B {} -- n={answered} collision", now_us());
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GearDigest {
    short: Option<u8>,
    random: u32,
    level: u8,
    groups: u16,
    scenes: [u8; SCENE_COUNT],
    scene_tc: [u16; SCENE_COUNT],
    color_ct: u16,
    tc_limits: (u16, u16),
    min_level: u8,
    max_level: u8,
    enabled: bool,
}

impl GearDigest {
    pub fn of(gear: &Gear) -> Self {
        let mut scene_tc = [0xFFFFu16; SCENE_COUNT];
        for (slot, colour) in scene_tc.iter_mut().zip(gear.scene_colours.iter()) {
            if colour.colour_type == COLOUR_TYPE_BYTE_TC {
                *slot = colour.tc;
            }
        }
        Self {
            short: gear.spec.short_address,
            random: gear.spec.random_address,
            level: gear.level,
            groups: gear.groups,
            scenes: gear.scenes,
            scene_tc,
            color_ct: gear.color_ct,
            tc_limits: (gear.tc_coolest_limit, gear.tc_warmest_limit),
            min_level: gear.min_level,
            max_level: gear.max_level,
            enabled: gear.enabled,
        }
    }
}

pub fn snapshot(fleet: &GearFleet, into: &mut Vec<GearDigest>) {
    into.clear();
    into.extend(fleet.gears().iter().map(GearDigest::of));
}

pub fn log_changes(before: &[GearDigest], fleet: &GearFleet) {
    if !enabled(Level::Change) {
        return;
    }
    for (index, gear) in fleet.gears().iter().enumerate() {
        let Some(was) = before.get(index) else {
            continue;
        };
        let now = GearDigest::of(gear);
        if *was == now {
            continue;
        }
        report(index, was, &now);
    }
}

fn name_of(index: usize, digest: &GearDigest) -> String {
    match digest.short {
        Some(short) => format!("A{short:02}"),
        None => format!("U{index:02}"),
    }
}

fn report(index: usize, was: &GearDigest, now: &GearDigest) {
    let at = now_us();
    let who = name_of(index, now);
    if was.short != now.short {
        println!(
            "C {at} {who} short_address {} -> {}",
            opt(was.short),
            opt(now.short)
        );
    }
    if was.level != now.level {
        println!("C {at} {who} level {} -> {}", was.level, now.level);
    }
    if was.groups != now.groups {
        println!(
            "C {at} {who} groups {:#06x} -> {:#06x}",
            was.groups, now.groups
        );
    }
    if was.color_ct != now.color_ct {
        println!(
            "C {at} {who} colour_mirek {} -> {}",
            was.color_ct, now.color_ct
        );
    }
    if was.random != now.random {
        println!(
            "C {at} {who} random_address {:#08x} -> {:#08x}",
            was.random, now.random
        );
    }
    if was.min_level != now.min_level || was.max_level != now.max_level {
        println!(
            "C {at} {who} level_range {}..{} -> {}..{}",
            was.min_level, was.max_level, now.min_level, now.max_level
        );
    }
    if was.tc_limits != now.tc_limits {
        println!(
            "C {at} {who} tc_limits {}..{} -> {}..{}",
            was.tc_limits.0, was.tc_limits.1, now.tc_limits.0, now.tc_limits.1
        );
    }
    if was.enabled != now.enabled {
        println!("C {at} {who} enabled {} -> {}", was.enabled, now.enabled);
    }
    report_scenes(at, &who, was, now);
}

fn report_scenes(at: i64, who: &str, was: &GearDigest, now: &GearDigest) {
    for scene in 0..SCENE_COUNT {
        let (old, new) = (was.scenes[scene], now.scenes[scene]);
        if old != new {
            println!("C {at} {who} scene[{scene}] {} -> {}", slot(old), slot(new));
        }
        let (old_tc, new_tc) = (was.scene_tc[scene], now.scene_tc[scene]);
        if old_tc != new_tc {
            println!(
                "C {at} {who} scene[{scene}] mirek {} -> {}",
                wide_slot(old_tc),
                wide_slot(new_tc)
            );
        }
    }
}

fn opt(value: Option<u8>) -> String {
    value.map_or_else(|| "-".to_string(), |v| v.to_string())
}

fn slot(value: u8) -> String {
    if value == 0xFF {
        "unset".to_string()
    } else {
        value.to_string()
    }
}

fn wide_slot(value: u16) -> String {
    if value == 0xFFFF {
        "unset".to_string()
    } else {
        value.to_string()
    }
}
