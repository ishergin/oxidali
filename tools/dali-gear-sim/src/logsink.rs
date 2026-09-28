use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::io::Write;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::OnceLock;

use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_TC;
use dali2rust_domain::dali::pres::describe::{describe_forward16, DescribeContext};
use dali2rust_gear_model::{ColorMode, Gear, GearFleet, SCENE_COUNT};
use esp_idf_svc::sys::{esp, esp_timer_get_time, uart_driver_install, uart_port_t};

use crate::spawn;

const QUEUE_LINES: usize = 512;
const WRITER_STACK: usize = 6144;
const WRITER_PRIORITY: u8 = 4;
const CONSOLE_UART: uart_port_t = 0;
const UART_RX_BUFFER: i32 = 1024;
const UART_TX_BUFFER: i32 = 8192;

unsafe extern "C" {
    fn uart_vfs_dev_use_driver(uart_num: core::ffi::c_int);
}

static SINK: OnceLock<SyncSender<String>> = OnceLock::new();
static DROPPED: AtomicU32 = AtomicU32::new(0);
static LEVEL: AtomicU8 = AtomicU8::new(Level::Change as u8);

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

pub fn start() {
    let (lines, queue) = sync_channel::<String>(QUEUE_LINES);
    let (ready, installed) = sync_channel::<()>(1);
    spawn::on_app_core(c"gear-log", WRITER_STACK, WRITER_PRIORITY, move || {
        install_console_driver();
        let _ = ready.send(());
        write_lines(queue);
    });
    let _ = installed.recv();
    let _ = SINK.set(lines);
}

fn install_console_driver() {
    // SAFETY: the console UART gets its driver once, before anything reads or writes through it.
    esp!(unsafe {
        uart_driver_install(
            CONSOLE_UART,
            UART_RX_BUFFER,
            UART_TX_BUFFER,
            0,
            core::ptr::null_mut(),
            0,
        )
    })
    .expect("console UART driver");
    // SAFETY: the driver for this port is installed just above.
    unsafe { uart_vfs_dev_use_driver(CONSOLE_UART as core::ffi::c_int) };
}

fn write_lines(queue: Receiver<String>) {
    let stdout = std::io::stdout();
    for mut line in queue {
        line.push('\n');
        let _ = stdout.lock().write_all(line.as_bytes());
    }
}

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

pub fn dropped() -> u32 {
    DROPPED.load(Ordering::Relaxed)
}

pub fn now_us() -> i64 {
    // SAFETY: a plain read of the IDF timer; no state, callable from any task.
    unsafe { esp_timer_get_time() }
}

pub fn note(args: core::fmt::Arguments<'_>) {
    if let Some(sink) = SINK.get() {
        let _ = sink.send(format!("# {args}"));
    }
}

#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => { $crate::logsink::note(format_args!($($arg)*)) };
}

fn event(line: String) {
    let Some(sink) = SINK.get() else {
        return;
    };
    if let Err(TrySendError::Full(_)) = sink.try_send(line) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn error(args: core::fmt::Arguments<'_>) {
    event(format!("E {} {args}", now_us()));
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
    let detail = d.detail.map(|detail| format!(" {detail}")).unwrap_or_default();
    event(format!(
        "F {} {address:02x} {command:02x} {target} {}{detail}",
        now_us(),
        d.name
    ));
}

pub fn backward(byte: u8, answered: usize) {
    if enabled(Level::Frame) {
        event(format!("B {} {byte:02x} n={answered}", now_us()));
    }
}

pub fn collision(answered: usize) {
    if enabled(Level::Frame) {
        event(format!("B {} -- n={answered} collision", now_us()));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GearDigest {
    short: Option<u8>,
    random: u32,
    level: u8,
    groups: u16,
    scenes: [u8; SCENE_COUNT],
    scene_tc: [u16; SCENE_COUNT],
    colour_mode: ColorMode,
    color_ct: u16,
    color_xy: (u16, u16),
    rgbwaf: [u8; 6],
    tc_limits: (u16, u16),
    min_level: u8,
    max_level: u8,
    identifying: bool,
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
            colour_mode: gear.color_mode,
            color_ct: gear.color_ct,
            color_xy: (gear.color_x, gear.color_y),
            rgbwaf: gear.rgbwaf.levels,
            tc_limits: (gear.tc_coolest_limit, gear.tc_warmest_limit),
            min_level: gear.min_level,
            max_level: gear.max_level,
            identifying: gear.identifying,
            enabled: gear.enabled,
        }
    }
}

pub fn snapshot(fleet: &GearFleet, into: &mut Vec<GearDigest>) {
    into.clear();
    into.extend(fleet.gears().iter().map(GearDigest::of));
}

pub fn log_changes(before: &[GearDigest], after: &[GearDigest]) {
    if !enabled(Level::Change) {
        return;
    }
    for (index, (was, now)) in before.iter().zip(after.iter()).enumerate() {
        if was != now {
            report(index, was, now);
        }
    }
}

fn report(index: usize, was: &GearDigest, now: &GearDigest) {
    let at = now_us();
    let who = match now.short {
        Some(short) => format!("A{short:02}"),
        None => format!("U{index:02}"),
    };
    let line = |what: String| event(format!("C {at} {who} {what}"));
    report_identity(was, now, &line);
    report_output(was, now, &line);
    report_colour(was, now, &line);
    report_limits(was, now, &line);
    report_scenes(was, now, &line);
}

fn report_identity(was: &GearDigest, now: &GearDigest, line: &dyn Fn(String)) {
    if was.short != now.short {
        line(format!("short_address {} -> {}", opt(was.short), opt(now.short)));
    }
    if was.random != now.random {
        line(format!("random_address {:#08x} -> {:#08x}", was.random, now.random));
    }
    if was.enabled != now.enabled {
        line(format!("enabled {} -> {}", was.enabled, now.enabled));
    }
    if was.groups != now.groups {
        line(format!("groups {:#06x} -> {:#06x}", was.groups, now.groups));
    }
}

fn report_output(was: &GearDigest, now: &GearDigest, line: &dyn Fn(String)) {
    if was.level != now.level {
        line(format!("level {} -> {}", was.level, now.level));
    }
    if was.identifying != now.identifying {
        line(format!("identify {} -> {}", was.identifying, now.identifying));
    }
}

fn report_colour(was: &GearDigest, now: &GearDigest, line: &dyn Fn(String)) {
    if was.colour_mode != now.colour_mode {
        line(format!("colour_mode {:?} -> {:?}", was.colour_mode, now.colour_mode));
    }
    if was.color_ct != now.color_ct {
        line(format!("colour_mirek {} -> {}", was.color_ct, now.color_ct));
    }
    if was.color_xy != now.color_xy {
        line(format!(
            "colour_xy {},{} -> {},{}",
            was.color_xy.0, was.color_xy.1, now.color_xy.0, now.color_xy.1
        ));
    }
    if was.rgbwaf != now.rgbwaf {
        line(format!("rgbwaf {} -> {}", channels(&was.rgbwaf), channels(&now.rgbwaf)));
    }
}

fn report_limits(was: &GearDigest, now: &GearDigest, line: &dyn Fn(String)) {
    if was.min_level != now.min_level || was.max_level != now.max_level {
        line(format!(
            "level_range {}..{} -> {}..{}",
            was.min_level, was.max_level, now.min_level, now.max_level
        ));
    }
    if was.tc_limits != now.tc_limits {
        line(format!(
            "tc_limits {}..{} -> {}..{}",
            was.tc_limits.0, was.tc_limits.1, now.tc_limits.0, now.tc_limits.1
        ));
    }
}

fn report_scenes(was: &GearDigest, now: &GearDigest, line: &dyn Fn(String)) {
    for scene in 0..SCENE_COUNT {
        let (old, new) = (was.scenes[scene], now.scenes[scene]);
        if old != new {
            line(format!("scene[{scene}] {} -> {}", slot(old), slot(new)));
        }
        let (old_tc, new_tc) = (was.scene_tc[scene], now.scene_tc[scene]);
        if old_tc != new_tc {
            line(format!(
                "scene[{scene}] mirek {} -> {}",
                wide_slot(old_tc),
                wide_slot(new_tc)
            ));
        }
    }
}

fn channels(levels: &[u8; 6]) -> String {
    levels
        .iter()
        .map(|level| level.to_string())
        .collect::<Vec<_>>()
        .join(",")
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
