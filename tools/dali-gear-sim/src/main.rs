mod answer;
mod console;
mod logsink;
mod persist;
mod phy;

use std::sync::{Arc, Mutex};

use dali2rust_gear_model::{bench_fleet, GearFleet, DEFAULT_RESERVED_SHORT_ADDRESSES};
use esp_idf_svc::hal::peripherals::Peripherals;

use answer::{AnswerLoop, LoopStats};
use console::Console;
use persist::{fingerprint, Store};

const RESERVED: u64 = DEFAULT_RESERVED_SHORT_ADDRESSES;

const DEFAULT_BASE: u8 = 10;
const DEFAULT_DT6: u8 = 27;
const DEFAULT_CCT: u8 = 18;
const DEFAULT_RGB: u8 = 9;
const DEFAULT_SEED: u32 = 0x0DA1_1000;

const GEAR_TASK_STACK: usize = 8192;

const HOUSEKEEPING_PERIOD_MS: u64 = 1_000;
const QUIET_CHECKS_BEFORE_SAVE: u32 = 2;
const IDLE_TICKS_SATURATED: u32 = 255;

pub struct FleetLayout {
    pub base: u8,
    pub dt6: u8,
    pub cct: u8,
    pub rgb: u8,
    pub seed: u32,
}

fn main() -> ! {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    banner();

    let peripherals = Peripherals::take().expect("peripherals");

    let store = match Store::open() {
        Ok(store) => Some(store),
        Err(e) => {
            note!("WARNING no NVS store ({e}) — nothing will be persisted");
            None
        }
    };

    let (fleet, was_restored) = load_or_default(store.as_ref());
    let layout = FleetLayout {
        base: DEFAULT_BASE,
        dt6: DEFAULT_DT6,
        cct: DEFAULT_CCT,
        rgb: DEFAULT_RGB,
        seed: DEFAULT_SEED,
    };

    let phy = phy::GearPhy::start(peripherals).expect("DALI PHY");
    report_bus_probe(phy);
    std::thread::sleep(std::time::Duration::from_millis(50));
    match phy::isr_core() {
        Some(core) => note!(
            "phy up: tx=gpio{} rx=gpio{}, isr on core {core}",
            phy::DALI_TX_GPIO,
            phy::DALI_RX_GPIO
        ),
        None => note!("WARNING phy timer has not ticked — the emulator is deaf and mute"),
    }

    let fleet = Arc::new(Mutex::new(fleet));
    let stats = Arc::new(Mutex::new(LoopStats::default()));
    let store = Arc::new(Mutex::new(store));
    let layout = Arc::new(Mutex::new(layout));

    spawn_gear_task(phy, Arc::clone(&fleet), Arc::clone(&stats));
    spawn_housekeeping(phy, Arc::clone(&fleet), Arc::clone(&store), was_restored);

    note!("ready");
    Console {
        fleet,
        stats,
        store,
        layout,
    }
    .run()
}

fn banner() {
    note!(
        "dali-gear-sim build={} {} (esp32c6)",
        env!("CARGO_PKG_VERSION"),
        compile_stamp()
    );
    note!(
        "reserved short addresses {:#018x} — the bench's real luminaires",
        RESERVED
    );
}

fn compile_stamp() -> &'static str {
    option_env!("DALI_GEAR_SIM_STAMP").unwrap_or("unstamped")
}

fn report_bus_probe(phy: &phy::GearPhy) {
    let (edges, level) = phy.probe();
    note!(
        "pinscan rx=gpio{} level={} edges={}",
        phy::DALI_RX_GPIO,
        u8::from(level),
        edges
    );
    if edges == 0 {
        note!("NOTE no bus activity seen — quiet segment, or the RX pin is wrong");
    }
}

fn load_or_default(store: Option<&Store>) -> (GearFleet, bool) {
    if let Some(store) = store {
        match store.load(RESERVED) {
            Ok(fleet) => {
                let on = fleet.gears().iter().filter(|g| g.enabled).count();
                note!("fleet restored: {} gear, {on} enabled", fleet.gears().len());
                return (fleet, true);
            }
            Err(e) => note!("no stored fleet ({e}) — building the default"),
        }
    }
    let specs = bench_fleet(
        DEFAULT_BASE,
        DEFAULT_DT6,
        DEFAULT_CCT,
        DEFAULT_RGB,
        DEFAULT_SEED,
    );
    let mut fleet = GearFleet::new(specs, RESERVED, DEFAULT_SEED);
    for gear in fleet.gears_mut() {
        gear.enabled = false;
    }
    note!(
        "default fleet: {} gear at {}..={}, ALL DISABLED — 'enable <addr>' to put one on the bus",
        fleet.gears().len(),
        DEFAULT_BASE,
        DEFAULT_BASE as usize + fleet.gears().len() - 1
    );
    (fleet, false)
}

fn spawn_gear_task(
    phy: &'static phy::GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    stats: Arc<Mutex<LoopStats>>,
) {
    std::thread::Builder::new()
        .name("dali-gear".into())
        .stack_size(GEAR_TASK_STACK)
        .spawn(move || AnswerLoop::new(phy, fleet, stats).run())
        .expect("gear task");
}

fn spawn_housekeeping(
    phy: &'static phy::GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    store: Arc<Mutex<Option<Store>>>,
    already_persisted: bool,
) {
    std::thread::Builder::new()
        .name("gear-housekeep".into())
        .stack_size(4096)
        .spawn(move || housekeeping_loop(phy, fleet, store, already_persisted))
        .expect("housekeeping task");
}

fn housekeeping_loop(
    phy: &'static phy::GearPhy,
    fleet: Arc<Mutex<GearFleet>>,
    store: Arc<Mutex<Option<Store>>>,
    already_persisted: bool,
) -> ! {
    let mut saved = if already_persisted {
        fleet.lock().ok().map(|f| fingerprint(&f))
    } else {
        None
    };
    let mut quiet = 0u32;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(HOUSEKEEPING_PERIOD_MS));
        quiet = if phy.idle_ticks() >= IDLE_TICKS_SATURATED {
            quiet.saturating_add(1)
        } else {
            0
        };
        let Ok(fleet) = fleet.lock() else { continue };
        let now = fingerprint(&fleet);
        if saved == Some(now) || quiet < QUIET_CHECKS_BEFORE_SAVE {
            continue;
        }
        let Ok(mut store) = store.lock() else { continue };
        let Some(store) = store.as_mut() else { continue };
        match store.save(&fleet) {
            Ok(bytes) => {
                saved = Some(now);
                note!("fleet saved ({bytes} bytes)");
            }
            Err(e) => logerr!("save_failed {e}"),
        }
    }
}
