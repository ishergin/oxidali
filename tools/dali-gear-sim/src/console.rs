use std::cell::Cell;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::devices::dt6_led::{
    FAILURE_OPEN_CIRCUIT, FAILURE_SHORT_CIRCUIT, FAILURE_THERMAL_OVERLOAD,
    FAILURE_THERMAL_SHUT_DOWN,
};
use dali2rust_domain::dali::devices::dt8_color::GEAR_FEATURES_AUTOMATIC_ACTIVATION as AUTOMATIC_ACTIVATION;
use dali2rust_gear_model::{bench_fleet, FleetStats, Gear, GearFleet};

use crate::answer::LoopStats;
use crate::boot_slot::BootSlot;
use crate::logsink::{self, Level};
use crate::note;
use crate::phy::GearPhy;

const SHORT_ADDRESSES: u32 = 64;
const READ_RETRY: Duration = Duration::from_millis(100);
const DROP_PERMILLE_MAX: u16 = 1000;

type Handler = fn(&Console, &[&str]);

const COMMANDS: &[(&str, Handler, &str)] = &[
    ("help", Console::help, "this list"),
    ("ready", Console::ready, "the ready line again: build, running slot, OTA state"),
    ("show", Console::show, "[addr]  the fleet table, or one gear"),
    ("stats", Console::stats, "loop, answer-cell, late-tick, fleet and wire counters"),
    ("reserve", Console::reserve, "<a-b,c ...>  addresses the fleet never holds; once per boot, before `fleet`"),
    ("fleet", Console::fleet, "<base> <dt6> <cct> <rgb>  build the fleet from base upward, all disabled"),
    ("enable", Console::enable, "<addr|all>  put gear on the bus"),
    ("disable", Console::disable, "<addr|all>  take gear off the bus"),
    ("unaddress", Console::unaddress, "<count>  take the short address off that many gear"),
    ("autoact", Console::auto_activation, "<addr> on|off  DT8 Automatic Activation bit"),
    ("metering", Console::metering, "<addr> energy|diagnostics|both|off  DiiA 252/253 banks"),
    ("luminaire", Console::luminaire, "<addr> off|3|4|5|bus-unit-on|bus-unit-off"),
    ("fail", Console::fail, "<addr> lamp|none|short|open|thermal|derate|<hex byte>"),
    ("drop", Console::drop_answers, "<addr> <permille 0..1000>  withhold that fraction of answers"),
    ("log", Console::log_level, "off|change|frame|trace"),
    ("reboot", Console::reboot, "restart; under the OTA role this hands the board back to the controller"),
];

pub struct Console {
    phy: &'static GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    stats: Arc<Mutex<LoopStats>>,
    reserved: Cell<Option<u64>>,
    seed: u32,
    build: &'static str,
    slot: BootSlot,
}

impl Console {
    pub fn new(
        phy: &'static GearPhy,
        fleet: Arc<Mutex<GearFleet>>,
        stats: Arc<Mutex<LoopStats>>,
        seed: u32,
        build: &'static str,
        slot: BootSlot,
    ) -> Self {
        Self {
            phy,
            fleet,
            stats,
            reserved: Cell::new(None),
            seed,
            build,
            slot,
        }
    }

    pub fn run(self) -> ! {
        let stdin = std::io::stdin();
        let mut raw = String::new();
        self.ready(&[]);
        loop {
            raw.clear();
            match stdin.lock().read_line(&mut raw) {
                Ok(read) if read > 0 => self.dispatch(&printable(&raw)),
                _ => {
                    // sleep-ok: the console driver returned nothing; retry instead of spinning on an empty port.
                    std::thread::sleep(READ_RETRY);
                }
            }
        }
    }

    fn dispatch(&self, line: &str) {
        let mut words = line.split_whitespace();
        let Some(verb) = words.next() else {
            return;
        };
        let args: Vec<&str> = words.collect();
        let verb = if verb == "?" { "help" } else { verb };
        match COMMANDS.iter().find(|(name, _, _)| *name == verb) {
            Some((_, handler, _)) => handler(self, &args),
            None => note!("unknown command '{verb}' — try 'help'"),
        }
    }

    fn ready(&self, _args: &[&str]) {
        note!("ready build={} slot={} state={}", self.build, self.slot.label, self.slot.state);
    }

    fn help(&self, _args: &[&str]) {
        note!("commands:");
        for (name, _, usage) in COMMANDS {
            note!("  {name} {usage}");
        }
    }

    fn with_fleet<R>(&self, f: impl FnOnce(&mut GearFleet) -> R) -> Option<R> {
        self.fleet.lock().ok().map(|mut fleet| f(&mut fleet))
    }

    fn with_gear_at(&self, addr: u8, mut f: impl FnMut(&mut Gear)) -> usize {
        self.with_fleet(|fleet| {
            let mut touched = 0;
            for gear in fleet.gears_mut() {
                if gear.spec.short_address == Some(addr) {
                    f(gear);
                    touched += 1;
                }
            }
            touched
        })
        .unwrap_or(0)
    }

    fn show(&self, args: &[&str]) {
        let filter = args.first().and_then(|w| w.parse::<u8>().ok());
        let rows = self
            .with_fleet(|fleet| {
                fleet
                    .gears()
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| filter.is_none() || g.spec.short_address == filter)
                    .map(|(index, g)| row(index, g))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        note!("idx addr random   type lvl groups on  drop");
        for line in &rows {
            note!("{line}");
        }
        note!("{} gear listed", rows.len());
    }

    fn stats(&self, _args: &[&str]) {
        let loop_stats = self.stats.lock().map(|s| *s).unwrap_or_default();
        let fleet = self.with_fleet(|fleet| {
            let on = fleet.gears().iter().filter(|g| g.enabled).count();
            (fleet.stats(), fleet.gears().len(), on)
        });
        let Some((fleet_stats, count, on)) = fleet else {
            return;
        };
        note!(
            "fleet {count} gear, {on} enabled, reserved {:#018x}",
            self.reserved.get().unwrap_or(0)
        );
        self.note_loop(&loop_stats);
        self.note_answer_cell();
        note_fleet(&fleet_stats);
        note_wire(&loop_stats, &fleet_stats);
    }

    fn note_loop(&self, s: &LoopStats) {
        note!(
            "heard={} forward16={} answered={} collisions={} cell_busy={}",
            s.frames_heard,
            s.forward16,
            s.answered,
            s.collisions,
            s.cell_busy
        );
        note!(
            "decode_failed={} other_width={} ring_dropped={} log_dropped={}",
            s.decode_failed,
            s.other_width,
            s.ring_dropped,
            logsink::dropped()
        );
    }

    fn note_answer_cell(&self) {
        let a = self.phy.answer_counts();
        note!(
            "sent={} stale={} expired={} late={} rejected={} collided={} last_arm_idle_ticks={}",
            a.sent,
            a.stale,
            a.expired,
            a.late,
            a.rejected,
            a.collided,
            self.phy.last_answer_arm_idle_ticks()
        );
        let (late_ticks, max_gap_us) = self.phy.late_tick_counts();
        note!("late_ticks={late_ticks} max_gap_us={max_gap_us}");
    }

    fn reserve(&self, args: &[&str]) {
        if let Some(mask) = self.reserved.get() {
            return note!("refused: reserved already {mask:#018x} for this boot; reboot to change it");
        }
        let Some(mask) = parse_reserve(&args.join(" ")) else {
            return note!("usage: reserve <a-b,c ...>   short addresses 0..=63 the fleet never holds");
        };
        self.reserved.set(Some(mask));
        note!("reserved {mask:#018x} ({} addresses)", mask.count_ones());
    }

    fn fleet(&self, args: &[&str]) {
        let Some(reserved) = self.reserved.get() else {
            return note!("refused: `reserve` first — the fleet must know every live address on this wire");
        };
        let (Some(base), Some(dt6), Some(cct), Some(rgb)) =
            (num(args, 0), num(args, 1), num(args, 2), num(args, 3))
        else {
            return note!("usage: fleet <base> <dt6> <cct> <rgb>");
        };
        let wanted = u32::from(dt6) + u32::from(cct) + u32::from(rgb);
        let free = SHORT_ADDRESSES - reserved.count_ones();
        if wanted > free {
            return note!("refused: {wanted} gear asked, {free} addresses are free of the reserve");
        }
        let specs = bench_fleet(base, dt6, cct, rgb, self.seed, reserved);
        let unaddressed = specs.iter().filter(|spec| spec.short_address.is_none()).count();
        if unaddressed > 0 {
            return note!("refused: {unaddressed} gear would have no address from base {base} up");
        }
        let mut built = GearFleet::new(specs, reserved, self.seed);
        for gear in built.gears_mut() {
            gear.enabled = false;
        }
        let span = address_span(&built);
        self.with_fleet(|fleet| *fleet = built);
        note!("fleet {wanted} gear at {span}, all disabled");
    }

    fn enable(&self, args: &[&str]) {
        self.set_enabled(args.first().copied(), true);
    }

    fn disable(&self, args: &[&str]) {
        self.set_enabled(args.first().copied(), false);
    }

    fn set_enabled(&self, which: Option<&str>, on: bool) {
        let verb = if on { "enable" } else { "disable" };
        if on && self.reserved.get().is_none() {
            return note!("refused: `reserve` first");
        }
        let touched = match which {
            Some("all") => self.with_fleet(|fleet| {
                fleet.gears_mut().iter_mut().for_each(|g| g.enabled = on);
                fleet.gears().len()
            }),
            Some(word) => word.parse::<u8>().ok().map(|addr| self.with_gear_at(addr, |g| g.enabled = on)),
            None => None,
        };
        match touched {
            Some(0) if on => note!("refused: no gear to enable at {}", which.unwrap_or_default()),
            Some(count) => note!("{verb}d {count} gear"),
            None => note!("usage: {verb} <addr|all>"),
        }
    }

    fn unaddress(&self, args: &[&str]) {
        let Some(count) = num(args, 0) else {
            return note!("usage: unaddress <count>");
        };
        let done = self
            .with_fleet(|fleet| {
                let mut done = 0u8;
                for gear in fleet.gears_mut().iter_mut().filter(|g| g.spec.short_address.is_some()) {
                    if done >= count {
                        break;
                    }
                    gear.spec.short_address = None;
                    done += 1;
                }
                done
            })
            .unwrap_or(0);
        note!("unaddressed {done} gear; they now answer only the search");
    }

    fn auto_activation(&self, args: &[&str]) {
        let (Some(addr), Some(on)) = (num(args, 0), args.get(1).and_then(|m| on_off(m))) else {
            return note!("usage: autoact <addr> on|off");
        };
        let touched = self.with_gear_at(addr, |gear| {
            gear.gear_features = if on {
                gear.gear_features | AUTOMATIC_ACTIVATION
            } else {
                gear.gear_features & !AUTOMATIC_ACTIVATION
            };
        });
        note!("automatic activation {} on {touched} gear at addr {addr}", if on { "on" } else { "off" });
    }

    fn metering(&self, args: &[&str]) {
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("usage: metering <addr> energy|diagnostics|both|off");
        };
        let Some((energy, diagnostics)) = metering_mode(mode) else {
            return note!("usage: metering <addr> energy|diagnostics|both|off");
        };
        let touched = self.with_gear_at(addr, |gear| gear.set_metering(energy, diagnostics));
        note!("metering {mode} on {touched} gear at short {addr}");
    }

    fn luminaire(&self, args: &[&str]) {
        const USAGE: &str = "usage: luminaire <addr> off|3|4|5|bus-unit-on|bus-unit-off";
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("{USAGE}");
        };
        if !LUMINAIRE_MODES.contains(&mode) {
            return note!("{USAGE}");
        }
        let touched = self.with_gear_at(addr, |gear| apply_luminaire(gear, mode));
        note!("luminaire {mode} on {touched} gear at short {addr}");
    }

    fn fail(&self, args: &[&str]) {
        const USAGE: &str = "usage: fail <addr> lamp|none|short|open|thermal|derate|<hex byte>";
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("{USAGE}");
        };
        let Some((lamp, byte)) = failure_mode(mode) else {
            return note!("{USAGE}");
        };
        self.with_gear_at(addr, |gear| {
            gear.lamp_failure = lamp;
            if let Some(raw) = byte {
                gear.faults.failure_status = raw;
            }
        });
        match byte {
            Some(raw) => note!("A{addr:02} failure_status=0x{raw:02X} lamp_failure={lamp}"),
            None => note!("A{addr:02} lamp_failure={lamp}"),
        }
    }

    fn drop_answers(&self, args: &[&str]) {
        let (Some(addr), Some(permille)) = (num(args, 0), num16(args, 1)) else {
            return note!("usage: drop <addr> <permille 0..1000>");
        };
        let permille = permille.min(DROP_PERMILLE_MAX);
        self.with_gear_at(addr, |gear| gear.faults.drop_answer_permille = permille);
        note!("A{addr:02} drop_answer_permille={permille}");
    }

    fn log_level(&self, args: &[&str]) {
        match args.first().and_then(|w| Level::parse(w)) {
            Some(level) => {
                logsink::set_level(level);
                note!("log level {}", level.name());
            }
            None => note!("usage: log off|change|frame|trace (now: {})", logsink::level().name()),
        }
    }

    fn reboot(&self, _args: &[&str]) {
        note!("rebooting");
        // SAFETY: IDF restart; never returns.
        unsafe { esp_idf_svc::sys::esp_restart() }
    }
}

fn note_fleet(f: &FleetStats) {
    note!(
        "reserved_conflicts={} refused_programs={} dropped_answers={}",
        f.reserved_conflicts,
        f.refused_programs,
        f.dropped_answers
    );
}

fn note_wire(s: &LoopStats, f: &FleetStats) {
    note!(
        "forward-frame settling by IEC 62386-101 Table 22 band, below_p1_floor={}",
        s.below_p1_floor
    );
    for (band, count) in s.settle_bands.iter().enumerate() {
        if *count > 0 {
            note!("  priority {} {}", band + 1, count);
        }
    }
    if f.enable_device_type_consumed_by_interloper > 0 {
        note!("violations enable_consumed={}", f.enable_device_type_consumed_by_interloper);
    }
    note!(
        "send_twice_pairs={} send_twice_interloper={}",
        f.send_twice_pairs_executed,
        f.send_twice_split_by_interloper
    );
    if f.dt8_gear_features_reserved_bits > 0 {
        note!("violations dt8_gear_features_reserved={}", f.dt8_gear_features_reserved_bits);
    }
}

fn row(index: usize, g: &Gear) -> String {
    format!(
        "{:3} {:>4} {:06x} {:>4} {:3} {:#06x} {:3} {}",
        index,
        g.spec.short_address.map_or("-".to_string(), |a| a.to_string()),
        g.spec.random_address,
        if g.is_dt8() { "DT8" } else { "DT6" },
        g.level,
        g.groups,
        if g.enabled { "yes" } else { "no" },
        g.faults.drop_answer_permille
    )
}

fn address_span(fleet: &GearFleet) -> String {
    let shorts: Vec<u8> = fleet.gears().iter().filter_map(|g| g.spec.short_address).collect();
    match (shorts.iter().min(), shorts.iter().max()) {
        (Some(lo), Some(hi)) => format!("{lo}..={hi}"),
        _ => "no short address".to_string(),
    }
}

fn parse_reserve(text: &str) -> Option<u64> {
    let mut mask = 0u64;
    for part in text.split([',', ' ']).filter(|p| !p.is_empty()) {
        let (lo, hi) = match part.split_once('-') {
            Some((lo, hi)) => (lo.parse::<u8>().ok()?, hi.parse::<u8>().ok()?),
            None => {
                let one = part.parse::<u8>().ok()?;
                (one, one)
            }
        };
        if lo > hi || u32::from(hi) >= SHORT_ADDRESSES {
            return None;
        }
        for addr in lo..=hi {
            mask |= 1u64 << addr;
        }
    }
    (mask != 0).then_some(mask)
}

const LUMINAIRE_MODES: &[&str] = &["off", "3", "4", "5", "bus-unit-on", "bus-unit-off"];

fn apply_luminaire(gear: &mut Gear, mode: &str) {
    match mode {
        "off" => gear.set_luminaire_format(None),
        "3" => gear.set_luminaire_format(Some(LuminaireFormat::V3)),
        "4" => gear.set_luminaire_format(Some(LuminaireFormat::V4)),
        "5" => gear.set_luminaire_format(Some(LuminaireFormat::V5)),
        "bus-unit-on" => gear.set_bus_unit_extension(true),
        _ => gear.set_bus_unit_extension(false),
    }
}

fn metering_mode(mode: &str) -> Option<(bool, bool)> {
    match mode {
        "energy" => Some((true, false)),
        "diagnostics" | "both" => Some((true, true)),
        "off" => Some((false, false)),
        _ => None,
    }
}

fn failure_mode(mode: &str) -> Option<(bool, Option<u8>)> {
    match mode {
        "lamp" => Some((true, None)),
        "none" => Some((false, Some(0x00))),
        "short" => Some((false, Some(FAILURE_SHORT_CIRCUIT))),
        "open" => Some((false, Some(FAILURE_OPEN_CIRCUIT))),
        "thermal" => Some((false, Some(FAILURE_THERMAL_SHUT_DOWN))),
        "derate" => Some((false, Some(FAILURE_THERMAL_OVERLOAD))),
        other => parse_byte(other).map(|raw| (false, Some(raw))),
    }
}

fn on_off(word: &str) -> Option<bool> {
    match word {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

fn num(args: &[&str], at: usize) -> Option<u8> {
    args.get(at)?.parse().ok()
}

fn num16(args: &[&str], at: usize) -> Option<u16> {
    args.get(at)?.parse().ok()
}

fn parse_byte(text: &str) -> Option<u8> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => text.parse::<u8>().ok(),
    }
}

fn printable(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .collect()
}
