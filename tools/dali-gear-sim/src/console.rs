use std::sync::{Arc, Mutex};

use dali2rust_gear_model::{bench_fleet, FleetStats, GearFleet};
use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::devices::dt6_led::{
    FAILURE_OPEN_CIRCUIT, FAILURE_SHORT_CIRCUIT, FAILURE_THERMAL_OVERLOAD,
    FAILURE_THERMAL_SHUT_DOWN,
};
use dali2rust_domain::dali::devices::dt8_color::GEAR_FEATURES_AUTOMATIC_ACTIVATION as AUTOMATIC_ACTIVATION;

use crate::answer::{LoopStats, MAX_SUBMIT_IDLE_TICKS, MIN_SUBMIT_IDLE_TICKS};
use crate::logsink::{self, Level};
use crate::persist::Store;
use crate::{note, FleetLayout};

pub struct Console {
    pub fleet: Arc<Mutex<GearFleet>>,
    pub stats: Arc<Mutex<LoopStats>>,
    pub store: Arc<Mutex<Option<Store>>>,
    pub layout: Arc<Mutex<FleetLayout>>,
}

struct Row {
    index: usize,
    short: Option<u8>,
    random: u32,
    level: u8,
    groups: u16,
    enabled: bool,
    dt8: bool,
    faults: u16,
}

impl Console {
    pub fn run(&self) -> ! {
        help();
        let mut line = String::new();
        loop {
            match read_line(&mut line) {
                Some(command) => {
                    self.dispatch(&command);
                    line.clear();
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }

    fn dispatch(&self, line: &str) {
        let mut words = line.split_whitespace();
        let Some(verb) = words.next() else {
            return;
        };
        let args: Vec<&str> = words.collect();
        match verb {
            "help" | "?" => help(),
            "show" => self.show(args.first().copied()),
            "stats" => self.stats(),
            "base" => self.set_base(&args),
            "unaddress" => self.unaddress(&args),
            "fleet" => self.rebuild(&args),
            "enable" => self.set_enabled(args.first().copied(), true),
            "disable" => self.set_enabled(args.first().copied(), false),
            "autoact" => self.auto_activation(&args),
            "metering" => self.metering(&args),
            "luminaire" => self.luminaire(&args),
            "fail" => self.fail(&args),
            "drop" => self.drop_answers(&args),
            "log" => self.log_level(args.first().copied()),
            "save" => self.save(),
            "load" => self.load(),
            "erase" => self.erase(),
            "reboot" => reboot(),
            other => note!("unknown command '{other}' — try 'help'"),
        }
    }

    fn show(&self, which: Option<&str>) {
        let filter = which.and_then(|w| w.parse::<u8>().ok());
        let rows = {
            let Ok(fleet) = self.fleet.lock() else {
                return;
            };
            fleet
                .gears()
                .iter()
                .enumerate()
                .filter(|(_, g)| filter.is_none() || g.spec.short_address == filter)
                .map(|(index, g)| Row {
                    index,
                    short: g.spec.short_address,
                    random: g.spec.random_address,
                    level: g.level,
                    groups: g.groups,
                    enabled: g.enabled,
                    dt8: g.is_dt8(),
                    faults: g.faults.drop_answer_permille,
                })
                .collect::<Vec<_>>()
        };
        note!("idx addr random   type lvl groups on  drop");
        for row in &rows {
            note!(
                "{:3} {:>4} {:06x} {:>4} {:3} {:#06x} {:3} {}",
                row.index,
                row.short.map_or("-".to_string(), |a| a.to_string()),
                row.random,
                if row.dt8 { "DT8" } else { "DT6" },
                row.level,
                row.groups,
                if row.enabled { "yes" } else { "no" },
                row.faults
            );
        }
        note!("{} gear listed", rows.len());
    }

    fn stats(&self) {
        let loop_stats = {
            let Ok(stats) = self.stats.lock() else { return };
            *stats
        };
        let (fleet_stats, count, on) = {
            let Ok(fleet) = self.fleet.lock() else { return };
            (
                fleet.stats(),
                fleet.gears().len(),
                fleet.gears().iter().filter(|g| g.enabled).count(),
            )
        };
        note!("fleet {count} gear, {on} enabled");
        note!(
            "heard={} forward16={} answered={} collisions={}",
            loop_stats.frames_heard,
            loop_stats.forward16,
            loop_stats.answered,
            loop_stats.collisions
        );
        note!(
            "aborted={} tx_rejected={} tx_collided={} decode_failed={} other_width={} ring_dropped={}",
            loop_stats.aborted,
            loop_stats.rejected,
            loop_stats.tx_collided,
            loop_stats.decode_failed,
            loop_stats.other_width,
            loop_stats.ring_dropped
        );
        note!(
            "reserved_conflicts={} refused_programs={} dropped_answers={}",
            fleet_stats.reserved_conflicts,
            fleet_stats.refused_programs,
            fleet_stats.dropped_answers
        );
        note!(
            "submit idle_ticks histogram (window {MIN_SUBMIT_IDLE_TICKS}..={MAX_SUBMIT_IDLE_TICKS}), \
             out_of_window={}",
            loop_stats.submit_out_of_window
        );
        for (bucket, count) in loop_stats.submit_ticks.iter().enumerate() {
            if *count > 0 {
                note!("  ticks {:3}..{:3} {}", bucket * 8, bucket * 8 + 7, count);
            }
        }
        Self::print_wire_observations(&loop_stats, &fleet_stats);
    }

    fn print_wire_observations(loop_stats: &LoopStats, fleet_stats: &FleetStats) {
        note!(
            "forward-frame settling by IEC 62386-101 Table 22 band, below_p1_floor={}",
            loop_stats.below_p1_floor
        );
        for (band, count) in loop_stats.settle_bands.iter().enumerate() {
            if *count > 0 {
                note!("  priority {} {}", band + 1, count);
            }
        }
        let violations = fleet_stats.enable_device_type_consumed_by_interloper;
        if violations > 0 {
            note!("violations enable_consumed={violations}");
        }
        note!(
            "send_twice_pairs={} send_twice_interloper={}",
            fleet_stats.send_twice_pairs_executed,
            fleet_stats.send_twice_split_by_interloper
        );
        let reserved = fleet_stats.dt8_gear_features_reserved_bits;
        if reserved > 0 {
            note!("violations dt8_gear_features_reserved={reserved}");
        }
    }

    fn set_base(&self, args: &[&str]) {
        let Some(base) = num(args, 0) else {
            note!("usage: base <addr>   (next `fleet` lays out from here; reserved addresses are skipped, not conflicted)");
            return;
        };
        let Ok(mut layout) = self.layout.lock() else {
            return;
        };
        layout.base = base;
        note!("base = {base}; run `fleet <dt6> <cct> <rgb>` to lay the fleet out again");
    }

    fn unaddress(&self, args: &[&str]) {
        let Some(count) = num(args, 0) else {
            note!("usage: unaddress <count>   (take the address off that many gear; `fleet` puts them back)");
            return;
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        let mut done = 0u8;
        for gear in fleet.gears_mut() {
            if done >= count {
                break;
            }
            if gear.spec.short_address.is_some() {
                gear.spec.short_address = None;
                done += 1;
            }
        }
        note!("unaddressed {done} gear; they now answer only the search");
    }

    fn rebuild(&self, args: &[&str]) {
        let (Some(dt6), Some(cct), Some(rgb)) = (num(args, 0), num(args, 1), num(args, 2)) else {
            note!("usage: fleet <dt6> <cct> <rgb>   (laid out from the base address up)");
            return;
        };
        let Ok(mut layout) = self.layout.lock() else {
            return;
        };
        layout.dt6 = dt6;
        layout.cct = cct;
        layout.rgb = rgb;
        let specs = bench_fleet(layout.base, dt6, cct, rgb, layout.seed);
        let reserved = {
            let Ok(fleet) = self.fleet.lock() else { return };
            fleet.reserved()
        };
        let mut built = GearFleet::new(specs, reserved, layout.seed);
        for gear in built.gears_mut() {
            gear.enabled = false;
        }
        let conflicts = built.stats().reserved_conflicts;
        let count = built.gears().len();
        if let Ok(mut fleet) = self.fleet.lock() {
            *fleet = built;
        }
        note!("fleet rebuilt: {count} gear from address {}, all disabled", layout.base);
        if conflicts > 0 {
            note!("WARNING {conflicts} gear wanted a reserved address and are unaddressed");
        }
    }

    fn set_enabled(&self, which: Option<&str>, on: bool) {
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        let mut touched = 0usize;
        match which {
            Some("all") => {
                for gear in fleet.gears_mut() {
                    gear.enabled = on;
                    touched += 1;
                }
            }
            Some(word) => match word.parse::<u8>() {
                Ok(addr) => {
                    for gear in fleet.gears_mut() {
                        if gear.spec.short_address == Some(addr) {
                            gear.enabled = on;
                            touched += 1;
                        }
                    }
                }
                Err(_) => return note!("usage: {} <addr|all>", if on { "enable" } else { "disable" }),
            },
            None => return note!("usage: {} <addr|all>", if on { "enable" } else { "disable" }),
        }
        note!("{} {touched} gear", if on { "enabled" } else { "disabled" });
    }

    fn metering(&self, args: &[&str]) {
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("usage: metering <addr> energy|diagnostics|both|off");
        };
        let (energy, diagnostics) = match mode {
            "energy" => (true, false),
            "diagnostics" => (true, true),
            "both" => (true, true),
            "off" => (false, false),
            _ => return note!("usage: metering <addr> energy|diagnostics|both|off"),
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        let mut touched = 0;
        for gear in fleet.gears_mut() {
            if gear.spec.short_address == Some(addr) {
                gear.set_metering(energy, diagnostics);
                touched += 1;
            }
        }
        note!("metering {mode} on {touched} gear at short {addr}");
    }

    fn luminaire(&self, args: &[&str]) {
        const USAGE: &str = "usage: luminaire <addr> off|3|4|5|bus-unit-on|bus-unit-off";
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("{USAGE}");
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        let mut touched = 0;
        for gear in fleet.gears_mut() {
            if gear.spec.short_address != Some(addr) {
                continue;
            }
            match mode {
                "off" => gear.set_luminaire_format(None),
                "3" => gear.set_luminaire_format(Some(LuminaireFormat::V3)),
                "4" => gear.set_luminaire_format(Some(LuminaireFormat::V4)),
                "5" => gear.set_luminaire_format(Some(LuminaireFormat::V5)),
                "bus-unit-on" => gear.set_bus_unit_extension(true),
                "bus-unit-off" => gear.set_bus_unit_extension(false),
                _ => return note!("{USAGE}"),
            }
            touched += 1;
        }
        note!("luminaire {mode} on {touched} gear at short {addr}");
    }

    fn auto_activation(&self, args: &[&str]) {
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("usage: autoact <addr> on|off");
        };
        let on = match mode {
            "on" => true,
            "off" => false,
            _ => return note!("usage: autoact <addr> on|off"),
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        let mut touched = 0usize;
        for gear in fleet.gears_mut() {
            if gear.spec.short_address == Some(addr) {
                gear.gear_features = if on {
                    gear.gear_features | AUTOMATIC_ACTIVATION
                } else {
                    gear.gear_features & !AUTOMATIC_ACTIVATION
                };
                touched += 1;
            }
        }
        note!(
            "automatic activation {} on {touched} gear at addr {addr}",
            if on { "on" } else { "off" }
        );
    }

    fn fail(&self, args: &[&str]) {
        let (Some(addr), Some(mode)) = (num(args, 0), args.get(1).copied()) else {
            return note!("{FAIL_USAGE}");
        };
        let (lamp, byte) = match mode {
            "lamp" => (true, None),
            "none" => (false, Some(0x00)),
            "short" => (false, Some(FAILURE_SHORT_CIRCUIT)),
            "open" => (false, Some(FAILURE_OPEN_CIRCUIT)),
            "thermal" => (false, Some(FAILURE_THERMAL_SHUT_DOWN)),
            "derate" => (false, Some(FAILURE_THERMAL_OVERLOAD)),
            other => match parse_byte(other) {
                Some(raw) => (false, Some(raw)),
                None => return note!("{FAIL_USAGE}"),
            },
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        for gear in fleet.gears_mut() {
            if gear.spec.short_address == Some(addr) {
                gear.lamp_failure = lamp;
                if let Some(raw) = byte {
                    gear.faults.failure_status = raw;
                }
            }
        }
        match byte {
            Some(raw) => note!("A{addr:02} failure_status=0x{raw:02X} lamp_failure={lamp}"),
            None => note!("A{addr:02} lamp_failure={lamp}"),
        }
    }

    fn drop_answers(&self, args: &[&str]) {
        let (Some(addr), Some(permille)) = (num(args, 0), num16(args, 1)) else {
            return note!("usage: drop <addr> <permille 0..1000>");
        };
        let Ok(mut fleet) = self.fleet.lock() else {
            return;
        };
        for gear in fleet.gears_mut() {
            if gear.spec.short_address == Some(addr) {
                gear.faults.drop_answer_permille = permille.min(1000);
            }
        }
        note!("A{addr:02} drop_answer_permille={}", permille.min(1000));
    }

    fn log_level(&self, which: Option<&str>) {
        match which.and_then(Level::parse) {
            Some(level) => {
                logsink::set_level(level);
                note!("log level {}", level.name());
            }
            None => note!("usage: log off|change|frame|trace (now: {})", logsink::level().name()),
        }
    }

    fn save(&self) {
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        let Some(store) = store.as_mut() else {
            return note!("no NVS store — nothing is persisted this session");
        };
        let Ok(fleet) = self.fleet.lock() else { return };
        match store.save(&fleet) {
            Ok(bytes) => note!("saved {bytes} bytes"),
            Err(e) => note!("save failed: {e}"),
        }
    }

    fn load(&self) {
        let Ok(store) = self.store.lock() else { return };
        let Some(store) = store.as_ref() else {
            return note!("no NVS store");
        };
        let reserved = {
            let Ok(fleet) = self.fleet.lock() else { return };
            fleet.reserved()
        };
        match store.load(reserved) {
            Ok(loaded) => {
                let count = loaded.gears().len();
                if let Ok(mut fleet) = self.fleet.lock() {
                    *fleet = loaded;
                }
                note!("loaded {count} gear");
            }
            Err(e) => note!("load failed: {e}"),
        }
    }

    fn erase(&self) {
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        let Some(store) = store.as_mut() else {
            return note!("no NVS store");
        };
        match store.erase() {
            Ok(()) => note!("stored fleet erased — next boot builds the default, disabled"),
            Err(e) => note!("erase failed: {e}"),
        }
    }
}

fn num(args: &[&str], at: usize) -> Option<u8> {
    args.get(at)?.parse().ok()
}

fn num16(args: &[&str], at: usize) -> Option<u16> {
    args.get(at)?.parse().ok()
}

fn reboot() -> ! {
    note!("rebooting");
    // SAFETY: IDF restart; never returns.
    unsafe { esp_idf_svc::sys::esp_restart() }
}

fn help() {
    note!("commands:");
    note!("  show [addr]           fleet table, or one gear");
    note!("  stats                 counters and the submit-timing histogram");
    note!("  base <addr>               where the next `fleet` starts laying out");
    note!("  unaddress <count>         take the address off that many gear (C1 denominator)");
    note!("  fleet <dt6> <cct> <rgb>   rebuild from the base address up (disabled)");
    note!("  enable|disable <addr|all>   put gear on or off the bus");
    note!("  autoact <addr> on|off       DT8 Automatic Activation bit (RAM, not saved)");
    note!("  metering <addr> energy|diagnostics|both|off");
    note!("  luminaire <addr> off|3|4|5|bus-unit-on|bus-unit-off");
    note!("                              DiiA 252/253 banks + the types that announce them");
    note!("  fail <addr> lamp|none|short|open|thermal|derate|<hex>");
    note!("                              lamp = 102 bit 1 alone; the rest set the");
    note!("                              207 failure byte and let the model derive");
    note!("                              what the standard makes follow from it");
    note!("  drop <addr> <permille>      withhold that fraction of answers");
    note!("  log off|change|frame|trace  log verbosity (default change)");
    note!("  save | load | erase   the stored fleet in NVS");
    note!("  reboot");
}

const FAIL_USAGE: &str = "usage: fail <addr> lamp|none|short|open|thermal|derate|<hex byte>";

fn parse_byte(text: &str) -> Option<u8> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => text.parse::<u8>().ok(),
    }
}

fn read_line(line: &mut String) -> Option<String> {
    use std::io::Read;
    let mut byte = [0u8; 1];
    loop {
        match std::io::stdin().read(&mut byte) {
            Ok(1) => match byte[0] {
                b'\n' | b'\r' => {
                    if line.trim().is_empty() {
                        line.clear();
                        return None;
                    }
                    return Some(line.trim().to_string());
                }
                c if c.is_ascii_graphic() || c == b' ' => line.push(c as char),
                _ => {}
            },
            _ => return None,
        }
    }
}
